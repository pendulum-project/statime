use rtic_monotonics::{systick::Systick, Monotonic};
use smoltcp::{
    iface::{Interface, SocketSet, SocketStorage},
    socket::udp,
};
use stm32_eth::dma::{EthernetDMA, RxRingEntry, TxRingEntry};

pub struct NetworkResources {
    pub rx_ring: [RxRingEntry; 2],
    pub tx_ring: [TxRingEntry; 2],
    pub rx_meta_storage: [udp::PacketMetadata; 8],
    pub rx_payload_storage: [u8; 8192],
    pub tx_meta_storage: [udp::PacketMetadata; 8],
    pub tx_payload_storage: [u8; 8192],
    pub sockets: [SocketStorage<'static>; 8],
}

impl NetworkResources {
    pub const fn new() -> Self {
        Self {
            rx_ring: [RxRingEntry::new(), RxRingEntry::new()],
            tx_ring: [TxRingEntry::new(), TxRingEntry::new()],
            rx_meta_storage: [udp::PacketMetadata::EMPTY; 8],
            rx_payload_storage: [0; 8192],
            tx_meta_storage: [udp::PacketMetadata::EMPTY; 8],
            tx_payload_storage: [0; 8192],
            sockets: [SocketStorage::EMPTY; 8],
        }
    }
}

pub struct NetworkStack {
    pub dma: EthernetDMA<'static, 'static>,
    pub iface: Interface,
    pub sockets: SocketSet<'static>,
}

impl NetworkStack {
    pub fn poll(&mut self) {
        self.iface
            .poll(now(), &mut &mut self.dma, &mut self.sockets);
    }

    pub fn poll_delay(&mut self) -> Option<smoltcp::time::Duration> {
        self.iface.poll_delay(now(), &self.sockets)
    }
}

pub const CLIENT_ADDR: [u8; 6] = [0x80, 0x00, 0xde, 0xad, 0xbe, 0xef];

fn now() -> smoltcp::time::Instant {
    let now_millis = Systick::now().ticks();
    // TODO handle case where systick is not 1kHz
    smoltcp::time::Instant::from_millis(now_millis)
}
