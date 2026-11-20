use core::num::NonZero;
use embassy_futures::select::{Either3, select3};
use embassy_net::{
    TryError,
    driver::{Timestamp, TxTimestamp},
    iface::Iface,
    udp::{self, UdpMetadata, UdpSocket},
    wire::{Ipv4Addr, SocketAddr},
};
use embassy_time::{Duration as EmbassyDuration, Instant, with_deadline};
use statime::port::{PortAction, PortActionIterator, TimestampContext};

#[cfg(feature = "monitor")]
use crate::PtpMonitor;
use crate::{RunError, deadline_from_now};

const EVENT_PORT: u16 = 319;
const GENERAL_PORT: u16 = 320;
const PRIMARY_MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 1, 129);
const LINK_LOCAL_MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 107);
const TX_PENDING: usize = 4;
const MAX_PACKET_LEN: usize = 256;
const MSG_DELAY_REQ: u8 = 0x1;
const MSG_PDELAY_REQ: u8 = 0x2;
const MSG_PDELAY_RESP: u8 = 0x3;

pub(super) struct PortIo<'a> {
    iface: Iface<'a>,
    event: UdpSocket<'a>,
    general: UdpSocket<'a>,
    joined: [bool; 2],
    pub(super) timers: Timers,
    pub(super) pending_tx: PendingTxQueue,
}

impl<'a> PortIo<'a> {
    pub(super) fn new(
        iface: Iface<'a>,
        tx_timestamp_timeout: EmbassyDuration,
    ) -> Result<Self, RunError> {
        let mut event =
            UdpSocket::new(iface.stack()).map_err(|_| RunError::Socket { port: EVENT_PORT })?;
        let mut general =
            UdpSocket::new(iface.stack()).map_err(|_| RunError::Socket { port: GENERAL_PORT })?;
        event
            .bind_to_iface(Some(iface.handle()))
            .map_err(|source| RunError::Bind {
                port: EVENT_PORT,
                source,
            })?;
        general
            .bind_to_iface(Some(iface.handle()))
            .map_err(|source| RunError::Bind {
                port: GENERAL_PORT,
                source,
            })?;
        event.bind(EVENT_PORT, 0).map_err(|source| RunError::Bind {
            port: EVENT_PORT,
            source,
        })?;
        general
            .bind(GENERAL_PORT, 0)
            .map_err(|source| RunError::Bind {
                port: GENERAL_PORT,
                source,
            })?;
        let mut io = Self {
            iface,
            event,
            general,
            joined: [false; 2],
            timers: Timers::default(),
            pending_tx: PendingTxQueue::new(tx_timestamp_timeout),
        };
        for (index, address) in [PRIMARY_MULTICAST, LINK_LOCAL_MULTICAST]
            .into_iter()
            .enumerate()
        {
            if !iface.has_multicast_group(address) {
                iface
                    .join_multicast_group(address)
                    .map_err(|source| RunError::Multicast { address, source })?;
                io.joined[index] = true;
            }
        }
        Ok(io)
    }

    pub(super) async fn handle(
        &mut self,
        actions: PortActionIterator<'_>,
        deadline: Instant,
        packet_id: &mut PacketIdGenerator,
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
                        remote_addr: multicast_endpoint(EVENT_PORT, link_local),
                        meta: packet_id.next(),
                        local_addr: None,
                    };
                    if !self.pending_tx.push(context, metadata.meta.id) {
                        continue;
                    }
                    let deadline = self.next_deadline(deadline);
                    match with_deadline(deadline, self.event.send_to(data, metadata)).await {
                        Ok(Ok(())) => self.pending_tx.sent(metadata.meta.id),
                        _ => {
                            self.pending_tx.take(metadata.meta.id);
                            warn!("ptp: event send failed or timed out");
                        }
                    }
                }
                PortAction::SendGeneral { data, link_local } => {
                    let metadata = UdpMetadata {
                        remote_addr: multicast_endpoint(GENERAL_PORT, link_local),
                        meta: udp::PacketMeta::default(),
                        local_addr: None,
                    };
                    let deadline = self
                        .next_deadline(deadline)
                        .min(Instant::now() + self.pending_tx.timeout);
                    if !matches!(
                        with_deadline(deadline, self.general.send_to(data, metadata)).await,
                        Ok(Ok(()))
                    ) {
                        warn!("ptp: general send failed or timed out");
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

    pub(super) fn receive(&self, receive_delay_requests: bool) -> Incoming {
        match self.event.try_recv() {
            Ok(packet) => {
                if packet.payload().len() > MAX_PACKET_LEN {
                    warn!("ptp: oversized event packet");
                    return Incoming::None;
                }
                return rx_event_timestamp(
                    packet.payload(),
                    packet.meta().meta,
                    receive_delay_requests,
                )
                .map_or(Incoming::None, |timestamp| {
                    Incoming::Event(packet, timestamp)
                });
            }
            Err(TryError::Other(error)) => {
                warn!("ptp: event receive failed: {}", error);
                return Incoming::None;
            }
            Err(TryError::WouldBlock) => {}
        }

        match self.general.try_recv() {
            Ok(packet)
                if packet.payload().len() <= MAX_PACKET_LEN
                    && ptp_message_type(packet.payload()).is_some() =>
            {
                Incoming::General(packet)
            }
            Ok(_) | Err(TryError::WouldBlock) => Incoming::None,
            Err(TryError::Other(error)) => {
                warn!("ptp: general receive failed: {}", error);
                Incoming::None
            }
        }
    }

    pub(super) fn next_deadline(&self, deadline: Instant) -> Instant {
        [
            self.timers.next_deadline(),
            self.pending_tx.next_timeout_deadline(),
        ]
        .into_iter()
        .flatten()
        .fold(deadline, Ord::min)
    }

    pub(super) async fn wait(&self, deadline: Instant) -> Option<TxTimestamp> {
        let stack = self.iface.stack();
        match with_deadline(
            deadline,
            select3(
                stack.tx_timestamp(),
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

impl Drop for PortIo<'_> {
    fn drop(&mut self) {
        for (joined, address) in self
            .joined
            .into_iter()
            .zip([PRIMARY_MULTICAST, LINK_LOCAL_MULTICAST])
        {
            if joined {
                let _ = self.iface.leave_multicast_group(address);
            }
        }
    }
}

pub(super) enum Incoming {
    Event(udp::RecvPacket, Timestamp),
    General(udp::RecvPacket),
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
pub(super) struct Timers([Option<Instant>; StatimeTimer::ALL.len()]);

impl Timers {
    fn reset(&mut self, timer: StatimeTimer, duration: core::time::Duration) {
        self.0[timer as usize] = Some(deadline_from_now(duration));
    }

    pub(super) fn take_due(&mut self) -> Option<StatimeTimer> {
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
pub(super) enum StatimeTimer {
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

pub(super) struct PendingTxQueue {
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

    fn push(&mut self, context: TimestampContext, packet_id: u32) -> bool {
        if let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(PendingTx {
                context,
                packet_id,
                started: Instant::now(),
            });
            true
        } else {
            warn!("ptp: tx timestamp queue full packet_id={=u32}", packet_id);
            false
        }
    }

    fn sent(&mut self, packet_id: u32) {
        if let Some(pending) = self
            .slots
            .iter_mut()
            .flatten()
            .find(|pending| pending.packet_id == packet_id)
        {
            pending.started = Instant::now();
        }
    }

    pub(super) fn take(&mut self, packet_id: u32) -> Option<TimestampContext> {
        self.slots
            .iter_mut()
            .find_map(|slot| slot.take_if(|pending| pending.packet_id == packet_id))
            .map(|pending| pending.context)
    }

    pub(super) fn expire(&mut self) {
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

pub(super) struct PacketIdGenerator(NonZero<u32>);

impl PacketIdGenerator {
    pub(super) const fn new() -> Self {
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

fn multicast_endpoint(port: u16, link_local: bool) -> SocketAddr {
    let address = if link_local {
        LINK_LOCAL_MULTICAST
    } else {
        PRIMARY_MULTICAST
    };
    SocketAddr::new(address.into(), port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::init_logging;

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
