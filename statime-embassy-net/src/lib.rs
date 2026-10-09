#![cfg_attr(not(test), no_std)]
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod embassy_clock;
#[cfg(feature = "monitor")]
mod monitor;

use embassy_net::{
    driver::{Timestamp, TxTimestamp},
    iface::{Iface, MulticastError},
    udp,
    wire::Ipv4Addr,
};
use embassy_time::{Duration as EmbassyDuration, Instant};
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
    port::{NoForwardedTLVs, PortActionIterator},
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

mod transport;
use transport::{Incoming, PacketIdGenerator, PortIo, StatimeTimer};

const TX_TIMESTAMP_TIMEOUT: EmbassyDuration = EmbassyDuration::from_millis(100);

/// Error encountered while starting a [`Runner`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum RunError {
    /// The stack has no free UDP socket slot.
    Socket {
        /// PTP port requiring a socket.
        port: u16,
    },
    /// The network stack could not join a required PTP multicast group.
    Multicast {
        /// Multicast address the runner tried to join.
        address: Ipv4Addr,
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
            Self::Socket { port } => {
                write!(formatter, "no UDP socket available for PTP port {port}")
            }
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
            _ => None,
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
    /// Also bounds waiting for send capacity; protocol deadlines may shorten it.
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
    iface: Iface<'a>,
    clock: C,
    config: Config,
    filter_config: F::Config,
    packet_id: PacketIdGenerator,
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
    /// Construct the service for an interface, its PTP clock, and protocol/filter configuration.
    /// Socket binding and multicast setup happen in [`run`](Self::run).
    pub fn new(iface: Iface<'a>, clock: C, config: Config, filter_config: F::Config) -> Self {
        Self {
            iface,
            clock,
            config,
            filter_config,
            packet_id: PacketIdGenerator::new(),
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
    /// Run the Embassy network runner concurrently. This service must be the
    /// stack's only TX timestamp requester and consumer. PTP sockets remain
    /// bound to the selected interface and its clock.
    ///
    /// Transient link or IP-configuration loss does not return an error. The
    /// runner retains its sockets and protocol state and resumes when the
    /// network stack becomes usable again.
    /// Reuse this runner after cancellation to preserve packet IDs. Replacing it
    /// requires that no TX timestamps from the old runner can arrive.
    pub async fn run(&mut self) -> Result<(), RunError> {
        let iface = self.iface;
        let clock = ClockRef {
            clock: &mut self.clock,
            #[cfg(feature = "monitor")]
            monitor: self.monitor,
        };
        let config = self.config;
        let filter_config = self.filter_config.clone();

        info!("ptp: waiting for network configuration");
        iface.wait_config_v4_up().await;
        let mut io = PortIo::new(iface, config.tx_timestamp_timeout)?;

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
            bmca,
            &mut self.packet_id,
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
                    bmca,
                    &mut self.packet_id,
                    #[cfg(feature = "monitor")]
                    self.monitor,
                )
                .await;
                continue;
            }

            io.pending_tx.expire();
            // Keep the owned packet alive only through this action batch.
            let mut incoming = Incoming::None;
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
                incoming = io.receive(receive_delay_requests);
                match &incoming {
                    Incoming::Event(packet, timestamp) => {
                        port.handle_event_receive(packet.payload(), time_from(*timestamp))
                    }
                    Incoming::General(packet) => port.handle_general_receive(packet.payload()),
                    Incoming::None => PortActionIterator::empty(),
                }
            };
            io.handle(
                actions,
                bmca,
                &mut self.packet_id,
                #[cfg(feature = "monitor")]
                self.monitor,
            )
            .await;

            drop(incoming);
            let next = io.next_deadline(bmca);
            tx_timestamp = io.wait(next).await;
        }
    }
}

fn clock_identity_from_mac(mac: [u8; 6]) -> ClockIdentity {
    // Use the IEEE EUI-64 expansion, not statime's zero-padded helper.
    ClockIdentity([mac[0], mac[1], mac[2], 0xff, 0xfe, mac[3], mac[4], mac[5]])
}

pub(crate) fn time_from(timestamp: Timestamp) -> Time {
    time_from_parts(timestamp.seconds, timestamp.quarter_nanos)
}

// Packet and adjustable-clock timestamps deliberately belong to independent
// Embassy driver crates. Convert their shared representation only here.
pub(crate) fn time_from_parts(seconds: u32, quarter_nanos: u32) -> Time {
    let nanos = u64::from(seconds) * 1_000_000_000 + u64::from(quarter_nanos >> 2);
    Time::from_nanos_subnanos(nanos, (quarter_nanos & 3) << 30)
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

    pub(crate) fn init_logging() {
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
}
