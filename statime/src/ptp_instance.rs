use core::{
    cell::RefCell,
    future::Future,
    pin::{pin, Pin},
};

use futures::StreamExt;

use crate::{
    bmc::bmca::Bmca,
    clock::{Clock, Timer},
    datastructures::datasets::{CurrentDS, DefaultDS, ParentDS, TimePropertiesDS},
    filters::Filter,
    network::NetworkPort,
    port::{Port, PortError, PortInBMCA, Ticker},
    time::Duration,
    utils::SignalContext,
};

/// A PTP node.
///
/// This object handles the complete running of the PTP protocol once created.
/// It provides all the logic for both ordinary and boundary clock mode.
///
/// # Example
/// Assuming we already have a network runtime and clock runtime, an ordinary
/// clock can be run by first creating all the datasets, then creating the port,
/// then finally setting up the instance and starting it:
///
/// ```ignore
/// let default_ds = DefaultDS::new_ordinary_clock(
///     clock_identity,
///     128,
///     128,
///     0,
///     false,
///     SdoId::new(0).unwrap(),
/// );
/// let time_properties_ds =
/// TimePropertiesDS::new_arbitrary_time(false, false, TimeSource::InternalOscillator);
/// let port_ds = PortDS::new(
///     PortIdentity {
///         clock_identity,
///         port_number: 1,
///     },
///     1,
///     1,
///     3,
///     0,
///     DelayMechanism::E2E,
///     1,
/// );
/// let port = Port::new(port_ds, &mut network_runtime, interface_name).await;
/// let mut instance = PtpInstance::new_ordinary_clock(
///     default_ds,
///     time_properties_ds,
///     port,
///     local_clock,
///     BasicFilter::new(0.25),
/// );
///
/// instance.run(&TimerImpl).await;
/// ```
pub struct PtpInstance<P, C, F, const N: usize> {
    default_ds: DefaultDS,
    current_ds: CurrentDS,
    parent_ds: ParentDS,
    time_properties_ds: TimePropertiesDS,
    ports: [Port<P>; N],
    local_clock: RefCell<C>,
    filter: RefCell<F>,
}

// START NEW INTERFACE
impl<P, C, F, const N: usize> PtpInstance<P, C, F, N> {
    #[allow(unused)]
    pub fn bmca(&mut self, ports: &[&mut PortInBMCA]) {
        todo!()
    }

    pub fn bmca_interval(&self) -> std::time::Duration {
        todo!()
    }
}
// END NEW INTERFACE

impl<P, C, F> PtpInstance<P, C, F, 1> {
    /// Create a new ordinary clock instance.
    ///
    /// This creates a PTP ordinary clock with a single port. Note that the port
    /// identity of the provided port needs to have a port number of 1.
    pub fn new_ordinary_clock(
        default_ds: DefaultDS,
        time_properties_ds: TimePropertiesDS,
        port: Port<P>,
        local_clock: C,
        filter: F,
    ) -> Self {
        PtpInstance::new_boundary_clock(default_ds, time_properties_ds, [port], local_clock, filter)
    }
}

impl<P, C, F, const N: usize> PtpInstance<P, C, F, N> {
    /// Create a new boundary clock instance.
    ///
    /// This creates a PTP boundary clock. Multiple ports can be provided to
    /// handle multiple network interfaces. For each provided port, the port
    /// number needs to equal the index of the port in the array plus 1.
    pub fn new_boundary_clock(
        default_ds: DefaultDS,
        time_properties_ds: TimePropertiesDS,
        ports: [Port<P>; N],
        local_clock: C,
        filter: F,
    ) -> Self {
        for (index, port) in ports.iter().enumerate() {
            assert_eq!(port.identity().port_number - 1, index as u16);
        }
        PtpInstance {
            default_ds,
            current_ds: Default::default(),
            parent_ds: ParentDS::new(default_ds),
            time_properties_ds,
            ports,
            local_clock: RefCell::new(local_clock),
            filter: RefCell::new(filter),
        }
    }
}

impl<P: NetworkPort, C: Clock, F: Filter, const N: usize> PtpInstance<P, C, F, N> {
    /// Run the PTP stack.
    ///
    /// This future needs to be awaited for the PTP protocol to be handled and
    /// the clock to be synchronized.
    pub async fn run(&mut self, timer: &impl Timer) -> ! {
        log::info!("Running!");

        let interval = self
            .ports
            .iter()
            .map(|port| port.announce_interval())
            .max()
            .expect("no ports");
        let mut bmca_timeout = pin!(Ticker::new(|interval| timer.after(interval), interval));

        let announce_receipt_timeouts = pin!(into_array::<_, N>(self.ports.iter().map(|port| {
            Ticker::new(
                |interval| timer.after(interval),
                port.announce_receipt_interval(),
            )
        })));
        let sync_timeouts = pin!(into_array::<_, N>(self.ports.iter().map(|port| {
            Ticker::new(|interval| timer.after(interval), port.sync_interval())
        })));
        let announce_timeouts = pin!(into_array::<_, N>(self.ports.iter().map(|port| {
            Ticker::new(|interval| timer.after(interval), port.announce_interval())
        })));

        let mut pinned_announce_receipt_timeouts = into_array::<_, N>(unsafe {
            announce_receipt_timeouts
                .get_unchecked_mut()
                .iter_mut()
                .map(|announce_receipt_timeout| Pin::new_unchecked(announce_receipt_timeout))
        });
        let mut pinned_sync_timeouts = into_array::<_, N>(unsafe {
            sync_timeouts
                .get_unchecked_mut()
                .iter_mut()
                .map(|sync_timeout| Pin::new_unchecked(sync_timeout))
        });
        let mut pinned_announce_timeouts = into_array::<_, N>(unsafe {
            announce_timeouts
                .get_unchecked_mut()
                .iter_mut()
                .map(|announce_timeout| Pin::new_unchecked(announce_timeout))
        });

        let mut stopcontexts = [(); N].map(|_| SignalContext::new());

        loop {
            let mut iter = stopcontexts.iter_mut();
            let stopperpairs =
                core::array::from_fn::<_, N, _>(move |_| iter.next().unwrap().signal());
            let signallers = core::array::from_fn::<_, N, _>(|i| stopperpairs[i].1.clone());
            let signals = stopperpairs.map(|v| v.0);

            let mut run_ports = self
                .ports
                .iter_mut()
                .zip(&mut pinned_announce_receipt_timeouts)
                .zip(&mut pinned_sync_timeouts)
                .zip(&mut pinned_announce_timeouts)
                .zip(signals)
                .map(
                    |(
                        (((port, announce_receipt_timeout), sync_timeout), announce_timeout),
                        stop,
                    )| {
                        port.run_port(
                            &self.local_clock,
                            &self.filter,
                            announce_receipt_timeout,
                            sync_timeout,
                            announce_timeout,
                            &self.default_ds,
                            &self.time_properties_ds,
                            &self.parent_ds,
                            &self.current_ds,
                            stop,
                        )
                    },
                );
            let run_ports =
                embassy_futures::join::join_array([(); N].map(|_| run_ports.next().unwrap()));

            embassy_futures::join::join(
                async {
                    bmca_timeout.next().await;
                    log::trace!("Signalling bmca");
                    signallers.map(|v| v.raise());
                },
                run_ports,
            )
            .await;

            self.run_bmca(&mut pinned_announce_receipt_timeouts);
        }
    }

    fn run_bmca<Fut: Future>(
        &mut self,
        pinned_timeouts: &mut [Pin<&mut Ticker<Fut, impl FnMut(Duration) -> Fut>>],
    ) {
        log::debug!("Running BMCA");
        let mut erbests = [None; N];

        let current_time = self
            .local_clock
            .try_borrow()
            .map(|borrow| borrow.now())
            .map_err(|_| PortError::ClockBusy)
            .unwrap()
            .into();

        for (index, port) in self.ports.iter_mut().enumerate() {
            erbests[index] = port.best_local_announce_message(current_time);
        }

        // TODO: What to do with `None`s?
        let ebest = Bmca::find_best_announce_message(erbests.iter().flatten().cloned());

        for (index, port) in self.ports.iter_mut().enumerate() {
            let recommended_state = Bmca::calculate_recommended_state(
                &self.default_ds,
                ebest,
                erbests[index],
                port.state(),
            );

            log::debug!("Recommended state port {}: {:?}", index, recommended_state);

            if let Some(recommended_state) = recommended_state {
                if let Err(error) = port.set_recommended_state(
                    recommended_state,
                    &mut pinned_timeouts[index],
                    &mut self.time_properties_ds,
                    &mut self.current_ds,
                    &mut self.parent_ds,
                ) {
                    log::error!("{:?}", error)
                }
            }
        }
    }
}

fn into_array<T, const N: usize>(iter: impl IntoIterator<Item = T>) -> [T; N] {
    let mut iter = iter.into_iter();
    let arr = [(); N].map(|_| iter.next().expect("not enough elements"));
    assert!(iter.next().is_none());
    arr
}
