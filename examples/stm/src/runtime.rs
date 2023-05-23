use core::mem;

use embassy_stm32::eth::{Instance, PHY};
use smoltcp::{
    iface::{Config, Interface, SocketSet},
    socket::udp,
    storage::PacketBuffer,
    wire::{EthernetAddress, IpCidr, IpEndpoint},
};
use statime::{
    network::{NetworkPacket, NetworkPort, NetworkRuntime},
    time::Instant,
};

use crate::{device::StmDevice, StmClock};

const MAX_PORTS: usize = 4;

pub struct StmRuntime<'d, T: Instance, P: PHY> {
    device: StmDevice<'d, T, P>,
    tx_buffer: &'d mut [u8],
    rx_buffer: &'d mut [u8],
    clock: &'d StmClock<'d>,
    open_ports: usize,
}

impl<'d, T: Instance, P: PHY> StmRuntime<'d, T, P> {
    pub fn new(
        device: StmDevice<'d, T, P>,
        tx_buffer: &'d mut [u8],
        rx_buffer: &'d mut [u8],
        clock: &'d StmClock<'d>,
    ) -> Self {
        StmRuntime {
            device,
            tx_buffer,
            rx_buffer,
            clock,
            open_ports: 0,
        }
    }
}

impl<'d, T: Instance, P: PHY> NetworkRuntime for StmRuntime<'d, T, P> {
    type InterfaceDescriptor = StmInterfaceDescriptor;
    type NetworkPort<'a> = StmPort<'a, T, P>;
    type Error = StmError;

    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<StmPort<'_, T, P>, StmError> {
        if self.open_ports == MAX_PORTS {
            return Err(StmError::Unavailable);
        }

        let mut config = Config::new();
        config.hardware_addr = Some(EthernetAddress(self.device.mac_addr()).into());
        let mut iface = Interface::new(config, &mut self.device);
        iface.update_ip_addrs(|ip_addrs| {
            ip_addrs
                .push(IpCidr::new(interface.remote_endpoint.addr, 0))
                .unwrap();
        });
        match interface.remote_endpoint.addr {
            Address::Ipv4(addr) => iface.routes_mut().add_default_ipv4_route(addr)?,
            _ => unreachable!(),
        }

        let (rx_metadata, rx_payload) =
            self.rx_buffer.split_at_mut(self.device.rx_buffer.len() / 2);
        let len = rx_metadata.len() / mem::size_of::<udp::PacketMetadata>();
        let metadata_storage =
            unsafe { core::slice::from_raw_parts_mut(rx_metadata.as_mut_ptr().cast(), len) };
        let payload_storage = &mut rx_payload[..];
        let rx_buffer = PacketBuffer::new(metadata_storage, payload_storage);

        let (tx_metadata, tx_payload) =
            self.tx_buffer.split_at_mut(self.device.tx_buffer.len() / 2);
        let len = tx_metadata.len() / mem::size_of::<udp::PacketMetadata>();
        let metadata_storage =
            unsafe { core::slice::from_raw_parts_mut(tx_metadata.as_mut_ptr().cast(), len) };
        let payload_storage = &mut tx_payload[..];
        let tx_buffer = PacketBuffer::new(metadata_storage, payload_storage);

        let mut socket = udp::Socket::new(rx_buffer, tx_buffer);
        let remote_endpoint = interface.remote_endpoint();
        socket.bind(remote_endpoint)?;

        self.open_ports += 1;

        Ok(StmPort {
            socket,
            device: &mut self.device,
            iface,
            remote_endpoint,
            clock: self.clock,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StmInterfaceDescriptor {
    remote_endpoint: IpEndpoint,
}

impl StmInterfaceDescriptor {
    pub fn new(remote_endpoint: IpEndpoint) -> Self {
        StmInterfaceDescriptor { remote_endpoint }
    }

    fn remote_endpoint(&self) -> IpEndpoint {
        self.remote_endpoint
    }
}

pub struct StmPort<'d, T: Instance, P: PHY> {
    socket: udp::Socket<'d>,
    device: &'d mut StmDevice<T, P>,
    iface: Interface,
    remote_endpoint: IpEndpoint,
    clock: &'d StmClock<'d>,
}

impl<T: Instance, P: PHY> NetworkPort for StmPort<'_, T, P> {
    type Error = StmError;

    async fn send(&mut self, data: &[u8]) -> Result<(), StmError> {
        self.socket.send_slice(data, self.remote_endpoint)?;
        Ok(())
    }

    async fn send_time_critical(&mut self, data: &[u8]) -> Result<Option<Instant>, StmError> {
        self.socket.send_slice(data, self.remote_endpoint)?;

        let mut sockets = SocketSet::new(&mut [self.socket]);
        loop {
            if self.iface.poll((), &mut self.device, &mut sockets) {
                break;
            }

            if let Some(delay) = self.iface.poll_delay() {
                let delay = embassy_time::Duration::from_micros(delay.micros());
                embassy_time::block_for(delay).await;
            }
        }

        Ok(())
    }

    async fn recv(&mut self) -> Result<NetworkPacket, StmError> {
        todo!()
    }
}

#[derive(Debug)]
pub enum StmError {
    Bind(udp::BindError),
    Send(udp::SendError),
    /// No more ports available.
    Unavailable,
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
