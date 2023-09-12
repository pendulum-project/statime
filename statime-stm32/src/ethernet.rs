use smoltcp::{iface::SocketStorage, socket::udp};
use stm32_eth::dma::{RxRingEntry, TxRingEntry};

pub struct NetworkResources {
    pub rx_ring: [RxRingEntry; 2],
    pub tx_ring: [TxRingEntry; 2],
    pub rx_meta_storage: [udp::PacketMetadata; 8],
    pub rx_payload_storage: [u8; 1024],
    pub tx_meta_storage: [udp::PacketMetadata; 8],
    pub tx_payload_storage: [u8; 1024],
    pub sockets: [SocketStorage<'static>; 8],
}

impl NetworkResources {
    pub const fn new() -> Self {
        Self {
            rx_ring: [RxRingEntry::new(), RxRingEntry::new()],
            tx_ring: [TxRingEntry::new(), TxRingEntry::new()],
            rx_meta_storage: [udp::PacketMetadata::EMPTY; 8],
            rx_payload_storage: [0; 1024],
            tx_meta_storage: [udp::PacketMetadata::EMPTY; 8],
            tx_payload_storage: [0; 1024],
            sockets: [SocketStorage::EMPTY; 8],
        }
    }
}
