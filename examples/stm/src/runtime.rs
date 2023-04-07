use crate::StmClock;
use smoltcp::socket::udp;
use smoltcp::socket::udp::PacketBuffer;
use smoltcp::wire::IpEndpoint;
use statime::network::{NetworkPacket, NetworkPort, NetworkRuntime};
use statime::time::Instant;

#[derive(Debug)]
pub struct StmRuntime<'a, 'd> {
    rx_buffer: &'a mut [u8],
    tx_buffer: &'a mut [u8],
    clock: StmClock<'d>,
}

impl<'a, 'd> StmRuntime<'a, 'd> {
    pub fn new(rx_buffer: &'a mut [u8], tx_buffer: &'a mut [u8], clock: StmClock<'d>) -> Self {
        StmRuntime {
            rx_buffer,
            tx_buffer,
            clock,
        }
    }
}

impl<'a, 'd> NetworkRuntime for StmRuntime<'a, 'd> {
    type InterfaceDescriptor = StmInterfaceDescriptor;
    type NetworkPort = StmPort<'a, 'd>;
    type Error = StmError;

    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<Self::NetworkPort, Self::Error> {
        let rx_buffer = PacketBuffer::new();
        let tx_buffer = PacketBuffer::new();
        let mut tc_socket = udp::Socket::new(rx_buffer, tx_buffer);

        let rx_buffer = PacketBuffer::new();
        let tx_buffer = PacketBuffer::new();
        let mut ntc_socket = udp::Socket::new(rx_buffer, tx_buffer);

        let tc_remote_endpoint = interface.remote_endpoint();
        let ntc_remote_endpoint = interface.remote_endpoint();

        tc_socket.bind(tc_remote_endpoint)?;
        ntc_socket.bind(ntc_remote_endpoint)?;

        Ok(StmPort {
            tc_socket,
            ntc_socket,
            tc_remote_endpoint,
            ntc_remote_endpoint,
            clock: self.clock,
        })
    }
}

#[derive(Debug)]
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

impl NetworkPort for StmPort {
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
