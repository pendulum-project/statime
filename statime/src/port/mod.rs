//! Abstraction of a network [`Port`] of a device.
//!
//! See [`Port`] for a detailed description.

use core::{iter::Fuse, ops::Deref};

use arrayvec::ArrayVec;
use atomic_refcell::{AtomicRef, AtomicRefCell};
pub use measurement::Measurement;
use rand::Rng;
use state::{MasterState, PortState};

use self::state::SlaveState;
pub use crate::datastructures::messages::MAX_DATA_LEN;
#[cfg(doc)]
use crate::PtpInstance;
use crate::{
    bmc::{
        acceptable_master::AcceptableMasterList,
        bmca::{BestAnnounceMessage, Bmca, RecommendedState},
    },
    clock::Clock,
    config::PortConfig,
    datastructures::{
        common::{LeapIndicator, PortIdentity, TimeSource, Tlv, TlvSetIterator},
        datasets::{CurrentDS, DefaultDS, ParentDS, TimePropertiesDS},
        messages::{Message, MessageBody},
    },
    filters::{Filter, FilterUpdate},
    ptp_instance::PtpInstanceState,
    time::{Duration, Time},
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
            PortActionIterator::from(list)
        }
    };
    [$action1:expr, $action2:expr] => {
        {
            let mut list = ::arrayvec::ArrayVec::new();
            list.push($action1);
            list.push($action2);
            PortActionIterator::from(list)
        }
    };
}

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

/// Identification of a packet that should be sent out.
///
/// The caller receives this from a [`PortAction::SendEvent`] and should return
/// it to the [`Port`] with [`Port::handle_send_timestamp`] once the transmit
/// timestamp of that packet is known.
///
/// This type is non-copy and non-clone on purpose to ensures a single
/// [`handle_send_timestamp`](`Port::handle_send_timestamp`) per
/// [`SendEvent`](`PortAction::SendEvent`).
#[derive(Debug)]
pub struct TimestampContext {
    inner: TimestampContextInner,
}

#[derive(Debug)]
enum TimestampContextInner {
    Sync { id: u16 },
    DelayReq { id: u16 },
}

#[derive(Debug, Clone)]
/// TLV that needs to be forwarded in the announce messages of other ports.
pub struct ForwardedTLV<'a> {
    tlv: Tlv<'a>,
    sender_identity: PortIdentity,
}

impl<'a> ForwardedTLV<'a> {
    /// Wire size of the TLV. Can be used to determine how many TLV's to keep
    pub fn size(&self) -> usize {
        self.tlv.wire_size()
    }

    /// Get an owned version of the struct.
    #[cfg(feature = "std")]
    pub fn into_owned(self) -> ForwardedTLV<'static> {
        ForwardedTLV {
            tlv: self.tlv.into_owned(),
            sender_identity: self.sender_identity,
        }
    }
}

/// Source of TLVs that need to be forwarded, provided to announce sender.
pub trait ForwardedTLVProvider {
    /// Should provide the next available TLV, unless it is larger than max_size
    fn next_if_smaller(&mut self, max_size: usize) -> Option<ForwardedTLV>;
}

/// Simple implementation when
#[derive(Debug, Copy, Clone)]
pub struct NoForwardedTLVs;

impl ForwardedTLVProvider for NoForwardedTLVs {
    fn next_if_smaller(&mut self, _max_size: usize) -> Option<ForwardedTLV> {
        None
    }
}

/// An action the [`Port`] needs the user to perform
#[derive(Debug)]
#[must_use]
#[allow(missing_docs)] // Explaining the fields as well as the variants does not add value
pub enum PortAction<'a> {
    /// Send a time-critical packet
    ///
    /// Once the packet is sent and the transmit timestamp known the user should
    /// return the given [`TimestampContext`] using
    /// [`Port::handle_send_timestamp`].
    SendEvent {
        context: TimestampContext,
        data: &'a [u8],
    },
    /// Send a general packet
    ///
    /// For a packet sent this way no timestamp needs to be captured.
    SendGeneral { data: &'a [u8] },
    /// Call [`Port::handle_announce_timer`] in `duration` from now
    ResetAnnounceTimer { duration: core::time::Duration },
    /// Call [`Port::handle_sync_timer`] in `duration` from now
    ResetSyncTimer { duration: core::time::Duration },
    /// Call [`Port::handle_delay_request_timer`] in `duration` from now
    ResetDelayRequestTimer { duration: core::time::Duration },
    /// Call [`Port::handle_announce_receipt_timer`] in `duration` from now
    ResetAnnounceReceiptTimer { duration: core::time::Duration },
    /// Call [`Port::handle_filter_update_timer`] in `duration` from now
    ResetFilterUpdateTimer { duration: core::time::Duration },
    /// Forward this TLV to the announce timer call of all other ports.
    /// The receiver must ensure the TLV is yielded only once to the announce
    /// method of a port.
    ///
    /// This can be ignored when implementing a single port or slave only ptp
    /// instance.
    ForwardTLV { tlv: ForwardedTLV<'a> },
}

const MAX_ACTIONS: usize = 2;

/// An Iterator over [`PortAction`]s
///
/// These are returned by [`Port`] when ever the library needs the user to
/// perform actions to the system.
///
/// **Guarantees to end user:** Any set of actions will only ever contain a
/// single event send
#[derive(Debug)]
#[must_use]
pub struct PortActionIterator<'a> {
    internal: Fuse<<ArrayVec<PortAction<'a>, MAX_ACTIONS> as IntoIterator>::IntoIter>,
    tlvs: TlvSetIterator<'a>,
    sender_identity: PortIdentity,
}

impl<'a> PortActionIterator<'a> {
    /// Get an empty Iterator
    ///
    /// This can for example be used to have a default value in chained `if`
    /// statements.
    pub fn empty() -> Self {
        Self {
            internal: ArrayVec::new().into_iter().fuse(),
            tlvs: TlvSetIterator::empty(),
            sender_identity: Default::default(),
        }
    }
    fn from(list: ArrayVec<PortAction<'a>, MAX_ACTIONS>) -> Self {
        Self {
            internal: list.into_iter().fuse(),
            tlvs: TlvSetIterator::empty(),
            sender_identity: Default::default(),
        }
    }
    fn from_filter(update: FilterUpdate) -> Self {
        if let Some(duration) = update.next_update {
            actions![PortAction::ResetFilterUpdateTimer { duration }]
        } else {
            actions![]
        }
    }
    fn with_forward_tlvs(self, tlvs: TlvSetIterator<'a>, sender_identity: PortIdentity) -> Self {
        Self {
            internal: self.internal,
            tlvs,
            sender_identity,
        }
    }
}

impl<'a> Iterator for PortActionIterator<'a> {
    type Item = PortAction<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.internal.next().or_else(|| loop {
            let tlv = self.tlvs.next()?;
            if tlv.tlv_type.announce_propagate() {
                return Some(PortAction::ForwardTLV {
                    tlv: ForwardedTLV {
                        tlv,
                        sender_identity: self.sender_identity,
                    },
                });
            }
        })
    }
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
            MessageBody::Announce(announce) => {
                if self
                    .bmca
                    .register_announce_message(&message.header, &announce)
                {
                    actions![PortAction::ResetAnnounceReceiptTimer {
                        duration: self.config.announce_duration(&mut self.rng),
                    }]
                    .with_forward_tlvs(message.suffix.tlv(), message.header.source_port_identity)
                } else {
                    actions![]
                }
            }
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

impl<'a, A: AcceptableMasterList, C: Clock, F: Filter, R: Rng> Port<InBmca<'a>, A, R, C, F> {
    pub(crate) fn calculate_best_local_announce_message(&mut self) {
        self.lifecycle.local_best = self.bmca.take_best_port_announce_message()
    }
}

impl<'a, A, C: Clock, F: Filter, R: Rng> Port<InBmca<'a>, A, R, C, F> {
    pub(crate) fn step_announce_age(&mut self, step: Duration) {
        self.bmca.step_age(step);
    }

    pub(crate) fn best_local_announce_message_for_bmca(&self) -> Option<BestAnnounceMessage> {
        // Announce messages received on a masterOnly PTP Port shall not be considered
        // in the global operation of the best master clock algorithm or in the update
        // of data sets. We still need them during the calculation of the recommended
        // port state though to avoid getting multiple masters in the segment.
        if self.config.master_only {
            None
        } else {
            self.lifecycle.local_best
        }
    }

    pub(crate) fn best_local_announce_message_for_state(&self) -> Option<BestAnnounceMessage> {
        // Announce messages received on a masterOnly PTP Port shall not be considered
        // in the global operation of the best master clock algorithm or in the update
        // of data sets. We still need them during the calculation of the recommended
        // port state though to avoid getting multiple masters in the segment.
        self.lifecycle.local_best
    }

    pub(crate) fn set_recommended_state(
        &mut self,
        recommended_state: RecommendedState,
        time_properties_ds: &mut TimePropertiesDS,
        current_ds: &mut CurrentDS,
        parent_ds: &mut ParentDS,
        default_ds: &DefaultDS,
    ) {
        self.set_recommended_port_state(&recommended_state, default_ds);

        match recommended_state {
            RecommendedState::M1(defaultds) | RecommendedState::M2(defaultds) => {
                // a slave-only PTP port should never end up in the master state
                debug_assert!(!default_ds.slave_only);

                current_ds.steps_removed = 0;
                current_ds.offset_from_master = Duration::ZERO;
                current_ds.mean_delay = Duration::ZERO;

                parent_ds.parent_port_identity.clock_identity = defaultds.clock_identity;
                parent_ds.parent_port_identity.port_number = 0;
                parent_ds.grandmaster_identity = defaultds.clock_identity;
                parent_ds.grandmaster_clock_quality = defaultds.clock_quality;
                parent_ds.grandmaster_priority_1 = defaultds.priority_1;
                parent_ds.grandmaster_priority_2 = defaultds.priority_2;

                time_properties_ds.leap_indicator = LeapIndicator::NoLeap;
                time_properties_ds.current_utc_offset = None;
                time_properties_ds.ptp_timescale = true;
                time_properties_ds.time_traceable = false;
                time_properties_ds.frequency_traceable = false;
                time_properties_ds.time_source = TimeSource::InternalOscillator;
            }
            RecommendedState::M3(_) | RecommendedState::P1(_) | RecommendedState::P2(_) => {}
            RecommendedState::S1(announce_message) => {
                // a master-only PTP port should never end up in the slave state
                debug_assert!(!self.config.master_only);

                current_ds.steps_removed = announce_message.steps_removed + 1;

                parent_ds.parent_port_identity = announce_message.header.source_port_identity;
                parent_ds.grandmaster_identity = announce_message.grandmaster_identity;
                parent_ds.grandmaster_clock_quality = announce_message.grandmaster_clock_quality;
                parent_ds.grandmaster_priority_1 = announce_message.grandmaster_priority_1;
                parent_ds.grandmaster_priority_2 = announce_message.grandmaster_priority_2;

                *time_properties_ds = announce_message.time_properties();

                if let Err(error) = self.clock.set_properties(time_properties_ds) {
                    log::error!("Could not update clock: {:?}", error);
                }
            }
        }

        // TODO: Discuss if we should change the clock's own time properties, or keep
        // the master's time properties separately
        if let RecommendedState::S1(announce_message) = &recommended_state {
            // Update time properties
            *time_properties_ds = announce_message.time_properties();
        }
    }

    fn set_recommended_port_state(
        &mut self,
        recommended_state: &RecommendedState,
        default_ds: &DefaultDS,
    ) {
        match recommended_state {
            // TODO set things like steps_removed once they are added
            // TODO make sure states are complete
            RecommendedState::S1(announce_message) => {
                // a master-only PTP port should never end up in the slave state
                debug_assert!(!self.config.master_only);

                let remote_master = announce_message.header.source_port_identity;

                let update_state = match &self.port_state {
                    PortState::Listening | PortState::Master(_) | PortState::Passive => true,
                    PortState::Slave(old_state) => old_state.remote_master() != remote_master,
                };

                if update_state {
                    let state = PortState::Slave(SlaveState::new(
                        remote_master,
                        self.filter_config.clone(),
                    ));
                    self.set_forced_port_state(state);

                    let duration = self.config.announce_duration(&mut self.rng);
                    let reset_announce = PortAction::ResetAnnounceReceiptTimer { duration };
                    let reset_delay = PortAction::ResetDelayRequestTimer {
                        duration: core::time::Duration::ZERO,
                    };
                    self.lifecycle.pending_action = actions![reset_announce, reset_delay];
                }
            }
            RecommendedState::M1(_) | RecommendedState::M2(_) | RecommendedState::M3(_) => {
                if default_ds.slave_only {
                    match self.port_state {
                        PortState::Listening => { /* do nothing */ }
                        PortState::Slave(_) | PortState::Passive => {
                            self.set_forced_port_state(PortState::Listening);

                            // consistent with Port<InBmca>::new()
                            let duration = self.config.announce_duration(&mut self.rng);
                            let reset_announce = PortAction::ResetAnnounceReceiptTimer { duration };
                            self.lifecycle.pending_action = actions![reset_announce];
                        }
                        PortState::Master(_) => {
                            let msg = "slave-only PTP port should not be in master state";
                            debug_assert!(!default_ds.slave_only, "{msg}");
                            log::error!("{msg}");
                        }
                    }
                } else {
                    match self.port_state {
                        PortState::Listening | PortState::Slave(_) | PortState::Passive => {
                            self.set_forced_port_state(PortState::Master(MasterState::new()));

                            // Immediately start sending announces and syncs
                            let duration = core::time::Duration::from_secs(0);
                            self.lifecycle.pending_action = actions![
                                PortAction::ResetAnnounceTimer { duration },
                                PortAction::ResetSyncTimer { duration }
                            ];
                        }
                        PortState::Master(_) => { /* do nothing */ }
                    }
                }
            }
            RecommendedState::P1(_) | RecommendedState::P2(_) => match self.port_state {
                PortState::Listening | PortState::Slave(_) | PortState::Master(_) => {
                    self.set_forced_port_state(PortState::Passive)
                }
                PortState::Passive => {}
            },
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        bmc::acceptable_master::AcceptAnyMaster,
        config::{DelayMechanism, InstanceConfig},
        datastructures::messages::{AnnounceMessage, Header, PtpVersion},
        filters::BasicFilter,
        time::Interval,
    };

    struct TestClock;

    impl Clock for TestClock {
        type Error = ();

        fn set_frequency(&mut self, _freq: f64) -> Result<Time, Self::Error> {
            Ok(Time::default())
        }

        fn now(&self) -> Time {
            panic!("Shouldn't be called");
        }

        fn set_properties(
            &mut self,
            _time_properties_ds: &TimePropertiesDS,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        fn step_clock(&mut self, _offset: Duration) -> Result<Time, Self::Error> {
            Ok(Time::default())
        }
    }

    fn default_announce_message_header() -> Header {
        Header {
            sdo_id: Default::default(),
            version: PtpVersion::new(2, 1).unwrap(),
            domain_number: Default::default(),
            alternate_master_flag: false,
            two_step_flag: false,
            unicast_flag: false,
            ptp_profile_specific_1: false,
            ptp_profile_specific_2: false,
            leap61: false,
            leap59: false,
            current_utc_offset_valid: false,
            ptp_timescale: false,
            time_tracable: false,
            frequency_tracable: false,
            synchronization_uncertain: false,
            correction_field: Default::default(),
            source_port_identity: Default::default(),
            sequence_id: Default::default(),
            log_message_interval: Default::default(),
        }
    }

    fn default_announce_message() -> AnnounceMessage {
        AnnounceMessage {
            header: default_announce_message_header(),
            origin_timestamp: Default::default(),
            current_utc_offset: Default::default(),
            grandmaster_priority_1: Default::default(),
            grandmaster_clock_quality: Default::default(),
            grandmaster_priority_2: Default::default(),
            grandmaster_identity: Default::default(),
            steps_removed: Default::default(),
            time_source: Default::default(),
        }
    }

    #[test]
    fn test_announce_receive() {
        let default_ds = DefaultDS::new(InstanceConfig {
            clock_identity: Default::default(),
            priority_1: 255,
            priority_2: 255,
            domain_number: 0,
            slave_only: false,
            sdo_id: Default::default(),
        });

        let parent_ds = ParentDS::new(default_ds);

        let state = AtomicRefCell::new(PtpInstanceState {
            default_ds,
            current_ds: Default::default(),
            parent_ds,
            time_properties_ds: Default::default(),
        });

        let port = Port::<_, _, _, _, BasicFilter>::new(
            &state,
            PortConfig {
                acceptable_master_list: AcceptAnyMaster,
                delay_mechanism: DelayMechanism::E2E {
                    interval: Interval::from_log_2(1),
                },
                announce_interval: Interval::from_log_2(1),
                announce_receipt_timeout: 3,
                sync_interval: Interval::from_log_2(0),
                master_only: false,
                delay_asymmetry: Duration::ZERO,
            },
            0.25,
            TestClock,
            Default::default(),
            rand::rngs::mock::StepRng::new(2, 1),
        );

        let (mut port, _) = port.end_bmca();

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity.0 = [1, 2, 3, 4, 5, 6, 7, 8];
        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: Default::default(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];

        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut port = port.start_bmca();
        port.calculate_best_local_announce_message();
        assert!(port.best_local_announce_message_for_bmca().is_some());
    }

    #[test]
    fn test_announce_receive_via_event() {
        let default_ds = DefaultDS::new(InstanceConfig {
            clock_identity: Default::default(),
            priority_1: 255,
            priority_2: 255,
            domain_number: 0,
            slave_only: false,
            sdo_id: Default::default(),
        });

        let parent_ds = ParentDS::new(default_ds);

        let state = AtomicRefCell::new(PtpInstanceState {
            default_ds,
            current_ds: Default::default(),
            parent_ds,
            time_properties_ds: Default::default(),
        });

        let port = Port::<_, _, _, _, BasicFilter>::new(
            &state,
            PortConfig {
                acceptable_master_list: AcceptAnyMaster,
                delay_mechanism: DelayMechanism::E2E {
                    interval: Interval::from_log_2(1),
                },
                announce_interval: Interval::from_log_2(1),
                announce_receipt_timeout: 3,
                sync_interval: Interval::from_log_2(0),
                master_only: false,
                delay_asymmetry: Duration::ZERO,
            },
            0.25,
            TestClock,
            Default::default(),
            rand::rngs::mock::StepRng::new(2, 1),
        );

        let (mut port, _) = port.end_bmca();

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity.0 = [1, 2, 3, 4, 5, 6, 7, 8];
        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: Default::default(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];

        let mut actions = port.handle_event_receive(packet, Time::from_micros(1));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_event_receive(packet, Time::from_micros(2));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_event_receive(packet, Time::from_micros(3));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut port = port.start_bmca();
        port.calculate_best_local_announce_message();
        assert!(port.best_local_announce_message_for_bmca().is_some());
    }
}
