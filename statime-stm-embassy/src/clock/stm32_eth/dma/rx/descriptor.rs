use crate::clock::stm32_eth::dma::{
    desc::Descriptor,
    ring::{RingDescriptor, RingEntry},
};

use crate::clock::stm32_eth::dma::PacketId;

use crate::clock::stm32_eth::ptp::Timestamp;

/// Errors that can occur during RX
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, PartialEq)]
pub(crate) enum RxDescriptorError {
    /// The received packet was truncated
    Truncated,
    /// An error occured with the DMA
    DmaError,
}

/// RX timestamp valid
/// NOTE(allow): unused if not(feature = "ptp")
#[allow(unused)]
const RXDESC_0_TIMESTAMP_VALID: u32 = 1 << 7;
/// Owned by DMA engine
const RXDESC_0_OWN: u32 = 1 << 31;
/// First descriptor
const RXDESC_0_FS: u32 = 1 << 9;
/// Last descriptor
const RXDESC_0_LS: u32 = 1 << 8;
/// Error summary
const RXDESC_0_ES: u32 = 1 << 15;
/// Frame length
const RXDESC_0_FL_MASK: u32 = 0x3FFF;
const RXDESC_0_FL_SHIFT: usize = 16;

const RXDESC_1_RBS_SHIFT: usize = 0;
const RXDESC_1_RBS_MASK: u32 = 0x0fff << RXDESC_1_RBS_SHIFT;
/// Second address chained
const RXDESC_1_RCH: u32 = 1 << 14;
/// End Of Ring
const RXDESC_1_RER: u32 = 1 << 15;

#[repr(C)]
/// An RX DMA Descriptor
pub struct RxDescriptor {
    desc: Descriptor,
    buffer1: Option<u32>,
    next_descriptor: Option<u32>,
    packet_id: Option<PacketId>,
    cached_timestamp: Option<Timestamp>,
}

impl Default for RxDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

impl RxDescriptor {
    /// Creates an zeroed RxDescriptor.
    pub const fn new() -> Self {
        Self {
            desc: Descriptor::new(),
            buffer1: None,
            next_descriptor: None,
            packet_id: None,
            cached_timestamp: None,
        }
    }

    /// Is owned by the DMA engine?
    fn is_owned(&self) -> bool {
        (self.desc.read(0) & RXDESC_0_OWN) == RXDESC_0_OWN
    }

    /// Pass ownership to the DMA engine
    ///
    /// Overrides old timestamp data
    pub fn set_owned(&mut self) {
        self.write_buffer1();
        self.write_buffer2();

        // "Preceding reads and writes cannot be moved past subsequent writes."
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);

        unsafe {
            self.desc.write(0, RXDESC_0_OWN);
        }

        // Used to flush the store buffer as fast as possible to make the buffer available for the
        // DMA.
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    fn has_error(&self) -> bool {
        (self.desc.read(0) & RXDESC_0_ES) == RXDESC_0_ES
    }

    /// Descriptor contains first buffer of frame
    fn is_first(&self) -> bool {
        (self.desc.read(0) & RXDESC_0_FS) == RXDESC_0_FS
    }

    /// Descriptor contains last buffers of frame
    fn is_last(&self) -> bool {
        (self.desc.read(0) & RXDESC_0_LS) == RXDESC_0_LS
    }

    /// Get PTP timestamps if available
    pub fn timestamp(&self) -> Option<Timestamp> {
        let is_valid = { self.desc.read(0) & RXDESC_0_TIMESTAMP_VALID == RXDESC_0_TIMESTAMP_VALID };

        let timestamp = Timestamp::from_descriptor(&self.desc);

        if is_valid && self.is_last() {
            timestamp
        } else {
            None
        }
    }

    /// Rewrite buffer1 to the last value we wrote to it
    ///
    /// In our case, the address of the data buffer for this descriptor
    ///
    /// This only has to be done on stm32f107. For f4 and f7, enhanced descriptors
    /// must be enabled for timestamping support, which we enable by default.
    fn write_buffer1(&mut self) {
        let buffer_addr = self
            .buffer1
            .expect("Writing buffer1 of an RX descriptor, but `buffer_address` is None");

        unsafe {
            self.desc.write(2, buffer_addr);
        }
    }

    fn set_buffer1(&mut self, buffer: *const u8, len: usize) {
        self.buffer1 = Some(buffer as u32);
        self.write_buffer1();
        unsafe {
            self.desc.modify(1, |w| {
                (w & !RXDESC_1_RBS_MASK) | ((len as u32) << RXDESC_1_RBS_SHIFT)
            });
        }
    }

    /// Rewrite buffer2 to the last value we wrote it to
    ///
    /// In our case, the address of the next descriptor (may be zero)
    ///
    /// This only has to be done on stm32f107. For f4 and f7, enhanced descriptors
    /// must be enabled for timestamping support, which we enable by default.
    fn write_buffer2(&mut self) {
        let addr = self
            .next_descriptor
            .expect("Writing buffer2 of an RX descriptor, but `next_descriptor` is None");

        unsafe {
            self.desc.write(3, addr);
        }
    }

    // points to next descriptor (RCH)
    fn set_buffer2(&mut self, buffer: *const u8) {
        self.next_descriptor = Some(buffer as u32);
        self.write_buffer2();
    }

    fn set_end_of_ring(&mut self) {
        unsafe {
            self.desc.modify(1, |w| w | RXDESC_1_RER);
        }
    }

    fn get_frame_len(&self) -> usize {
        ((self.desc.read(0) >> RXDESC_0_FL_SHIFT) & RXDESC_0_FL_MASK) as usize
    }
}

/// An RX DMA Ring Descriptor entry
pub type RxRingEntry = RingEntry<RxDescriptor>;

impl RingDescriptor for RxDescriptor {
    fn setup(&mut self, buffer: *const u8, len: usize, next: Option<&Self>) {
        // Defer this initialization to this function, so we can have `RingEntry` on bss.
        unsafe {
            self.desc.write(1, RXDESC_1_RCH);
        }
        self.set_buffer1(buffer, len);
        match next {
            Some(next) => self.set_buffer2(&next.desc as *const Descriptor as *const u8),
            None => {
                #[allow(clippy::zero_ptr)]
                self.set_buffer2(0 as *const u8);
                self.set_end_of_ring();
            }
        };
        self.set_owned();
    }
}

impl RxRingEntry {
    /// The initial value for an Rx Ring Entry
    pub const RX_INIT: Self = Self::new();

    pub(super) fn is_available(&self) -> bool {
        !self.desc().is_owned()
    }

    /// Only call this if [`RxRingEntry::is_available`]
    pub(super) fn recv(&mut self, packet_id: Option<PacketId>) -> Result<usize, RxDescriptorError> {
        if self.desc().has_error() {
            self.desc_mut().set_owned();
            Err(RxDescriptorError::DmaError)
        } else if self.desc().is_first() && self.desc().is_last() {
            let frame_len = self.desc().get_frame_len();

            // "Subsequent reads and writes cannot be moved ahead of preceding reads."
            core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Acquire);

            {
                // Cache the PTP timestamp
                self.desc_mut().cached_timestamp = self.desc().timestamp();
            }

            // Set the Packet ID for this descriptor.
            self.desc_mut().packet_id = packet_id;

            Ok(frame_len)
        } else {
            self.desc_mut().set_owned();
            Err(RxDescriptorError::Truncated)
        }
    }
}

impl RxRingEntry {
    pub fn has_packet_id(&self, id: &PacketId) -> bool {
        Some(id) == self.desc().packet_id.as_ref()
    }

    pub fn read_timestamp(&self) -> Option<Timestamp> {
        self.desc().cached_timestamp.clone()
    }
}
