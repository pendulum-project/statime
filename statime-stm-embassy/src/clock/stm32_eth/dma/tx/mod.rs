use super::PacketId;
use crate::clock::stm32_eth::peripherals::ETHERNET_DMA;

use super::{PacketIdNotFound, Timestamp};

mod descriptor;
pub use descriptor::{TxDescriptor, TxRingEntry};

#[cfg(any(feature = "ptp", feature = "async-await"))]
use core::task::Poll;

/// Errors that can occur during Ethernet TX
#[derive(Debug, PartialEq)]
pub enum TxError {
    /// Ring buffer is full
    WouldBlock,
}

/// Tx DMA state
pub struct TxRing<'a> {
    entries: &'a mut [TxRingEntry],
    next_entry: usize,
}

impl<'ring> TxRing<'ring> {
    /// Allocate
    ///
    /// `start()` will be needed before `send()`
    pub(crate) fn new(entries: &'ring mut [TxRingEntry]) -> Self {
        TxRing {
            entries,
            next_entry: 0,
        }
    }

    /// Start the Tx DMA engine
    pub(crate) fn start(&mut self, eth_dma: &ETHERNET_DMA) {
        // Setup ring
        {
            let mut previous: Option<&mut TxRingEntry> = None;
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

        let ring_ptr = self.entries[0].desc() as *const TxDescriptor;
        // Register TxDescriptor
        eth_dma
            .dmatdlar
            // Note: unsafe block required for `stm32f107`.
            .write(|w| unsafe { w.stl().bits(ring_ptr as u32) });

        // "Preceding reads and writes cannot be moved past subsequent writes."
        #[cfg(feature = "fence")]
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);

        // We don't need a compiler fence here because all interactions with `Descriptor` are
        // volatiles

        // Start transmission
        eth_dma.dmaomr.modify(|_, w| w.st().set_bit());
    }

    /// Stop the TX DMA
    pub(crate) fn stop(&self, eth_dma: &ETHERNET_DMA) {
        eth_dma.dmaomr.modify(|_, w| w.st().clear_bit());

        // DMA accesses do not stop before the running state
        // of the DMA has changed to something other than
        // running.
        while self.is_running() {}
    }

    /// If this returns `true`, the next `send` will succeed.
    pub fn next_entry_available(&self) -> bool {
        !self.entries[self.next_entry].is_available()
    }

    /// Check if we can send the next TX entry.
    ///
    /// If [`Ok(res)`] is returned, the caller of must ensure
    /// that [`self.entries[res].send()`](TxRingEntry::send) is called
    /// before a new invocation of `send_next_impl`.
    fn send_next_impl(&mut self) -> Result<usize, TxError> {
        let entries_len = self.entries.len();
        let entry_num = self.next_entry;
        let entry = &mut self.entries[entry_num];

        if entry.is_available() {
            self.next_entry = (self.next_entry + 1) % entries_len;
            Ok(entry_num)
        } else {
            Err(TxError::WouldBlock)
        }
    }

    /// Prepare a packet for sending.
    ///
    /// Write the data that you wish to send to the buffer
    /// represented by the returned [`TxPacket`] by using it
    /// as a slice.
    ///
    /// When all data is copied into the TX buffer, use [`TxPacket::send()`]
    /// to transmit it.
    pub fn send_next<'borrow>(
        &'borrow mut self,
        length: usize,
        packet_id: Option<PacketId>,
    ) -> Result<TxPacket<'borrow, 'ring>, TxError> {
        let entry = self.send_next_impl()?;
        let tx_buffer = self.entries[entry].buffer_mut();

        assert!(length <= tx_buffer.len(), "Not enough space in TX buffer");

        Ok(TxPacket {
            ring: self,
            idx: entry,
            length,
            packet_id,
        })
    }

    /// Prepare a packet for sending.
    ///
    /// Write the data that you wish to send to the buffer
    /// represented by the returned [`TxPacket`] by using it
    /// as a slice.
    ///
    /// When all data is copied into the TX buffer, use [`TxPacket::send()`]
    /// to transmit it.
    #[cfg(feature = "async-await")]
    pub async fn prepare_packet<'borrow>(
        &'borrow mut self,
        length: usize,
        packet_id: Option<PacketId>,
    ) -> TxPacket<'borrow, 'ring> {
        let entry = core::future::poll_fn(|ctx| match self.send_next_impl() {
            Ok(packet) => Poll::Ready(packet),
            Err(_) => {
                crate::dma::EthernetDMA::tx_waker().register(ctx.waker());
                Poll::Pending
            }
        })
        .await;

        let tx_buffer = self.entries[entry].buffer_mut();
        assert!(length <= tx_buffer.len(), "Not enough space in TX buffer");

        TxPacket {
            ring: self,
            idx: entry,
            length,
            packet_id,
        }
    }

    /// Demand that the DMA engine polls the current `TxDescriptor`
    /// (when we just transferred ownership to the hardware).
    pub(crate) fn demand_poll(&self) {
        // SAFETY: we only perform an atomic write to `dmatpdr`
        let eth_dma = unsafe { &*ETHERNET_DMA::ptr() };
        eth_dma.dmatpdr.write(|w| {
            #[cfg(any(feature = "stm32f4xx-hal", feature = "stm32f7xx-hal"))]
            {
                w.tpd().poll()
            }
            #[cfg(feature = "stm32f1xx-hal")]
            unsafe {
                // TODO: There is no nice `poll` method for `stm32f107`?
                w.tpd().bits(0)
            }
        });
    }

    /// Is the Tx DMA engine running?
    pub fn is_running(&self) -> bool {
        self.running_state().is_running()
    }

    pub(crate) fn running_state(&self) -> RunningState {
        // SAFETY: we only perform an atomic read of `dmasr`.
        let eth_dma = unsafe { &*ETHERNET_DMA::ptr() };

        match eth_dma.dmasr.read().tps().bits() {
            // Reset or Stop Transmit Command issued
            0b000 => RunningState::Stopped,
            // Fetching transmit transfer descriptor
            0b001 => RunningState::Running,
            // Waiting for status
            0b010 => RunningState::Running,
            // Reading Data from host memory buffer and queuing it to transmit buffer
            0b011 => RunningState::Running,
            0b100 | 0b101 => RunningState::Reserved,
            // Transmit descriptor unavailable
            0b110 => RunningState::Suspended,
            _ => RunningState::Unknown,
        }
    }
}

#[cfg(feature = "ptp")]
impl TxRing<'_> {
    fn entry_for_id(&self, id: &PacketId) -> Option<usize> {
        self.entries.iter().enumerate().find_map(
            |(idx, e)| {
                if e.has_packet_id(id) {
                    Some(idx)
                } else {
                    None
                }
            },
        )
    }

    fn entry_available(&self, index: usize) -> bool {
        self.entries[index].is_available()
    }

    fn entry_timestamp(&self, index: usize) -> Option<Timestamp> {
        self.entries[index].timestamp()
    }

    /// Blockingly wait untill the timestamp for the
    /// given ID is available.
    pub fn wait_for_timestamp(
        &self,
        packet_id: &PacketId,
    ) -> Result<Option<Timestamp>, PacketIdNotFound> {
        loop {
            if let Poll::Ready(res) = self.poll_timestamp(packet_id) {
                return res;
            }
        }
    }

    /// Poll to check if the timestamp for the given ID is already
    /// available.
    pub fn poll_timestamp(
        &self,
        packet_id: &PacketId,
    ) -> Poll<Result<Option<Timestamp>, PacketIdNotFound>> {
        let entry = if let Some(entry) = self.entry_for_id(packet_id) {
            entry
        } else {
            return Poll::Ready(Err(PacketIdNotFound));
        };

        if self.entry_available(entry) {
            Poll::Ready(Ok(self.entry_timestamp(entry)))
        } else {
            Poll::Pending
        }
    }

    /// Wait until the timestamp for the given ID is available.
    #[cfg(feature = "async-await")]
    pub async fn timestamp(
        &mut self,
        packet_id: &PacketId,
    ) -> Result<Option<Timestamp>, PacketIdNotFound> {
        core::future::poll_fn(move |ctx| {
            let res = self.poll_timestamp(packet_id);
            if res.is_pending() {
                crate::dma::EthernetDMA::tx_waker().register(ctx.waker());
            }
            res
        })
        .await
    }
}

#[derive(Debug, PartialEq)]
/// The run state of the TX DMA.
pub enum RunningState {
    /// Reset or Stop Transmit Command issued
    Stopped,
    /// Fetching transmit transfer descriptor;
    /// Waiting for status;
    /// Reading Data from host memory buffer and queuing it to transmit buffer
    Running,
    /// Reserved for future use
    Reserved,
    /// Transmit descriptor unavailable
    Suspended,
    /// Invalid value
    Unknown,
}

impl RunningState {
    /// Check whether this state represents that the
    /// TX DMA is running
    pub fn is_running(&self) -> bool {
        *self == RunningState::Running
    }
}

/// A struct that represents a soon-to-be-sent packet.
///
/// Implements [`Deref`] and [`DerefMut`] with `[u8]` as a target
/// so it can be used as a slice.
///
/// [`Deref`]: core::ops::Deref
/// [`DerefMut`]: core::ops::DerefMut
pub struct TxPacket<'borrow, 'ring> {
    ring: &'borrow mut TxRing<'ring>,
    idx: usize,
    length: usize,
    packet_id: Option<PacketId>,
}

impl core::ops::Deref for TxPacket<'_, '_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.ring.entries[self.idx].buffer()[..self.length]
    }
}

impl core::ops::DerefMut for TxPacket<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ring.entries[self.idx].buffer_mut()[..self.length]
    }
}

impl TxPacket<'_, '_> {
    /// Send this packet!
    pub fn send(self) {
        drop(self);
    }
}

impl Drop for TxPacket<'_, '_> {
    fn drop(&mut self) {
        self.ring.entries[self.idx].send(self.length, self.packet_id.clone());
        self.ring.demand_poll();
    }
}
