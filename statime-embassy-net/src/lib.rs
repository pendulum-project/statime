#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod embassy_clock;
#[cfg(feature = "monitor")]
mod monitor;
mod storage;

use core::num::NonZero;
use embassy_futures::select::{Either3, select3};
use embassy_net::{
    IpAddress, IpEndpoint, Ipv4Address, MulticastError, Stack, TryError,
    driver::{Timestamp, TxTimestamp},
    udp,
    udp::{UdpMetadata, UdpSocket},
};
use embassy_time::{Duration as EmbassyDuration, Instant, with_deadline};
use rand_core::SeedableRng;
use rand_xorshift::XorShiftRng;
use statime::{
    Clock, PtpInstance,
    config::{
        AcceptAnyMaster, ClockIdentity, ClockQuality, DelayMechanism, InstanceConfig, PortConfig,
        PtpMinorVersion, TimePropertiesDS, TimeSource,
    },
    filters::{Filter, FixedWanderKalmanFilter},
    observability::port::PortState,
    port::{NoForwardedTLVs, PortAction, PortActionIterator, TimestampContext},
    time::{Duration, Interval, Time},
};

#[cfg(feature = "defmt")]
macro_rules! info {
    ($($arg:tt)*) => { defmt::info!($($arg)*) };
}

#[cfg(not(feature = "defmt"))]
macro_rules! info {
    ($format:literal $(, $arg:expr)* $(,)?) => {{
        let _ = $format;
        $(let _ = &$arg;)*
    }};
}

#[cfg(feature = "defmt")]
macro_rules! warn {
    ($($arg:tt)*) => { defmt::warn!($($arg)*) };
}

#[cfg(not(feature = "defmt"))]
macro_rules! warn {
    ($format:literal $(, $arg:expr)* $(,)?) => {{
        let _ = $format;
        $(let _ = &$arg;)*
    }};
}

pub use embassy_clock::{EmbassyClock, EmbassyClockError};
#[cfg(feature = "monitor")]
pub use monitor::{ClockState, PtpMonitor};
pub use storage::PtpStorage;

const EVENT_PORT: u16 = 319;
const GENERAL_PORT: u16 = 320;
const PRIMARY_MULTICAST: Ipv4Address = Ipv4Address::new(224, 0, 1, 129);
const LINK_LOCAL_MULTICAST: Ipv4Address = Ipv4Address::new(224, 0, 0, 107);
const TX_TIMESTAMP_TIMEOUT: EmbassyDuration = EmbassyDuration::from_millis(100);
const TX_PENDING: usize = 4;
const MSG_DELAY_REQ: u8 = 0x1;
const MSG_PDELAY_REQ: u8 = 0x2;
const MSG_PDELAY_RESP: u8 = 0x3;

/// Error encountered while starting a [`Runner`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum RunError {
    /// The network stack could not join a required PTP multicast group.
    Multicast {
        /// Multicast address the runner tried to join.
        address: Ipv4Address,
        /// Error reported by the network stack.
        source: MulticastError,
    },
    /// A PTP UDP socket could not be bound.
    Bind {
        /// UDP port the runner tried to bind.
        port: u16,
        /// Error reported by the socket.
        source: udp::BindError,
    },
}

impl core::fmt::Display for RunError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Multicast { address, source } => {
                write!(formatter, "joining PTP multicast group {address}: {source}")
            }
            Self::Bind { port, source } => {
                write!(formatter, "binding PTP UDP port {port}: {source:?}")
            }
        }
    }
}

impl core::error::Error for RunError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Multicast { source, .. } => Some(source),
            Self::Bind { .. } => None,
        }
    }
}

/// Configuration for one PTP ordinary-clock runner.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    /// Ethernet MAC address used to derive the PTP clock identity.
    pub mac_address: [u8; 6],
    /// Seed for statime's per-port random number generator.
    pub rng_seed: u64,
    /// PTP domain number accepted and transmitted by this ordinary clock.
    pub domain_number: u8,
    /// Best master clock algorithm priority1 value.
    pub priority_1: u8,
    /// Best master clock algorithm priority2 value.
    pub priority_2: u8,
    /// Keep this clock out of master state.
    pub slave_only: bool,
    /// Accuracy and stability advertised when this clock becomes master.
    pub clock_quality: ClockQuality,
    /// Logarithmic E2E delay request interval.
    pub delay_request_interval: Interval,
    /// Logarithmic announce interval.
    pub announce_interval: Interval,
    /// Logarithmic sync interval.
    pub sync_interval: Interval,
    /// Number of missed announces before announce receipt timeout.
    pub announce_receipt_timeout: u8,
    /// Static path delay asymmetry correction.
    pub delay_asymmetry: Duration,
    /// PTP v2 minor version used by the statime port.
    pub minor_ptp_version: PtpMinorVersion,
    /// Time properties advertised by this clock when it becomes master.
    pub time_properties: TimePropertiesDS,
    /// Maximum time to wait for a hardware transmit timestamp.
    pub tx_timestamp_timeout: EmbassyDuration,
}

impl Config {
    /// Create a slave-only ordinary-clock configuration for `mac_address`.
    ///
    /// The MAC address defines the clock identity and `rng_seed` seeds
    /// statime's per-port random scheduling.
    pub fn new(mac_address: [u8; 6], rng_seed: u64) -> Self {
        Self {
            mac_address,
            rng_seed,
            domain_number: 0,
            priority_1: 128,
            priority_2: 128,
            slave_only: true,
            clock_quality: ClockQuality::default(),
            delay_request_interval: Interval::from_log_2(0),
            announce_interval: Interval::from_log_2(0),
            sync_interval: Interval::from_log_2(0),
            announce_receipt_timeout: 3,
            delay_asymmetry: Duration::ZERO,
            minor_ptp_version: PtpMinorVersion::Zero,
            time_properties: TimePropertiesDS::new_arbitrary_time(
                false,
                false,
                TimeSource::InternalOscillator,
            ),
            tx_timestamp_timeout: TX_TIMESTAMP_TIMEOUT,
        }
    }
}

/// Single-port PTP ordinary-clock service.
///
/// Construct one runner per Ethernet port and call [`run`](Self::run) from a
/// background task. Dropping the future is not a supported recovery path; this
/// is intended to run for the lifetime of the network stack.
///
/// `F` selects the Statime measurement filter. Its [`Filter::Config`] is
/// supplied to [`new`](Self::new), keeping filter policy outside the Embassy
/// transport adapter.
///
/// The clock must control the same hardware time domain used for packet
/// timestamps by the underlying network driver.
pub struct Runner<'a, C, F: Filter = FixedWanderKalmanFilter> {
    stack: Stack<'a>,
    clock: C,
    storage: &'a mut PtpStorage,
    config: Config,
    filter_config: F::Config,
    #[cfg(feature = "monitor")]
    monitor: Option<&'a PtpMonitor>,
}

struct ClockRef<'a, C> {
    clock: &'a mut C,
    #[cfg(feature = "monitor")]
    monitor: Option<&'a PtpMonitor>,
}

impl<C: Clock> Clock for ClockRef<'_, C> {
    type Error = C::Error;

    fn now(&self) -> Time {
        self.clock.now()
    }

    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
        let result = self.clock.step_clock(offset);
        #[cfg(feature = "monitor")]
        if result.is_ok()
            && let Some(monitor) = self.monitor
        {
            monitor.unavailable();
        }
        result
    }

    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
        self.clock.set_frequency(ppm)
    }

    fn set_properties(&mut self, time_properties_ds: &TimePropertiesDS) -> Result<(), Self::Error> {
        let result = self.clock.set_properties(time_properties_ds);
        #[cfg(feature = "monitor")]
        if result.is_ok()
            && let Some(monitor) = self.monitor
        {
            monitor.set_ptp_timescale(time_properties_ds.ptp_timescale);
        }
        result
    }
}

impl<'a, C: Clock, F: Filter> Runner<'a, C, F> {
    /// Bind the PTP service to an Embassy network stack, PTP clock, socket
    /// storage, protocol configuration, and filter configuration.
    pub fn new(
        stack: Stack<'a>,
        clock: C,
        storage: &'a mut PtpStorage,
        config: Config,
        filter_config: F::Config,
    ) -> Self {
        Self {
            stack,
            clock,
            storage,
            config,
            filter_config,
            #[cfg(feature = "monitor")]
            monitor: None,
        }
    }

    /// Report clock tracking state through `monitor`.
    #[cfg(feature = "monitor")]
    pub fn with_monitor(mut self, monitor: &'a PtpMonitor) -> Self {
        self.monitor = Some(monitor);
        self
    }

    /// Run the PTP service until cancelled, or return an initialization error.
    ///
    /// Transient link or IP-configuration loss does not return an error. The
    /// runner retains its sockets and protocol state and resumes when the
    /// network stack becomes usable again.
    pub async fn run(&mut self) -> Result<(), RunError> {
        let stack = self.stack;
        let clock = ClockRef {
            clock: &mut self.clock,
            #[cfg(feature = "monitor")]
            monitor: self.monitor,
        };
        let storage = &mut *self.storage;
        let config = self.config;
        let filter_config = self.filter_config.clone();

        info!("ptp: waiting for network configuration");
        stack.wait_config_up().await;
        stack
            .join_multicast_group(PRIMARY_MULTICAST)
            .map_err(|source| RunError::Multicast {
                address: PRIMARY_MULTICAST,
                source,
            })?;
        info!("ptp: joined primary multicast group");
        stack
            .join_multicast_group(LINK_LOCAL_MULTICAST)
            .map_err(|source| RunError::Multicast {
                address: LINK_LOCAL_MULTICAST,
                source,
            })?;
        info!("ptp: joined link-local multicast group");

        let event_socket = UdpSocket::new(
            stack,
            &mut storage.event.rx_meta,
            &mut storage.event.rx_buffer,
            &mut storage.event.tx_meta,
            &mut storage.event.tx_buffer,
        );
        let general_socket = UdpSocket::new(
            stack,
            &mut storage.general.rx_meta,
            &mut storage.general.rx_buffer,
            &mut storage.general.tx_meta,
            &mut storage.general.tx_buffer,
        );
        let mut io = PortIo::new(event_socket, general_socket, config.tx_timestamp_timeout)?;

        let clock_identity = clock_identity_from_mac(config.mac_address);
        info!(
            "ptp: clock identity {=u64:#020x}",
            u64::from_be_bytes(clock_identity.0),
        );

        let instance = PtpInstance::<F>::new(
            InstanceConfig {
                clock_identity,
                priority_1: config.priority_1,
                priority_2: config.priority_2,
                domain_number: config.domain_number,
                sdo_id: Default::default(),
                slave_only: config.slave_only,
                path_trace: false,
                clock_quality: config.clock_quality,
            },
            config.time_properties,
        );
        let port = instance.add_port(
            PortConfig {
                acceptable_master_list: AcceptAnyMaster,
                delay_mechanism: DelayMechanism::E2E {
                    interval: config.delay_request_interval,
                },
                announce_interval: config.announce_interval,
                announce_receipt_timeout: config.announce_receipt_timeout,
                sync_interval: config.sync_interval,
                master_only: false,
                delay_asymmetry: config.delay_asymmetry,
                minor_ptp_version: config.minor_ptp_version,
            },
            filter_config,
            clock,
            XorShiftRng::seed_from_u64(config.rng_seed),
        );
        let (mut port, actions) = port.end_bmca();

        let mut forwarded_tlvs = NoForwardedTLVs;
        let mut bmca = deadline_from_now(instance.bmca_interval());
        let mut tx_timestamp: Option<TxTimestamp> = None;

        io.handle(
            actions,
            #[cfg(feature = "monitor")]
            self.monitor,
        )
        .await;
        info!("ptp: task started");

        loop {
            if Instant::now() >= bmca {
                bmca = deadline_from_now(instance.bmca_interval());
                let old_state = port.port_ds().port_state;
                let mut bmca_port = port.start_bmca();
                instance.bmca(&mut [&mut bmca_port]);
                let (running_port, actions) = bmca_port.end_bmca();
                port = running_port;
                let new_state = port.port_ds().port_state;
                #[cfg(feature = "monitor")]
                if new_state != PortState::Slave
                    && let Some(monitor) = self.monitor
                {
                    monitor.holdover();
                }
                if new_state != old_state {
                    info!(
                        "ptp: state {} -> {}",
                        state_name(old_state),
                        state_name(new_state)
                    );
                }
                io.handle(
                    actions,
                    #[cfg(feature = "monitor")]
                    self.monitor,
                )
                .await;
                continue;
            }

            let actions = if let Some(timestamp) = tx_timestamp.take() {
                match io.pending_tx.take(timestamp.id) {
                    Some(context) => {
                        port.handle_send_timestamp(context, time_from(timestamp.timestamp))
                    }
                    None => PortActionIterator::empty(),
                }
            } else if let Some(timer) = io.timers.take_due() {
                match timer {
                    StatimeTimer::Announce => port.handle_announce_timer(&mut forwarded_tlvs),
                    StatimeTimer::Sync => port.handle_sync_timer(),
                    StatimeTimer::DelayRequest => port.handle_delay_request_timer(),
                    StatimeTimer::AnnounceReceipt => port.handle_announce_receipt_timer(),
                    StatimeTimer::FilterUpdate => {
                        #[cfg(feature = "monitor")]
                        if let Some(monitor) = self.monitor {
                            monitor.holdover();
                        }
                        port.handle_filter_update_timer()
                    }
                }
            } else {
                let receive_delay_requests = port.port_ds().port_state == PortState::Master;
                match io.receive(&mut storage.packet, receive_delay_requests) {
                    Incoming::Event(packet, timestamp) => {
                        port.handle_event_receive(packet, time_from(timestamp))
                    }
                    Incoming::General(packet) => port.handle_general_receive(packet),
                    Incoming::None => PortActionIterator::empty(),
                }
            };
            io.pending_tx.expire();

            io.handle(
                actions,
                #[cfg(feature = "monitor")]
                self.monitor,
            )
            .await;

            let next = io.next_deadline(bmca);
            tx_timestamp = io.wait(stack, next).await;
        }
    }
}

struct PortIo<'a> {
    event: UdpSocket<'a>,
    general: UdpSocket<'a>,
    timers: Timers,
    packet_id: PacketIdGenerator,
    pending_tx: PendingTxQueue,
}

impl<'a> PortIo<'a> {
    fn new(
        mut event: UdpSocket<'a>,
        mut general: UdpSocket<'a>,
        tx_timestamp_timeout: EmbassyDuration,
    ) -> Result<Self, RunError> {
        event.bind(EVENT_PORT).map_err(|source| RunError::Bind {
            port: EVENT_PORT,
            source,
        })?;
        general
            .bind(GENERAL_PORT)
            .map_err(|source| RunError::Bind {
                port: GENERAL_PORT,
                source,
            })?;
        Ok(Self {
            event,
            general,
            timers: Timers::default(),
            packet_id: PacketIdGenerator::new(),
            pending_tx: PendingTxQueue::new(tx_timestamp_timeout),
        })
    }

    async fn handle(
        &mut self,
        actions: PortActionIterator<'_>,
        #[cfg(feature = "monitor")] monitor: Option<&PtpMonitor>,
    ) {
        for action in actions {
            match action {
                PortAction::SendEvent {
                    context,
                    data,
                    link_local,
                } => {
                    let metadata = UdpMetadata {
                        endpoint: multicast_endpoint(EVENT_PORT, link_local),
                        meta: self.packet_id.next(),
                        local_address: None,
                    };
                    match self.event.send_to(data, metadata).await {
                        Ok(()) => self.pending_tx.push(context, metadata.meta.id),
                        Err(error) => warn!("ptp: event send failed: {}", &error),
                    }
                }
                PortAction::SendGeneral { data, link_local } => {
                    let metadata = UdpMetadata {
                        endpoint: multicast_endpoint(GENERAL_PORT, link_local),
                        meta: udp::PacketMeta::default(),
                        local_address: None,
                    };
                    if let Err(error) = self.general.send_to(data, metadata).await {
                        warn!("ptp: general send failed: {}", error);
                    }
                }
                PortAction::ResetAnnounceTimer { duration } => {
                    self.timers.reset(StatimeTimer::Announce, duration)
                }
                PortAction::ResetSyncTimer { duration } => {
                    self.timers.reset(StatimeTimer::Sync, duration)
                }
                PortAction::ResetDelayRequestTimer { duration } => {
                    self.timers.reset(StatimeTimer::DelayRequest, duration)
                }
                PortAction::ResetAnnounceReceiptTimer { duration } => {
                    self.timers.reset(StatimeTimer::AnnounceReceipt, duration)
                }
                PortAction::ResetFilterUpdateTimer { duration } => {
                    #[cfg(feature = "monitor")]
                    if let Some(monitor) = monitor {
                        monitor.tracking();
                    }
                    self.timers.reset(StatimeTimer::FilterUpdate, duration)
                }
                PortAction::ForwardTLV { .. } => {}
            }
        }
    }

    fn receive<'b>(&self, packet: &'b mut [u8], receive_delay_requests: bool) -> Incoming<'b> {
        match self.event.try_recv_from(packet) {
            Ok((n, meta)) => {
                let packet = &packet[..n];
                return rx_event_timestamp(packet, meta.meta, receive_delay_requests)
                    .map_or(Incoming::None, |timestamp| {
                        Incoming::Event(packet, timestamp)
                    });
            }
            Err(TryError::Other(udp::RecvError::Truncated)) => {
                warn!("ptp: truncated event packet");
                return Incoming::None;
            }
            Err(TryError::WouldBlock) => {}
        }

        match self.general.try_recv_from(packet) {
            Ok((n, _)) if ptp_message_type(&packet[..n]).is_some() => {
                Incoming::General(&packet[..n])
            }
            Ok(_) | Err(TryError::WouldBlock) => Incoming::None,
            Err(TryError::Other(udp::RecvError::Truncated)) => {
                warn!("ptp: truncated general packet");
                Incoming::None
            }
        }
    }

    fn next_deadline(&self, deadline: Instant) -> Instant {
        [
            self.timers.next_deadline(),
            self.pending_tx.next_timeout_deadline(),
        ]
        .into_iter()
        .flatten()
        .fold(deadline, Ord::min)
    }

    async fn wait(&self, stack: Stack<'_>, deadline: Instant) -> Option<TxTimestamp> {
        match with_deadline(
            deadline,
            select3(
                stack.poll_tx_timestamps(),
                self.event.wait_recv_ready(),
                self.general.wait_recv_ready(),
            ),
        )
        .await
        {
            Ok(Either3::First(timestamp)) => Some(timestamp),
            _ => None,
        }
    }
}

enum Incoming<'a> {
    Event(&'a [u8], Timestamp),
    General(&'a [u8]),
    None,
}

fn rx_event_timestamp(
    packet: &[u8],
    meta: udp::PacketMeta,
    receive_delay_requests: bool,
) -> Option<Timestamp> {
    let message_type = ptp_message_type(packet)?;
    if matches!(message_type, MSG_PDELAY_REQ | MSG_PDELAY_RESP)
        || message_type == MSG_DELAY_REQ && !receive_delay_requests
    {
        return None;
    }
    let timestamp = meta.timestamp;
    if timestamp.is_none() {
        warn!(
            "ptp: missing rx timestamp packet_id={=u32} message_type={=u8}",
            meta.id, message_type
        );
    }
    timestamp
}

fn ptp_message_type(packet: &[u8]) -> Option<u8> {
    packet.get(..34)?;
    Some(packet[0] & 0x0f)
}

#[derive(Default)]
struct Timers([Option<Instant>; StatimeTimer::ALL.len()]);

impl Timers {
    fn reset(&mut self, timer: StatimeTimer, duration: core::time::Duration) {
        self.0[timer as usize] = Some(deadline_from_now(duration));
    }

    fn take_due(&mut self) -> Option<StatimeTimer> {
        let now = Instant::now();
        StatimeTimer::ALL.into_iter().find(|&timer| {
            self.0[timer as usize]
                .take_if(|deadline| now >= *deadline)
                .is_some()
        })
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.0.into_iter().flatten().min()
    }
}

#[repr(usize)]
#[derive(Clone, Copy)]
enum StatimeTimer {
    Announce,
    Sync,
    DelayRequest,
    AnnounceReceipt,
    FilterUpdate,
}

impl StatimeTimer {
    const ALL: [Self; 5] = [
        Self::Announce,
        Self::Sync,
        Self::DelayRequest,
        Self::AnnounceReceipt,
        Self::FilterUpdate,
    ];
}

struct PendingTx {
    context: TimestampContext,
    packet_id: u32,
    started: Instant,
}

struct PendingTxQueue {
    slots: [Option<PendingTx>; TX_PENDING],
    timeout: EmbassyDuration,
}

impl PendingTxQueue {
    fn new(timeout: EmbassyDuration) -> Self {
        Self {
            slots: Default::default(),
            timeout,
        }
    }

    fn push(&mut self, context: TimestampContext, packet_id: u32) {
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(PendingTx {
                context,
                packet_id,
                started: Instant::now(),
            });
        } else {
            warn!("ptp: tx timestamp queue full packet_id={=u32}", packet_id);
        }
    }

    fn take(&mut self, packet_id: u32) -> Option<TimestampContext> {
        self.slots
            .iter_mut()
            .find_map(|slot| slot.take_if(|pending| pending.packet_id == packet_id))
            .map(|pending| pending.context)
    }

    fn expire(&mut self) {
        for slot in self.slots.iter_mut() {
            if let Some(pending) = slot.take_if(|pending| pending.started.elapsed() >= self.timeout)
            {
                warn!(
                    "ptp: missing tx timestamp packet_id={=u32}",
                    pending.packet_id
                );
            }
        }
    }

    fn next_timeout_deadline(&self) -> Option<Instant> {
        self.slots
            .iter()
            .filter_map(|slot| slot.as_ref().map(|pending| pending.started + self.timeout))
            .min()
    }
}

struct PacketIdGenerator(NonZero<u32>);

impl PacketIdGenerator {
    const fn new() -> Self {
        Self(NonZero::<u32>::MIN)
    }

    fn next(&mut self) -> udp::PacketMeta {
        let id = self.0;
        self.0 = self.0.checked_add(1).unwrap_or(NonZero::<u32>::MIN);
        let mut meta = udp::PacketMeta::default();
        meta.id = id.get();
        meta.request_timestamp = true;
        meta
    }
}

fn clock_identity_from_mac(mac: [u8; 6]) -> ClockIdentity {
    // Use the IEEE EUI-64 expansion, not statime's zero-padded helper.
    ClockIdentity([mac[0], mac[1], mac[2], 0xff, 0xfe, mac[3], mac[4], mac[5]])
}

pub(crate) fn time_from(timestamp: Timestamp) -> Time {
    let nanos =
        u64::from(timestamp.seconds) * 1_000_000_000 + u64::from(timestamp.quarter_nanos >> 2);
    Time::from_nanos_subnanos(nanos, (timestamp.quarter_nanos & 3) << 30)
}

fn multicast_endpoint(port: u16, link_local: bool) -> IpEndpoint {
    let address = if link_local {
        LINK_LOCAL_MULTICAST
    } else {
        PRIMARY_MULTICAST
    };
    IpEndpoint::new(IpAddress::Ipv4(address), port)
}

fn deadline_from_now(duration: core::time::Duration) -> Instant {
    let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
    Instant::now() + EmbassyDuration::from_nanos(nanos)
}

fn state_name(state: PortState) -> &'static str {
    match state {
        PortState::Initializing => "initializing",
        PortState::Faulty => "faulty",
        PortState::Disabled => "disabled",
        PortState::Listening => "listening",
        PortState::PreMaster => "pre_master",
        PortState::Master => "master",
        PortState::Passive => "passive",
        PortState::Uncalibrated => "uncalibrated",
        PortState::Slave => "slave",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_logging() {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(defmt2log::init_from_current_exe);
    }

    #[test]
    fn preserves_quarter_nanoseconds() {
        init_logging();
        for (quarter_nanos, subnanos) in [(40, 0), (41, 1 << 30), (42, 1 << 31), (43, 3 << 30)] {
            assert_eq!(
                time_from(Timestamp {
                    seconds: 2,
                    quarter_nanos,
                }),
                Time::from_nanos_subnanos(2_000_000_010, subnanos),
            );
        }
    }

    #[test]
    fn accepts_delay_requests_only_for_a_master() {
        init_logging();
        let mut packet = [0; 34];
        packet[0] = MSG_DELAY_REQ;
        let timestamp = Timestamp::from_seconds_and_nanos(2, 10);
        let mut meta = udp::PacketMeta::default();
        meta.timestamp = Some(timestamp);

        assert_eq!(rx_event_timestamp(&packet, meta, false), None);
        assert_eq!(rx_event_timestamp(&packet, meta, true), Some(timestamp));
    }

    #[test]
    fn ignores_peer_delay_messages() {
        init_logging();
        let timestamp = Timestamp::from_seconds_and_nanos(2, 10);
        let mut meta = udp::PacketMeta::default();
        meta.timestamp = Some(timestamp);

        for message_type in [MSG_PDELAY_REQ, MSG_PDELAY_RESP] {
            let mut packet = [0; 34];
            packet[0] = message_type;
            assert_eq!(rx_event_timestamp(&packet, meta, true), None);
        }
    }
}
