use defmt::unwrap;
use rtic::Mutex;
use rtic_sync::channel::Sender;
use smoltcp::{
    iface::SocketHandle,
    socket::udp::{self, UdpMetadata},
    wire::{IpAddress, IpEndpoint},
};
use statime::{InBmca, PortAction, PortActionIterator, Running, TimestampContext};
use stm32_eth::dma::PacketId;

use crate::{ethernet::NetworkStack, StmPort, TimerName};

pub struct Port {
    timer_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
    packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
    time_critical_socket: SocketHandle,
    general_socket: SocketHandle,
    state: PortState,
}

impl Port {
    pub fn new(
        timer_sender: Sender<'static, (TimerName, core::time::Duration), 4>,
        packet_id_sender: Sender<'static, (statime::TimestampContext, PacketId), 16>,
        time_critical_socket: SocketHandle,
        general_socket: SocketHandle,
        state: StmPort<InBmca<'static>>,
    ) -> Self {
        Self {
            timer_sender,
            packet_id_sender,
            time_critical_socket,
            general_socket,
            state: PortState::InBmca(state),
        }
    }

    pub fn handle_timecritical_receive(
        &mut self,
        data: &[u8],
        timestamp: statime::Time,
        net: &mut impl Mutex<T = NetworkStack>,
    ) {
        let mut running_port_state = self.state.take_running();
        let actions = running_port_state.handle_timecritical_receive(data, timestamp);
        self.handle_port_actions(actions, net);
        self.state.set_running(running_port_state);
    }

    pub fn handle_general_receive(&mut self, data: &[u8], net: &mut impl Mutex<T = NetworkStack>) {
        let mut running_port_state = self.state.take_running();
        let actions = running_port_state.handle_general_receive(data);
        self.handle_port_actions(actions, net);
        self.state.set_running(running_port_state);
    }

    pub fn handle_timer(&mut self, timer: TimerName, net: &mut impl Mutex<T = NetworkStack>) {
        let mut running_port_state = self.state.take_running();
        let actions = match timer {
            TimerName::Announce => running_port_state.handle_announce_timer(),
            TimerName::Sync => running_port_state.handle_sync_timer(),
            TimerName::DelayRequest => running_port_state.handle_delay_request_timer(),
            TimerName::AnnounceReceipt => running_port_state.handle_announce_receipt_timer(),
            TimerName::FilterUpdate => running_port_state.handle_filter_update_timer(),
        };
        self.handle_port_actions(actions, net);
        self.state.set_running(running_port_state);
    }

    pub fn handle_send_timestamp(
        &mut self,
        context: TimestampContext,
        timestamp: statime::Time,
        net: &mut impl Mutex<T = NetworkStack>,
    ) {
        let mut running_port_state = self.state.take_running();
        let actions = running_port_state.handle_send_timestamp(context, timestamp);
        self.handle_port_actions(actions, net);
        self.state.set_running(running_port_state);
    }

    pub fn perform_bmca(
        &mut self,
        f: impl FnOnce(&mut StmPort<InBmca<'static>>),
        net: &mut impl Mutex<T = NetworkStack>,
    ) {
        let bmca_state = self.state.make_bmca_mode();
        f(bmca_state);
        let actions = self.state.make_running();
        self.handle_port_actions(actions, net);
    }

    fn handle_port_actions(
        &mut self,
        actions: statime::PortActionIterator<'_>,
        net: &mut impl Mutex<T = NetworkStack>,
    ) {
        // In this function it's likely the case that self.state is in the empty state
        // due to ownership rules. So don't use that field.

        for action in actions {
            match action {
                PortAction::SendTimeCritical { context, data } => {
                    const TO: IpEndpoint = IpEndpoint {
                        addr: IpAddress::v4(224, 0, 1, 129),
                        port: 319,
                    };
                    match send(net, self.time_critical_socket, &TO, data) {
                        Ok(pid) => unwrap!(self.packet_id_sender.try_send((context, pid)).ok()),
                        Err(e) => {
                            defmt::error!("Failed to send time critical packet, because: {}", e)
                        }
                    }
                }
                PortAction::SendGeneral { data } => {
                    const TO: IpEndpoint = IpEndpoint {
                        addr: IpAddress::v4(224, 0, 1, 129),
                        port: 320,
                    };
                    match send(net, self.general_socket, &TO, data) {
                        Ok(_) => (),
                        Err(e) => defmt::error!("Failed to send general packet, because: {}", e),
                    }
                }
                PortAction::ResetAnnounceTimer { duration } => {
                    unwrap!(self
                        .timer_sender
                        .try_send((TimerName::Announce, duration))
                        .ok());
                }
                PortAction::ResetSyncTimer { duration } => {
                    unwrap!(self.timer_sender.try_send((TimerName::Sync, duration)).ok());
                }
                PortAction::ResetDelayRequestTimer { duration } => {
                    unwrap!(self
                        .timer_sender
                        .try_send((TimerName::DelayRequest, duration))
                        .ok());
                }
                PortAction::ResetAnnounceReceiptTimer { duration } => {
                    unwrap!(self
                        .timer_sender
                        .try_send((TimerName::AnnounceReceipt, duration))
                        .ok());
                }
                PortAction::ResetFilterUpdateTimer { duration } => {
                    unwrap!(self
                        .timer_sender
                        .try_send((TimerName::FilterUpdate, duration))
                        .ok());
                }
            }
        }
    }

    pub fn time_critical_socket(&self) -> SocketHandle {
        self.time_critical_socket
    }

    pub fn general_socket(&self) -> SocketHandle {
        self.general_socket
    }
}

enum PortState {
    None,
    Running(StmPort<Running<'static>>),
    InBmca(StmPort<InBmca<'static>>),
}

impl PortState {
    /// Change to state to the [PortState::InBmca] mode and return a reference
    /// to it.
    fn make_bmca_mode(&mut self) -> &mut StmPort<InBmca<'static>> {
        *self = match core::mem::replace(self, PortState::None) {
            PortState::Running(port) => PortState::InBmca(port.start_bmca()),
            val => val,
        };

        match self {
            PortState::InBmca(port) => port,
            _ => defmt::unreachable!(),
        }
    }

    /// Change to state to the [PortState::Running] and return the associated
    /// port actions
    fn make_running(&mut self) -> PortActionIterator<'static> {
        let (this, actions) = match core::mem::replace(self, PortState::None) {
            PortState::InBmca(port) => {
                let (port, actions) = port.end_bmca();
                (PortState::Running(port), actions)
            }
            _ => defmt::panic!("Port in bad state"),
        };

        *self = this;

        actions
    }

    /// Get the running port and leave behind an empty port. Panics if the port
    /// is not currently in running mode.
    fn take_running(&mut self) -> StmPort<Running<'static>> {
        match core::mem::replace(self, PortState::None) {
            Self::Running(v) => v,
            _ => defmt::panic!("Port is not in running mode"),
        }
    }

    /// Set the port to running after a previous [Self::take_running].
    fn set_running(&mut self, port: StmPort<Running<'static>>) {
        match self {
            PortState::None => *self = Self::Running(port),
            _ => defmt::panic!("Port not in empty state"),
        }
    }
}

fn send(
    net: &mut impl Mutex<T = NetworkStack>,
    socket: SocketHandle,
    to: &smoltcp::wire::IpEndpoint,
    data: &[u8],
) -> Result<PacketId, udp::SendError> {
    net.lock(|net| {
        // Get an Id to track our packet
        let packet_id = net.dma.next_packet_id();

        // Combine the receiver with the packet id
        let mut meta: UdpMetadata = (*to).into();
        meta.meta = packet_id.clone().into();

        // Actually send the packet
        net.sockets
            .get_mut::<udp::Socket>(socket)
            .send_slice(data, meta)?;

        // Send out the packet, this makes sure the MAC actually sees the packet and
        // knows about the packet_id
        net.poll();

        Ok(packet_id)
    })
}
