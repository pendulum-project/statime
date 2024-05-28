use core::{
    marker::PhantomData,
    sync::atomic::{AtomicI8, Ordering},
};

use atomic_refcell::AtomicRefCell;
use rand::Rng;

#[allow(unused_imports)]
use crate::float_polyfill::FloatPolyfill;
use crate::{
    bmc::{acceptable_master::AcceptableMasterList, bmca::Bmca},
    clock::Clock,
    config::{InstanceConfig, PortConfig},
    datastructures::{
        common::PortIdentity,
        datasets::{InternalCurrentDS, InternalDefaultDS, InternalParentDS, TimePropertiesDS},
    },
    filters::Filter,
    observability::{current::CurrentDS, default::DefaultDS, parent::ParentDS},
    port::{InBmca, Port},
    time::Duration,
};

/// A PTP node.
///
/// This object handles the complete running of the PTP protocol once created.
/// It provides all the logic for both ordinary and boundary clock mode.
///
/// # Example
///
/// ```no_run
/// # struct MockClock;
/// # impl statime::Clock for MockClock {
/// #     type Error = ();
/// #     fn now(&self) -> statime::time::Time {
/// #         unimplemented!()
/// #     }
/// #     fn step_clock(&mut self, _: statime::time::Duration) -> Result<statime::time::Time, Self::Error> {
/// #         unimplemented!()
/// #     }
/// #     fn set_frequency(&mut self, _: f64) -> Result<statime::time::Time, Self::Error> {
/// #         unimplemented!()
/// #     }
/// #     fn set_properties(&mut self, _: &TimePropertiesDS) -> Result<(), Self::Error> {
/// #         unimplemented!()
/// #     }
/// # }
/// # mod system {
/// #     pub fn get_mac() -> [u8; 6] { unimplemented!() }
/// #     pub fn sleep(time: core::time::Duration) { unimplemented!() }
/// # }
/// # let port_config: statime::config::PortConfig<AcceptAnyMaster> = unimplemented!();
/// # let filter_config = unimplemented!();
/// # let clock: MockClock = unimplemented!();
/// # let rng: rand::rngs::mock::StepRng = unimplemented!();
/// #
/// use statime::PtpInstance;
/// use statime::config::{AcceptAnyMaster, ClockIdentity, InstanceConfig, TimePropertiesDS, TimeSource};
/// use statime::filters::BasicFilter;
///
/// let instance_config = InstanceConfig {
///     clock_identity: ClockIdentity::from_mac_address(system::get_mac()),
///     priority_1: 128,
///     priority_2: 128,
///     domain_number: 0,
///     slave_only: false,
///     sdo_id: Default::default(),
/// };
/// let time_properties_ds = TimePropertiesDS::new_arbitrary_time(false, false, TimeSource::InternalOscillator);
///
/// let mut instance = PtpInstance::<BasicFilter>::new(
///     instance_config,
///     time_properties_ds,
/// );
///
/// let mut port = instance.add_port(port_config, filter_config, clock, rng);
///
/// // Send of port to its own thread/task to do its work
///
/// loop {
///     instance.bmca(&mut [&mut port]);
///     system::sleep(instance.bmca_interval());
/// }
/// ```
pub struct PtpInstance<F> {
    state: AtomicRefCell<PtpInstanceState>,
    log_bmca_interval: AtomicI8,
    _filter: PhantomData<F>,
}

#[derive(Debug)]
pub(crate) struct PtpInstanceState {
    pub(crate) default_ds: InternalDefaultDS,
    pub(crate) current_ds: InternalCurrentDS,
    pub(crate) parent_ds: InternalParentDS,
    pub(crate) time_properties_ds: TimePropertiesDS,
}

impl PtpInstanceState {
    fn bmca<A: AcceptableMasterList, C: Clock, F: Filter, R: Rng>(
        &mut self,
        ports: &mut [&mut Port<'_, InBmca, A, R, C, F>],
        bmca_interval: Duration,
    ) {
        debug_assert_eq!(self.default_ds.number_ports as usize, ports.len());

        for port in ports.iter_mut() {
            port.calculate_best_local_announce_message()
        }

        let ebest = Bmca::<()>::find_best_announce_message(
            ports
                .iter()
                .filter_map(|port| port.best_local_announce_message_for_bmca()),
        );

        for port in ports.iter_mut() {
            let recommended_state = Bmca::<()>::calculate_recommended_state(
                &self.default_ds,
                ebest,
                port.best_local_announce_message_for_state(), // erbest
                port.state(),
            );

            log::debug!(
                "Recommended state port {}: {recommended_state:?}",
                port.number(),
            );

            if let Some(recommended_state) = recommended_state {
                port.set_recommended_state(
                    recommended_state,
                    &mut self.time_properties_ds,
                    &mut self.current_ds,
                    &mut self.parent_ds,
                    &self.default_ds,
                );
            }
        }

        // And update announce message ages
        for port in ports.iter_mut() {
            port.step_announce_age(bmca_interval);
        }
    }
}

impl<F> PtpInstance<F> {
    /// Construct a new [`PtpInstance`] with the given config and time
    /// properties
    pub fn new(config: InstanceConfig, time_properties_ds: TimePropertiesDS) -> Self {
        let default_ds = InternalDefaultDS::new(config);

        Self {
            state: AtomicRefCell::new(PtpInstanceState {
                default_ds,
                current_ds: Default::default(),
                parent_ds: InternalParentDS::new(default_ds),
                time_properties_ds,
            }),
            log_bmca_interval: AtomicI8::new(i8::MAX),
            _filter: PhantomData,
        }
    }

    /// Return IEEE-1588 defaultDS for introspection
    pub fn default_ds(&self) -> DefaultDS {
        (&self.state.borrow().default_ds).into()
    }

    /// Return IEEE-1588 currentDS for introspection
    pub fn current_ds(&self) -> CurrentDS {
        (&self.state.borrow().current_ds).into()
    }

    /// Return IEEE-1588 parentDS for introspection
    pub fn parent_ds(&self) -> ParentDS {
        (&self.state.borrow().parent_ds).into()
    }

    /// Return IEEE-1588 timePropertiesDS for introspection
    pub fn time_properties_ds(&self) -> TimePropertiesDS {
        self.state.borrow().time_properties_ds
    }
}

impl<F: Filter> PtpInstance<F> {
    /// Add and initialize this port
    ///
    /// We start in the BMCA state because that is convenient
    ///
    /// When providing the port with a different clock than the instance clock,
    /// the caller is responsible for propagating any property changes to this
    /// clock, and for synchronizing this clock with the instance clock as
    /// appropriate based on the ports state.
    pub fn add_port<A, C, R: Rng>(
        &self,
        config: PortConfig<A>,
        filter_config: F::Config,
        clock: C,
        rng: R,
    ) -> Port<'_, InBmca, A, R, C, F> {
        self.log_bmca_interval
            .fetch_min(config.announce_interval.as_log_2(), Ordering::Relaxed);
        let mut state = self.state.borrow_mut();
        let port_identity = PortIdentity {
            clock_identity: state.default_ds.clock_identity,
            port_number: state.default_ds.number_ports,
        };
        state.default_ds.number_ports += 1;

        Port::new(
            &self.state,
            config,
            filter_config,
            clock,
            port_identity,
            rng,
        )
    }

    /// Run the best master clock algorithm (BMCA)
    ///
    /// The caller must pass all the ports that were created on this instance in
    /// the slice!
    pub fn bmca<A: AcceptableMasterList, C: Clock, R: Rng>(
        &self,
        ports: &mut [&mut Port<'_, InBmca, A, R, C, F>],
    ) {
        self.state.borrow_mut().bmca(
            ports,
            Duration::from_seconds(
                2f64.powi(self.log_bmca_interval.load(Ordering::Relaxed) as i32),
            ),
        )
    }

    /// Time to wait between calls to [`PtpInstance::bmca`]
    pub fn bmca_interval(&self) -> core::time::Duration {
        core::time::Duration::from_secs_f64(
            2f64.powi(self.log_bmca_interval.load(Ordering::Relaxed) as i32),
        )
    }
}
