//! Abstraction of a network [`Port`] of a device.
//!
//! See [`Port`] for a detailed description.

use core::ops::Deref;

pub use actions::{
    ForwardedTLV, ForwardedTLVProvider, NoForwardedTLVs, PortAction, PortActionIterator,
    TimestampContext,
};
use atomic_refcell::{AtomicRef, AtomicRefCell};
pub use measurement::Measurement;
use rand::Rng;
use state::{MasterState, PortState};

pub use crate::datastructures::messages::MAX_DATA_LEN;
#[cfg(doc)]
use crate::PtpInstance;
use crate::{
    bmc::{
        acceptable_master::AcceptableMasterList,
        bmca::{BestAnnounceMessage, Bmca},
    },
    clock::Clock,
    config::PortConfig,
    datastructures::{
        common::PortIdentity,
        messages::{Message, MessageBody},
    },
    filters::Filter,
    ptp_instance::PtpInstanceState,
    time::Time,
};

// Needs to be here because of use rules
macro_rules! actions {
    [] => {
        {
            crate::port::PortActionIterator::from(::arrayvec::ArrayVec::new())
        }
    };
    [$action:expr] => {
        {
            let mut list = ::arrayvec::ArrayVec::new();
            list.push($action);
            crate::port::PortActionIterator::from(list)
        }
    };
    [$action1:expr, $action2:expr] => {
        {
            let mut list = ::arrayvec::ArrayVec::new();
            list.push($action1);
            list.push($action2);
            crate::port::PortActionIterator::from(list)
        }
    };
}

mod actions;
mod bmca;
mod measurement;
mod sequence_id;
pub(crate) mod state;

/// A single port of the PTP instance
///
/// One of these needs to be created per port of the PTP instance. They are
/// created by calling [`PtpInstance::add_port`].
///
/// # Generics
/// A [`Port`] is generic over:
/// * **`L`**: The state of the `Port`, either [`InBmca`], or [`Running`].
/// * **`A`**: The type of the [`PortConfig::acceptable_master_list`] which
///   should implement [`AcceptableMasterList`]
/// * **`R`**: The type of the random number generator ([`Rng`]) used to
///   randomize timing
/// * **`C`**: The type of the [`Clock`] used by this [`Port`]
/// * **`F`**: The type of the [`Filter`] used by this [`Port`]
///
///
/// ## Type States
/// A [`Port`] can be in two states. Either in [`Running`] allowing access to
/// the [`handle_*`](`Port::handle_send_timestamp`) methods. Or in [`InBmca`]
/// state where it can be used with a [`PtpInstance`](`crate::PtpInstance`) to
/// run the best master clock algotithm (BMCA).
///
/// To transition from [`InBmca`] to [`Running`] use [`Port::end_bmca`]. To
/// transition from [`Running`] to [`InBmca`] use [`Port::start_bmca`].
///
/// # Example
///
/// ## Initialization
/// A [`Port`] can be created from a [`PtpInstance`]. It requires a
/// [`PortConfig`], a [`Filter::Config`], a [`Clock`], and a [`Rng`].
///
/// ```no_run
/// # mod system {
/// #     pub struct Clock;
/// #     impl statime::Clock for Clock {
/// #         type Error = ();
/// #         fn now(&self) -> statime::time::Time {
/// #             unimplemented!()
/// #         }
/// #         fn step_clock(&mut self, _: statime::time::Duration) -> Result<statime::time::Time, Self::Error> {
/// #             unimplemented!()
/// #         }
/// #         fn set_frequency(&mut self, _: f64) -> Result<statime::time::Time, Self::Error> {
/// #             unimplemented!()
/// #         }
/// #         fn set_properties(&mut self, _: &statime::config::TimePropertiesDS) -> Result<(), Self::Error> {
/// #             unimplemented!()
/// #         }
/// #     }
/// # }
/// use rand::thread_rng;
/// use statime::config::{AcceptAnyMaster, DelayMechanism, PortConfig};
/// use statime::filters::BasicFilter;
/// use statime::PtpInstance;
/// use statime::time::Interval;
///
/// # let (instance_config, time_properties_ds) = unimplemented!();
/// let mut instance = PtpInstance::<BasicFilter>::new(instance_config, time_properties_ds);
///
/// // TODO make these values sensible
/// let interval = Interval::from_log_2(-2); // 2^(-2)s = 250ms
/// let port_config = PortConfig {
///     acceptable_master_list: AcceptAnyMaster,
///     delay_mechanism: DelayMechanism::E2E { interval },
///     announce_interval: interval,
///     announce_receipt_timeout: 0,
///     sync_interval: interval,
///     master_only: false,
///     delay_asymmetry: Default::default(),
/// };
/// let filter_config = 1.0;
/// let clock = system::Clock {};
/// let rng = thread_rng();
///
/// let port_in_bmca = instance.add_port(port_config, filter_config, clock, rng);
///
/// // To handle events for the port it needs to change to running mode
/// let (running_port, actions) = port_in_bmca.end_bmca();
///
/// // This returns the first actions that need to be handled for the port
/// # fn handle_actions<T>(_:T){}
/// handle_actions(actions);
///
/// # running_port.start_bmca(); // Make sure we check `A` bound
/// ```
///
/// ## Handling actions
/// The [`Port`] informs the user about any actions it needs the user to handle
/// via [`PortAction`]s returned from its methods. The user is responsible for
/// handling these events in their system specific way.
///
/// ```no_run
/// use statime::port::{PortAction, PortActionIterator, TimestampContext};
/// use statime::time::Time;
///
/// # mod system {
/// #     pub struct Timer;
/// #     impl Timer {
/// #         pub fn expire_in(&mut self, time: core::time::Duration) {}
/// #     }
/// #     pub struct UdpSocket;
/// #     impl UdpSocket {
/// #         pub fn send(&mut self, buf: &[u8]) -> statime::time::Time { unimplemented!() }
/// #     }
/// # }
/// struct MyPortResources {
///     announce_timer: system::Timer,
///     sync_timer: system::Timer,
///     delay_req_timer: system::Timer,
///     announce_receipt_timer: system::Timer,
///     filter_update_timer: system::Timer,
///     time_critical_socket: system::UdpSocket,
///     general_socket: system::UdpSocket,
///     send_timestamp: Option<(TimestampContext, Time)>
/// }
///
/// fn handle_actions(resources: &mut MyPortResources, actions: PortActionIterator) {
///     for action in actions {
///         match action {
///             PortAction::SendEvent { context, data } => {
///                 let timestamp = resources.time_critical_socket.send(data);
///                 resources.send_timestamp = Some((context, timestamp));
///             }
///             PortAction::SendGeneral { data } => {
///                 resources.general_socket.send(data);
///             }
///             PortAction::ResetAnnounceTimer { duration } => {
///                 resources.announce_timer.expire_in(duration)
///             }
///             PortAction::ResetSyncTimer { duration } => resources.sync_timer.expire_in(duration),
///             PortAction::ResetDelayRequestTimer { duration } => {
///                 resources.delay_req_timer.expire_in(duration)
///             }
///             PortAction::ResetAnnounceReceiptTimer { duration } => {
///                 resources.announce_receipt_timer.expire_in(duration)
///             }
///             PortAction::ResetFilterUpdateTimer { duration } => {
///                 resources.filter_update_timer.expire_in(duration)
///             }
///             PortAction::ForwardTLV { .. } => {}
///         }
///     }
/// }
/// ```
///
/// ## Handling system events
/// After the initialization the user has to inform the [`Port`] about any
/// events relevant to it via the [`handle_*`](`Port::handle_send_timestamp`)
/// methods.
///
/// ```no_run
/// # mod system {
/// #    pub struct Timer;
/// #    impl Timer {
/// #        pub fn has_expired(&self) -> bool { true }
/// #    }
/// #    pub struct UdpSocket;
/// #    impl UdpSocket {
/// #         pub fn recv(&mut self) -> Option<(&'static [u8], statime::time::Time)> { unimplemented!() }
/// #    }
/// # }
/// # struct MyPortResources {
/// #     announce_timer: system::Timer,
/// #     sync_timer: system::Timer,
/// #     delay_req_timer: system::Timer,
/// #     announce_receipt_timer: system::Timer,
/// #     filter_update_timer: system::Timer,
/// #     time_critical_socket: system::UdpSocket,
/// #     general_socket: system::UdpSocket,
/// #     send_timestamp: Option<(statime::port::TimestampContext, statime::time::Time)>
/// # }
///
/// use rand::Rng;
/// use statime::Clock;
/// use statime::config::AcceptableMasterList;
/// use statime::filters::Filter;
/// use statime::port::{NoForwardedTLVs, Port, PortActionIterator, Running};
///
/// fn something_happend(resources: &mut MyPortResources, running_port: &mut Port<Running, impl AcceptableMasterList, impl Rng, impl Clock, impl Filter>) {
///     let actions = if resources.announce_timer.has_expired() {
///         running_port.handle_announce_timer(&mut NoForwardedTLVs)
///     } else if resources.sync_timer.has_expired() {
///         running_port.handle_sync_timer()
///     } else if resources.delay_req_timer.has_expired() {
///         running_port.handle_delay_request_timer()
///     } else if resources.announce_receipt_timer.has_expired() {
///         running_port.handle_announce_receipt_timer()
///     } else if resources.filter_update_timer.has_expired() {
///         running_port.handle_filter_update_timer()
///     } else if let Some((data, timestamp)) = resources.time_critical_socket.recv() {
///         running_port.handle_event_receive(data, timestamp)
///     } else if let Some((data, _timestamp)) = resources.general_socket.recv() {
///         running_port.handle_general_receive(data)
///     } else if let Some((context, timestamp)) = resources.send_timestamp.take() {
///         running_port.handle_send_timestamp(context, timestamp)
///     } else {
///         PortActionIterator::empty()
///     };
///
/// #   fn handle_actions<T,U>(_:T,_:U){}
///     handle_actions(resources, actions);
/// }
/// ```
#[derive(Debug)]
pub struct Port<L, A, R, C, F: Filter> {
    config: PortConfig<()>,
    filter_config: F::Config,
    clock: C,
    // PortDS port_identity
    pub(crate) port_identity: PortIdentity,
    // Corresponds with PortDS port_state and enabled
    port_state: PortState<F>,
    bmca: Bmca<A>,
    packet_buffer: [u8; MAX_DATA_LEN],
    lifecycle: L,
    rng: R,
}

/// Type state of [`Port`] entered by [`Port::end_bmca`]
#[derive(Debug)]
pub struct Running<'a> {
    state_refcell: &'a AtomicRefCell<PtpInstanceState>,
    state: AtomicRef<'a, PtpInstanceState>,
}

/// Type state of [`Port`] entered by [`Port::start_bmca`]
#[derive(Debug)]
pub struct InBmca<'a> {
    pending_action: PortActionIterator<'static>,
    local_best: Option<BestAnnounceMessage>,
    state_refcell: &'a AtomicRefCell<PtpInstanceState>,
}

impl<'a, A: AcceptableMasterList, C: Clock, F: Filter, R: Rng> Port<Running<'a>, A, R, C, F> {
    /// Inform the port about a transmit timestamp being available
    ///
    /// `context` is the handle of the packet that was send from the
    /// [`PortAction::SendEvent`] that caused the send.
    pub fn handle_send_timestamp(
        &mut self,
        context: TimestampContext,
        timestamp: Time,
    ) -> PortActionIterator<'_> {
        let actions = self.port_state.handle_timestamp(
            self.config.delay_asymmetry,
            context,
            timestamp,
            self.port_identity,
            &self.lifecycle.state.default_ds,
            &mut self.clock,
            &mut self.packet_buffer,
        );

        actions
    }

    /// Handle the announce timer going off
    pub fn handle_announce_timer(
        &mut self,
        tlv_provider: &mut impl ForwardedTLVProvider,
    ) -> PortActionIterator<'_> {
        self.port_state.send_announce(
            self.lifecycle.state.deref(),
            &self.config,
            self.port_identity,
            tlv_provider,
            &mut self.packet_buffer,
        )
    }

    /// Handle the sync timer going off
    pub fn handle_sync_timer(&mut self) -> PortActionIterator<'_> {
        self.port_state.send_sync(
            &self.config,
            self.port_identity,
            &self.lifecycle.state.default_ds,
            &mut self.packet_buffer,
        )
    }

    /// Handle the delay request timer going off
    pub fn handle_delay_request_timer(&mut self) -> PortActionIterator<'_> {
        self.port_state.send_delay_request(
            &mut self.rng,
            &self.config,
            self.port_identity,
            &self.lifecycle.state.default_ds,
            &mut self.packet_buffer,
        )
    }

    /// Handle the announce receipt timer going off
    pub fn handle_announce_receipt_timer(&mut self) -> PortActionIterator<'_> {
        // we didn't hear announce messages from other masters, so become master
        // ourselves
        match self.port_state {
            PortState::Master(_) => (),
            _ => self.set_forced_port_state(PortState::Master(MasterState::new())),
        }

        // Immediately start sending syncs and announces
        actions![
            PortAction::ResetAnnounceTimer {
                duration: core::time::Duration::from_secs(0)
            },
            PortAction::ResetSyncTimer {
                duration: core::time::Duration::from_secs(0)
            }
        ]
    }

    /// Handle the filter update timer going off
    pub fn handle_filter_update_timer(&mut self) -> PortActionIterator {
        self.port_state.handle_filter_update(&mut self.clock)
    }

    /// Set this [`Port`] into [`InBmca`] mode to use it with
    /// [`PtpInstance::bmca`].
    pub fn start_bmca(self) -> Port<InBmca<'a>, A, R, C, F> {
        Port {
            port_state: self.port_state,
            config: self.config,
            filter_config: self.filter_config,
            clock: self.clock,
            port_identity: self.port_identity,
            bmca: self.bmca,
            rng: self.rng,
            packet_buffer: [0; MAX_DATA_LEN],
            lifecycle: InBmca {
                pending_action: actions![],
                local_best: None,
                state_refcell: self.lifecycle.state_refcell,
            },
        }
    }

    /// Handle a message over the event channel
    pub fn handle_event_receive<'b>(
        &'b mut self,
        data: &'b [u8],
        timestamp: Time,
    ) -> PortActionIterator<'b> {
        let message = match Message::deserialize(data) {
            Ok(message) => message,
            Err(error) => {
                log::warn!("Could not parse packet: {:?}", error);
                return actions![];
            }
        };

        // Only process messages from the same domain
        if message.header().sdo_id != self.lifecycle.state.default_ds.sdo_id
            || message.header().domain_number != self.lifecycle.state.default_ds.domain_number
        {
            return actions![];
        }

        if message.is_event() {
            self.port_state.handle_event_receive(
                self.config.delay_asymmetry,
                message,
                timestamp,
                self.config.min_delay_req_interval(),
                self.port_identity,
                &mut self.clock,
                &mut self.packet_buffer,
            )
        } else {
            self.handle_general_internal(message)
        }
    }

    /// Handle a general ptp message
    pub fn handle_general_receive<'b>(&'b mut self, data: &'b [u8]) -> PortActionIterator<'b> {
        let message = match Message::deserialize(data) {
            Ok(message) => message,
            Err(error) => {
                log::warn!("Could not parse packet: {:?}", error);
                return actions![];
            }
        };

        // Only process messages from the same domain
        if message.header().sdo_id != self.lifecycle.state.default_ds.sdo_id
            || message.header().domain_number != self.lifecycle.state.default_ds.domain_number
        {
            return actions![];
        }

        self.handle_general_internal(message)
    }

    fn handle_general_internal<'b>(&'b mut self, message: Message<'b>) -> PortActionIterator<'b> {
        match message.body {
            MessageBody::Announce(announce) => self.handle_announce(&message, announce),
            _ => self.port_state.handle_general_receive(
                self.config.delay_asymmetry,
                message,
                self.port_identity,
                &mut self.clock,
            ),
        }
    }
}

impl<'a, A, C, F: Filter, R> Port<InBmca<'a>, A, R, C, F> {
    /// End a BMCA cycle and make the
    /// [`handle_*`](`Port::handle_send_timestamp`) methods available again
    pub fn end_bmca(self) -> (Port<Running<'a>, A, R, C, F>, PortActionIterator<'static>) {
        (
            Port {
                port_state: self.port_state,
                config: self.config,
                filter_config: self.filter_config,
                clock: self.clock,
                port_identity: self.port_identity,
                bmca: self.bmca,
                rng: self.rng,
                packet_buffer: [0; MAX_DATA_LEN],
                lifecycle: Running {
                    state_refcell: self.lifecycle.state_refcell,
                    state: self.lifecycle.state_refcell.borrow(),
                },
            },
            self.lifecycle.pending_action,
        )
    }
}

impl<L, A, R, C: Clock, F: Filter> Port<L, A, R, C, F> {
    fn set_forced_port_state(&mut self, mut state: PortState<F>) {
        log::info!(
            "new state for port {}: {} -> {}",
            self.port_identity.port_number,
            self.port_state,
            state
        );
        core::mem::swap(&mut self.port_state, &mut state);
        state.demobilize_filter(&mut self.clock);
    }
}

impl<L, A, R, C, F: Filter> Port<L, A, R, C, F> {
    /// Indicate whether this [`Port`] is steering its clock.
    pub fn is_steering(&self) -> bool {
        matches!(self.port_state, PortState::Slave(_))
    }

    /// Indicate whether this [`Port`] is in the master state.
    pub fn is_master(&self) -> bool {
        matches!(self.port_state, PortState::Master(_))
    }

    pub(crate) fn state(&self) -> &PortState<F> {
        &self.port_state
    }

    pub(crate) fn number(&self) -> u16 {
        self.port_identity.port_number
    }
}

impl<'a, A, C, F: Filter, R: Rng> Port<InBmca<'a>, A, R, C, F> {
    /// Create a new port from a port dataset on a given interface.
    pub(crate) fn new(
        state_refcell: &'a AtomicRefCell<PtpInstanceState>,
        config: PortConfig<A>,
        filter_config: F::Config,
        clock: C,
        port_identity: PortIdentity,
        mut rng: R,
    ) -> Self {
        let duration = config.announce_duration(&mut rng);
        let bmca = Bmca::new(
            config.acceptable_master_list,
            config.announce_interval.as_duration().into(),
            port_identity,
        );

        Port {
            config: PortConfig {
                acceptable_master_list: (),
                delay_mechanism: config.delay_mechanism,
                announce_interval: config.announce_interval,
                announce_receipt_timeout: config.announce_receipt_timeout,
                sync_interval: config.sync_interval,
                master_only: config.master_only,
                delay_asymmetry: config.delay_asymmetry,
            },
            filter_config,
            clock,
            port_identity,
            port_state: PortState::Listening,
            bmca,
            rng,
            packet_buffer: [0; MAX_DATA_LEN],
            lifecycle: InBmca {
                pending_action: actions![PortAction::ResetAnnounceReceiptTimer { duration }],
                local_best: None,
                state_refcell,
            },
        }
    }
}
