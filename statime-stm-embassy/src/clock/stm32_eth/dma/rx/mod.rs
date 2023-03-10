pub(crate) use self::descriptor::RxDescriptor;

use self::descriptor::RxDescriptorError;
pub use self::descriptor::RxRingEntry;

use super::PacketId;
use crate::clock::stm32_eth::peripherals::ETHERNET_DMA;

mod descriptor;

use crate::clock::stm32_eth::{dma::PacketIdNotFound, ptp::Timestamp};

use core::task::Poll;

/// Errors that can occur during RX
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq)]
pub enum RxError {
    /// The received packet was truncated
    Truncated,
    /// An error occured with the DMA
    DmaError,
    /// Receiving would block
    WouldBlock,
}

impl From<RxDescriptorError> for RxError {
    fn from(value: RxDescriptorError) -> Self {
        match value {
            RxDescriptorError::Truncated => Self::Truncated,
            RxDescriptorError::DmaError => Self::DmaError,
        }
    }
}

/// Rx DMA state
pub struct RxRing<'a> {
    entries: &'a mut [RxRingEntry],
    next_entry: usize,
}

impl<'a> RxRing<'a> {
    /// Allocate
    pub(crate) fn new(entries: &'a mut [RxRingEntry]) -> Self {
        RxRing {
            entries,
            next_entry: 0,
        }
    }

    /// Setup the DMA engine (**required**)
    pub(crate) fn start(&mut self, eth_dma: &ETHERNET_DMA) {
        // Setup ring
        {
            let mut previous: Option<&mut RxRingEntry> = None;
            for entry in self.entries.iter_mut() {
                if let Some(prev_entry) = &mut previous {
                    prev_entry.setup(Some(entry));
                }
                previous = Some(entry);
            }
            if let Some(entry) = &mut previous {
                entry.setup(None);
            }
        }
        self.next_entry = 0;
        let ring_ptr = self.entries[0].desc() as *const RxDescriptor;

        // Register RxDescriptor
        eth_dma
            .dmardlar
            .write(|w| unsafe { w.srl().bits(ring_ptr as u32) });

        // We already have fences in `set_owned`, which is called in `setup`

        // Start receive
        eth_dma.dmaomr.modify(|_, w| w.sr().set_bit());

        self.demand_poll();
    }

    /// Stop the RX DMA
    pub(crate) fn stop(&self, eth_dma: &ETHERNET_DMA) {
        eth_dma.dmaomr.modify(|_, w| w.sr().clear_bit());

        // DMA accesses do not stop before the running state
        // of the DMA has changed to something other than
        // running.
        while self.running_state().is_running() {}
    }

    /// Demand that the DMA engine polls the current `RxDescriptor`
    /// (when in [`RunningState::Stopped`].)
    fn demand_poll(&self) {
        // SAFETY: we only perform an atomic write to `dmarpdr`.
        let eth_dma = unsafe { &*ETHERNET_DMA::ptr() };
        eth_dma.dmarpdr.write(|w| unsafe { w.rpd().bits(1) });
    }

    /// Get current `RunningState`
    pub fn running_state(&self) -> RunningState {
        // SAFETY: we only perform an atomic read of `dmasr`.
        let eth_dma = unsafe { &*ETHERNET_DMA::ptr() };
        match eth_dma.dmasr.read().rps().bits() {
            //  Reset or Stop Receive Command issued
            0b000 => RunningState::Stopped,
            //  Fetching receive transfer descriptor
            0b001 => RunningState::Running,
            //  Waiting for receive packet
            0b011 => RunningState::Running,
            //  Receive descriptor unavailable
            0b100 => RunningState::Stopped,
            //  Closing receive descriptor
            0b101 => RunningState::Running,
            //  Transferring the receive packet data from receive buffer to host memory
            0b111 => RunningState::Running,
            _ => RunningState::Unknown,
        }
    }

    /// Check if we can receive a new packet
    pub fn next_entry_available(&self) -> bool {
        if !self.running_state().is_running() {
            self.demand_poll();
        }

        self.entries[self.next_entry].is_available()
    }

    /// Receive the next packet (if any is ready).
    ///
    /// This function returns a tuple of `Ok((entry_index, length))` on
    /// success. Whoever receives the `Ok` must ensure that `set_owned`
    /// is eventually called on the entry with that index.
    fn recv_next_impl(
        &mut self,
        // NOTE(allow): packet_id is unused if ptp is disabled.
        #[allow(unused_variables)] packet_id: Option<PacketId>,
    ) -> Result<(usize, usize), RxError> {
        if !self.running_state().is_running() {
            self.demand_poll();
        }

        let entries_len = self.entries.len();
        let entry_num = self.next_entry;
        let entry = &mut self.entries[entry_num];

        if entry.is_available() {
            let length = entry.recv(packet_id)?;

            self.next_entry = (self.next_entry + 1) % entries_len;

            Ok((entry_num, length))
        } else {
            Err(RxError::WouldBlock)
        }
    }

    /// Receive the next packet (if any is ready), or return [`Err`]
    /// immediately.
    pub fn recv_next(&mut self, packet_id: Option<PacketId>) -> Result<RxPacket, RxError> {
        let (entry, length) = self.recv_next_impl(packet_id.map(|p| p.into()))?;
        Ok(RxPacket {
            entry: &mut self.entries[entry],
            length,
        })
    }

    /// Receive the next packet.
    ///
    /// The returned [`RxPacket`] can be used as a slice, and
    /// will contain the ethernet data.
    pub async fn recv(&mut self, packet_id: Option<PacketId>) -> RxPacket {
        let (entry, length) = core::future::poll_fn(|ctx| {
            let res = self.recv_next_impl(packet_id.clone());

            match res {
                Ok(value) => Poll::Ready(value),
                Err(_) => {
                    crate::clock::stm32_eth::dma::EthernetDMA::rx_waker().register(ctx.waker());
                    Poll::Pending
                }
            }
        })
        .await;

        RxPacket {
            entry: &mut self.entries[entry],
            length,
        }
    }
}

impl<'a> RxRing<'a> {
    /// Get the timestamp for a specific ID
    pub fn timestamp(&self, id: &PacketId) -> Result<Option<Timestamp>, PacketIdNotFound> {
        let entry = self.entries.iter().find(|e| e.has_packet_id(id));

        let entry = entry.ok_or(PacketIdNotFound)?;

        Ok(entry.read_timestamp())
    }
}

/// Running state of the `RxRing`
#[derive(PartialEq, Eq, Debug)]
pub enum RunningState {
    /// Running state is unknown.
    Unknown,
    /// The RX DMA is stopped.
    Stopped,
    /// The RX DMA is running.
    Running,
}

impl RunningState {
    /// whether self equals to `RunningState::Running`
    pub fn is_running(&self) -> bool {
        *self == RunningState::Running
    }
}

/// A received packet.
///
/// This packet implements [Deref<\[u8\]>](core::ops::Deref) and should be used
/// as a slice.
pub struct RxPacket<'a> {
    entry: &'a mut RxRingEntry,
    length: usize,
}

impl<'a> core::ops::Deref for RxPacket<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.entry.as_slice()[0..self.length]
    }
}

impl<'a> core::ops::DerefMut for RxPacket<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entry.as_mut_slice()[0..self.length]
    }
}

impl<'a> Drop for RxPacket<'a> {
    fn drop(&mut self) {
        self.entry.desc_mut().set_owned();
    }
}

impl<'a> RxPacket<'a> {
    /// Pass the received packet back to the DMA engine.
    pub fn free(self) {
        drop(self)
    }

    /// Get the timestamp associated with this packet
    pub fn timestamp(&self) -> Option<Timestamp> {
        self.entry.read_timestamp()
    }
}
