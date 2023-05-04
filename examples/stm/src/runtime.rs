use embassy_stm32::eth::{Instance, PHY};
use smoltcp::{
    iface::{Config, Interface},
    socket::{udp, udp::PacketBuffer},
    wire::{EthernetAddress, IpEndpoint},
};
use statime::{
    clock::Clock,
    network::{NetworkPacket, NetworkPort, NetworkRuntime},
    time::Instant,
};

use crate::{device::StmDevice, StmClock};

pub struct StmRuntime<'dd, 'dc, T: Instance, P: PHY> {
    device: StmDevice<'dd, T, P>,
    clock: StmClock<'dc>,
}

impl<'dd, 'dc, T: Instance, P: PHY> StmRuntime<'dd, 'dc, T, P> {
    pub fn new(device: StmDevice<'dd, T, P>, clock: StmClock<'dc>) -> Self {
        StmRuntime { device, clock }
    }
}

impl<'dd, 'dc, T: Instance, P: PHY> NetworkRuntime for StmRuntime<'dd, 'dc, T, P> {
    type InterfaceDescriptor = StmInterfaceDescriptor;
    type NetworkPort = StmPort<'dd, 'dc>;
    type Error = StmError;

    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<StmPort<'dd, 'dc>, StmError> {
        let mut config = Config::new();
        config.hardware_addr = Some(EthernetAddress(self.device.mac_addr()).into());

        let mut iface = Interface::new(config, &mut self.device);
        iface.update_ip_addrs(|ip_addrs| {
            // ip_addrs.push(IpCidr::new(...));
        });

        let payload_storage = self.device.rx().buffers as *const _ as &mut [u8];
        let rx_buffer = PacketBuffer::new(&mut self.device.rx(), payload_storage);

        let payload_storage = self.device.tx().buffers as *const _ as &mut [u8];
        let tx_buffer = PacketBuffer::new(&mut self.device.tx(), payload_storage);

        let mut tc_socket = udp::Socket::new(rx_buffer, tx_buffer);
        let tc_remote_endpoint = interface.remote_endpoint();
        tc_socket.bind(tc_remote_endpoint)?;

        let rx_buffer = PacketBuffer::new(&mut self.device.rx(), &mut self.device.rx());
        let tx_buffer = PacketBuffer::new(&mut self.device.tx(), &mut self.device.tx());
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

pub struct StmPort<'dd, 'dc> {
    tc_socket: udp::Socket<'dd>,
    ntc_socket: udp::Socket<'dd>,
    tc_remote_endpoint: IpEndpoint,
    ntc_remote_endpoint: IpEndpoint,
    clock: StmClock<'dc>,
}

impl<'a, 'd> NetworkPort for StmPort<'a, 'd> {
    type Error = StmError;

    async fn send(&mut self, data: &[u8]) -> Result<(), StmError> {
        Ok(self.ntc_socket.send_slice(data, self.ntc_remote_endpoint)?)
    }

    async fn send_time_critical(&mut self, data: &[u8]) -> Result<Instant, StmError> {
        self.tc_socket.send_slice(data, self.tc_remote_endpoint)?;
        Ok(self.clock.now())
    }

    async fn recv(&mut self) -> Result<NetworkPacket, StmError> {
        todo!()
    }
}

#[derive(Debug)]
enum StmError {
    Bind(udp::BindError),
    Send(udp::SendError),
}

impl From<udp::BindError> for StmError {
    fn from(value: udp::BindError) -> Self {
        StmError::Bind(value)
    }
}

impl From<udp::SendError> for StmError {
    fn from(value: udp::SendError) -> Self {
        StmError::Send(value)
    }
}
