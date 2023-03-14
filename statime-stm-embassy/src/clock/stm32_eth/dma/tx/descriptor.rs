use crate::clock::stm32_eth::dma::{
    desc::Descriptor,
    ring::{RingDescriptor, RingEntry},
    PacketId,
};

use crate::clock::stm32_eth::ptp::Timestamp;

/// Owned by DMA engine
const TXDESC_0_OWN: u32 = 1 << 31;
/// Interrupt on completion
const TXDESC_0_IC: u32 = 1 << 30;
/// First segment of frame
const TXDESC_0_FS: u32 = 1 << 28;
/// Last segment of frame
const TXDESC_0_LS: u32 = 1 << 29;
/// Checksum insertion control
const TXDESC_0_CIC0: u32 = 1 << 23;
const TXDESC_0_CIC1: u32 = 1 << 22;
/// Timestamp this packet
const TXDESC_0_TIMESTAMP_ENABLE: u32 = 1 << 25;
/// This descriptor contains a timestamp
// NOTE(allow): packet_id is unused if ptp is disabled.
#[allow(dead_code)]
const TXDESC_0_TIMESTAMP_STATUS: u32 = 1 << 17;
/// Transmit end of ring
const TXDESC_0_TER: u32 = 1 << 21;
/// Second address chained
const TXDESC_0_TCH: u32 = 1 << 20;
/// Error status
const TXDESC_0_ES: u32 = 1 << 15;
/// TX done bit
const TXDESC_1_TBS_SHIFT: usize = 0;
const TXDESC_1_TBS_MASK: u32 = 0x0fff << TXDESC_1_TBS_SHIFT;

/// A TX DMA Ring Descriptor
#[repr(C)]
pub struct TxDescriptor {
    desc: Descriptor,
    packet_id: Option<PacketId>,
    buffer1: u32,
    next_descriptor: u32,
    is_last: bool,
}

impl Default for TxDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

impl TxDescriptor {
    /// Creates an zeroed TxDescriptor.
    pub const fn new() -> Self {
        Self {
            desc: Descriptor::new(),
            packet_id: None,
            buffer1: 0,
            next_descriptor: 0,
            is_last: false,
        }
    }

    #[allow(unused)]
    fn has_error(&self) -> bool {
        (self.desc.read(0) & TXDESC_0_ES) == TXDESC_0_ES
    }

    /// Is owned by the DMA engine?
    fn is_owned(&self) -> bool {
        (self.desc.read(0) & TXDESC_0_OWN) == TXDESC_0_OWN
    }

    // NOTE(allow): packet_id is unused if ptp is disabled.
    #[allow(dead_code)]
    fn is_last(&self) -> bool {
        self.desc.read(0) & TXDESC_0_LS == TXDESC_0_LS
    }

    /// Pass ownership to the DMA engine
    fn set_owned(&mut self, length: usize, packet_id: Option<PacketId>) {
        // Reconfigure packet ID
        self.packet_id = packet_id;

        self.set_buffer1_len(length);

        // These descriptor values are sometimes overwritten by
        // timestamp data, so we rewrite this data.
        let buffer1 = self.buffer1;
        unsafe {
            self.desc.write(2, buffer1);
        }

        let buffer2 = self.next_descriptor;
        unsafe {
            self.desc.write(3, buffer2);
        }

        // "Preceding reads and writes cannot be moved past subsequent writes."
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);

        let mut extra_flags = 0;

        if self.packet_id.is_some() {
            extra_flags |= TXDESC_0_TIMESTAMP_ENABLE;
        }

        if self.is_last {
            extra_flags |= TXDESC_0_TER;
        }

        unsafe {
            self.desc.write(
                0,
                TXDESC_0_OWN
                    | TXDESC_0_TCH
                    | TXDESC_0_FS
                    | TXDESC_0_LS
                    | TXDESC_0_CIC0
                    | TXDESC_0_CIC1
                    | TXDESC_0_IC
                    | extra_flags,
            )
        }

        // Used to flush the store buffer as fast as possible to make the buffer available for the
        // DMA.
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }

    fn set_buffer1_len(&mut self, len: usize) {
        unsafe {
            self.desc.modify(1, |w| {
                (w & !TXDESC_1_TBS_MASK) | ((len as u32) << TXDESC_1_TBS_SHIFT)
            });
        }
    }

    fn timestamp(&self) -> Option<Timestamp> {
        let tdes0 = self.desc.read(0);

        let contains_timestamp = (tdes0 & TXDESC_0_TIMESTAMP_STATUS) == TXDESC_0_TIMESTAMP_STATUS;

        if !self.is_owned() && contains_timestamp && self.is_last() {
            Timestamp::from_descriptor(&self.desc)
        } else {
            None
        }
    }
}

/// A TX DMA Ring Descriptor entry
pub type TxRingEntry = RingEntry<TxDescriptor>;

impl RingDescriptor for TxDescriptor {
    fn setup(&mut self, buffer: *const u8, _len: usize, next: Option<&Self>) {
        // Defer this initialization to this function, so we can have `RingEntry` on bss.
        let next_desc_addr = if let Some(next) = next {
            &next.desc as *const Descriptor as *const u8 as u32
        } else {
            self.is_last = true;
            0
        };

        self.buffer1 = buffer as u32;
        self.next_descriptor = next_desc_addr;
    }
}

impl TxRingEntry {
    pub(super) fn is_available(&self) -> bool {
        !self.desc().is_owned()
    }

    /// Only call this if [`TxRingEntry::is_available`]
    pub(super) fn send(&mut self, length: usize, packet_id: Option<PacketId>) {
        self.desc_mut().set_owned(length, packet_id);
    }

    /// Only call this if [`TxRingEntry::is_available`]
    pub fn buffer(&self) -> &[u8] {
        self.as_slice()
    }

    /// Only call this if [`TxRingEntry::is_available`]
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

impl TxRingEntry {
    pub fn has_packet_id(&self, packet_id: &PacketId) -> bool {
        self.desc().packet_id.as_ref() == Some(packet_id)
    }

    pub fn timestamp(&self) -> Option<Timestamp> {
        self.desc().timestamp().clone()
    }
}
