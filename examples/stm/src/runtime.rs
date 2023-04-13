use embassy_stm32::eth::{Instance, PHY};
use smoltcp::{
    iface::{Config, Interface},
    socket::{udp, udp::PacketBuffer},
    wire::{EthernetAddress, IpEndpoint},
};
use smoltcp::iface::{SocketSet, SocketStorage};
use smoltcp::wire::IpCidr;
use statime::{
    network::{NetworkPacket, NetworkPort, NetworkRuntime},
    time::Instant,
};

use crate::{eth::Ethernet, StmClock};

pub struct StmRuntime<'dc, 'dd, T: Instance, P: PHY> {
    clock: StmClock<'dc>,
    device: Ethernet<'dd, T, P>,
}

impl<'dc, 'dd, T: Instance, P: PHY> StmRuntime<'dc, 'dd, T, P> {
    pub fn new(clock: StmClock<'dc>, device: Ethernet<'dd, T, P>) -> Self {
        StmRuntime { clock, device }
    }
}

impl<'dc, 'dd, T: Instance, P: PHY> NetworkRuntime for StmRuntime<'dc, 'dd, T, P> {
    type InterfaceDescriptor = StmInterfaceDescriptor;
    type NetworkPort = StmPort<'dc, 'dd>;
    type Error = StmError;

    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<Self::NetworkPort, Self::Error> {
        let mut config = Config::new();
        config.hardware_addr = Some(EthernetAddress(self.device.mac_addr).into());

        let mut iface = Interface::new(config, &mut self.device);
        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs.push(IpCidr::new())
        })

        let rx_buffer = PacketBuffer::new(&mut self.device.rx, &mut self.device.rx);
        let tx_buffer = PacketBuffer::new(&mut self.device.tx, &mut self.device.tx);
        let mut tc_socket = udp::Socket::new(rx_buffer, tx_buffer);
        let tc_remote_endpoint = interface.remote_endpoint();
        tc_socket.bind(tc_remote_endpoint)?;

        let rx_buffer = PacketBuffer::new(&mut self.device.rx, &mut self.device.rx);
        let tx_buffer = PacketBuffer::new(&mut self.device.tx, &mut self.device.tx);
        let mut ntc_socket = udp::Socket::new(rx_buffer, tx_buffer);
        let ntc_remote_endpoint = interface.remote_endpoint();
        ntc_socket.bind(ntc_remote_endpoint)?;

        // let mut sockets = SocketSet::new(&mut [SocketStorage::EMPTY; 2]);
        // let tc_handle = sockets.add(tc_socket);
        // let ntc_handle = sockets.add(ntc_socket);
        // let tc_socket = sockets.get_mut::<udp::Socket>(tc_handle);
        // let ntc_socket = sockets.get_mut::<udp::Socket>(ntc_handle);

        Ok(StmPort {
            tc_socket,
            ntc_socket,
            tc_remote_endpoint,
            ntc_remote_endpoint,
            clock: self.clock,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StmInterfaceDescriptor {}

impl StmInterfaceDescriptor {
    fn remote_endpoint(&self) -> IpEndpoint {
        todo!()
    }
}

#[derive(Debug)]
pub struct StmPort<'a, 'd> {
    tc_socket: udp::Socket<'a>,
    ntc_socket: udp::Socket<'a>,
    tc_remote_endpoint: IpEndpoint,
    ntc_remote_endpoint: IpEndpoint,
    clock: StmClock<'d>,
}

impl<'a, 'd> NetworkPort for StmPort<'a, 'd> {
    type Error = StmError;

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        Ok(self.ntc_socket.send_slice(data, self.ntc_remote_endpoint)?)
    }

    async fn send_time_critical(&mut self, data: &[u8]) -> Result<Instant, Self::Error> {
        todo!()
    }

    async fn recv(&mut self) -> Result<NetworkPacket, Self::Error> {
        todo!()
    }
}

#[derive(Debug)]
enum StmError {
    Send(udp::SendError),
}

impl From<udp::SendError> for StmError {
    fn from(value: udp::SendError) -> Self {
        StmError::Send(value)
    }
}
