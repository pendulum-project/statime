use statime::network::{NetworkPacket, NetworkPort, NetworkRuntime};
use statime::time::Instant;

#[derive(Clone, Debug)]
pub struct Stm32Runtime;

impl Stm32Runtime {
    pub fn new() -> Self {
        Stm32Runtime
    }
}

impl NetworkRuntime for Stm32Runtime {
    type InterfaceDescriptor = Stm32InterfaceDescriptor;
    type NetworkPort = Stm32NetworkPort;
    type Error = Stm32NetworkError;

    async fn open(
        &mut self,
        interface: Self::InterfaceDescriptor,
    ) -> Result<Self::NetworkPort, Self::Error> {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct Stm32InterfaceDescriptor;

#[derive(Clone, Debug)]
pub struct Stm32NetworkPort;

impl NetworkPort for Stm32NetworkPort {
    type Error = ();

    async fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        todo!()
    }

    async fn send_time_critical(&mut self, data: &[u8]) -> Result<Instant, Self::Error> {
        todo!()
    }

    async fn recv(&mut self) -> Result<NetworkPacket, Self::Error> {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub enum Stm32NetworkError {}
