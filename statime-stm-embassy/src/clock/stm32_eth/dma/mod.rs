//! Ethernet DMA access and configuration.

use cortex_m::peripheral::NVIC;

use crate::clock::stm32_eth::{peripherals::ETHERNET_DMA, stm32::Interrupt};

#[cfg(feature = "async-await")]
use futures::task::AtomicWaker;

#[cfg(any(feature = "ptp", feature = "async-await"))]
use core::task::Poll;

pub(crate) mod desc;

pub(crate) mod ring;

mod rx;
pub use rx::{RunningState as RxRunningState, RxError, RxPacket, RxRing, RxRingEntry};

mod tx;
pub use tx::{RunningState as TxRunningState, TxError, TxPacket, TxRing, TxRingEntry};

#[cfg(feature = "ptp")]
use crate::ptp::Timestamp;

mod packet_id;
pub use packet_id::PacketId;

/// From the datasheet: *VLAN Frame maxsize = 1522*
pub(crate) const MTU: usize = 1522;

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Clone, Copy, Debug, PartialEq)]
/// This struct is returned if a packet ID is not associated
/// with any TX or RX descriptors.
pub struct PacketIdNotFound;

/// Ethernet DMA.
pub struct EthernetDMA<'rx, 'tx> {
    pub(crate) eth_dma: ETHERNET_DMA,
    pub(crate) rx_ring: RxRing<'rx>,
    pub(crate) tx_ring: TxRing<'tx>,
}

impl<'rx, 'tx> EthernetDMA<'rx, 'tx> {
    /// Create and initialise the ethernet DMA
    ///
    /// # Note
    /// - Make sure that the buffers reside in a memory region that is
    /// accessible by the peripheral. Core-Coupled Memory (CCM) is
    /// usually not accessible.
    pub(crate) fn new(
        eth_dma: ETHERNET_DMA,
        rx_buffer: &'rx mut [RxRingEntry],
        tx_buffer: &'tx mut [TxRingEntry],
    ) -> Self {
        // reset DMA bus mode register
        eth_dma.dmabmr.modify(|_, w| w.sr().set_bit());

        // Wait until done
        while eth_dma.dmabmr.read().sr().bit_is_set() {}

        // operation mode register
        eth_dma.dmaomr.modify(|_, w| {
            // Dropping of TCP/IP checksum error frames disable
            w.dtcefd()
                .set_bit()
                // Receive store and forward
                .rsf()
                .set_bit()
                // Disable flushing of received frames
                .dfrf()
                .set_bit()
                // Transmit store and forward
                .tsf()
                .set_bit()
                // Forward error frames
                .fef()
                .set_bit()
                // Operate on second frame
                .osf()
                .set_bit()
        });

        // bus mode register
        eth_dma.dmabmr.modify(|_, w| {
            // For any non-f107 chips, we must use enhanced descriptor format to support checksum
            // offloading and/or timestamps.
            #[cfg(not(feature = "stm32f1xx-hal"))]
            let w = w.edfe().set_bit();

            unsafe {
                // Address-aligned beats
                w.aab()
                    .set_bit()
                    // Fixed burst
                    .fb()
                    .set_bit()
                    // Rx DMA PBL
                    .rdp()
                    .bits(32)
                    // Programmable burst length
                    .pbl()
                    .bits(32)
                    // Rx Tx priority ratio 2:1
                    .pm()
                    .bits(0b01)
                    // Use separate PBL
                    .usp()
                    .set_bit()
            }
        });

        let mut dma = EthernetDMA {
            eth_dma,
            rx_ring: RxRing::new(rx_buffer),
            tx_ring: TxRing::new(tx_buffer),
        };

        dma.rx_ring.start(&dma.eth_dma);
        dma.tx_ring.start(&dma.eth_dma);

        dma
    }

    /// Split the [`EthernetDMA`] into concurrently operating send and
    /// receive parts.
    pub fn split(&mut self) -> (&mut RxRing<'rx>, &mut TxRing<'tx>) {
        (&mut self.rx_ring, &mut self.tx_ring)
    }

    /// Enable RX and TX interrupts
    ///
    /// In your handler you must call
    /// [`EthernetDMA::interrupt_handler()`] or [`stm32_eth::eth_interrupt_handler`](crate::eth_interrupt_handler)
    /// to clear interrupt pending bits. Otherwise the interrupt will reoccur immediately.
    ///
    /// [`EthernetPTP::interrupt_handler()`]: crate::ptp::EthernetPTP::interrupt_handler
    #[cfg_attr(
        feature = "ptp",
        doc = "If you have PTP enabled, you must also call [`EthernetPTP::interrupt_handler()`] if you wish to make use of the PTP timestamp trigger feature."
    )]
    pub fn enable_interrupt(&self) {
        self.eth_dma.dmaier.modify(|_, w| {
            w
                // Normal interrupt summary enable
                .nise()
                .set_bit()
                // Receive Interrupt Enable
                .rie()
                .set_bit()
                // Transmit Interrupt Enable
                .tie()
                .set_bit()
        });

        // Enable ethernet interrupts
        unsafe {
            NVIC::unmask(Interrupt::ETH);
        }
    }

    /// Handle the DMA parts of the `ETH` interrupt.
    pub fn interrupt_handler() -> InterruptReasonSummary {
        // SAFETY: we only perform atomic reads/writes through `eth_dma`.
        let eth_dma = unsafe { &*ETHERNET_DMA::ptr() };

        let status = eth_dma.dmasr.read();

        let status = InterruptReasonSummary {
            is_rx: status.rs().bit_is_set(),
            is_tx: status.ts().bit_is_set(),
            is_error: status.ais().bit_is_set(),
        };

        eth_dma
            .dmasr
            .write(|w| w.nis().set_bit().ts().set_bit().rs().set_bit());

        #[cfg(feature = "async-await")]
        {
            if status.is_tx {
                EthernetDMA::tx_waker().wake();
            }

            if status.is_rx {
                EthernetDMA::rx_waker().wake();
            }
        }

        status
    }

    /// Try to receive a packet.
    ///
    /// If no packet is available, this function returns [`Err(RxError::WouldBlock)`](RxError::WouldBlock).
    ///
    /// It may also return another kind of [`RxError`].
    pub fn recv_next(&mut self, packet_id: Option<PacketId>) -> Result<RxPacket, RxError> {
        self.rx_ring.recv_next(packet_id.map(Into::into))
    }

    /// Is Rx DMA currently running?
    ///
    /// It stops if the ring is full. Call [`EthernetDMA::recv_next()`] to free an
    /// entry and to demand poll from the hardware.
    pub fn rx_is_running(&self) -> bool {
        self.rx_ring.running_state().is_running()
    }

    /// Is Tx DMA currently running?
    pub fn tx_is_running(&self) -> bool {
        self.tx_ring.is_running()
    }

    /// Try to send a packet with data.
    ///
    /// If there are no free TX slots, this function will
    /// return [`Err(TxError::WouldBlock)`](TxError::WouldBlock).
    pub fn send<F>(
        &mut self,
        length: usize,
        packet_id: Option<PacketId>,
        f: F,
    ) -> Result<(), TxError>
    where
        F: FnOnce(&mut [u8]),
    {
        let mut tx_packet = self.tx_ring.send_next(length, packet_id)?;
        f(&mut tx_packet);
        tx_packet.send();
        Ok(())
    }

    /// Check if there is a packet available for reading.
    ///
    /// If this function returns true, it is guaranteed that the
    /// next call to [`EthernetDMA::recv_next`] will return [`Ok`].
    pub fn rx_available(&mut self) -> bool {
        self.rx_ring.next_entry_available()
    }

    /// Check if sending a packet now would succeed.
    ///
    /// If this function returns true, it is guaranteed that
    /// the next call to [`EthernetDMA::send`] will return [`Ok`]
    pub fn tx_available(&mut self) -> bool {
        self.tx_ring.next_entry_available()
    }
}

impl Drop for EthernetDMA<'_, '_> {
    // On drop, stop all DMA actions.
    fn drop(&mut self) {
        self.tx_ring.stop(&self.eth_dma);

        self.rx_ring.stop(&self.eth_dma);
    }
}

#[cfg(feature = "async-await")]
impl<'rx, 'tx> EthernetDMA<'rx, 'tx> {
    pub(crate) fn rx_waker() -> &'static AtomicWaker {
        static WAKER: AtomicWaker = AtomicWaker::new();
        &WAKER
    }

    pub(crate) fn tx_waker() -> &'static AtomicWaker {
        static WAKER: AtomicWaker = AtomicWaker::new();
        &WAKER
    }

    /// Receive a packet.
    ///
    /// See [`RxRing::recv`].
    pub async fn recv(&mut self, packet_id: Option<PacketId>) -> RxPacket {
        self.rx_ring.recv(packet_id).await
    }

    /// Prepare a packet for sending.
    ///
    /// See [`TxRing::prepare_packet`].
    pub async fn prepare_packet<'borrow>(
        &'borrow mut self,
        length: usize,
        packet_id: Option<PacketId>,
    ) -> TxPacket<'borrow, 'tx> {
        self.tx_ring.prepare_packet(length, packet_id).await
    }

    /// Wait for an RX or TX interrupt to have
    /// occured.
    pub async fn rx_or_tx(&mut self) {
        let mut polled_once = false;
        core::future::poll_fn(|ctx| {
            if polled_once {
                Poll::Ready(())
            } else {
                polled_once = true;
                EthernetDMA::rx_waker().register(ctx.waker());
                EthernetDMA::tx_waker().register(ctx.waker());
                Poll::Pending
            }
        })
        .await;
    }
}

#[cfg(feature = "ptp")]
impl EthernetDMA<'_, '_> {
    /// Try to get the timestamp for the given packet ID.
    ///
    /// This function will attempt to find both RX and TX timestamps,
    /// so make sure that the provided packet ID is unique between the two.
    pub fn poll_timestamp(
        &self,
        packet_id: &PacketId,
    ) -> Poll<Result<Option<Timestamp>, PacketIdNotFound>> {
        // Check if it's a TX packet
        let tx = self.poll_tx_timestamp(packet_id);

        if tx != Poll::Ready(Err(PacketIdNotFound)) {
            return tx;
        }

        // It's not a TX packet, check if it's an RX packet
        Poll::Ready(self.rx_timestamp(packet_id))
    }

    /// Get the RX timestamp for the given packet ID.
    pub fn rx_timestamp(
        &self,
        packet_id: &PacketId,
    ) -> Result<Option<Timestamp>, PacketIdNotFound> {
        self.rx_ring.timestamp(packet_id)
    }

    /// Blockingly wait until the TX timestamp for
    /// the given ID is available.
    pub fn wait_for_tx_timestamp(
        &self,
        packet_id: &PacketId,
    ) -> Result<Option<Timestamp>, PacketIdNotFound> {
        self.tx_ring.wait_for_timestamp(packet_id)
    }

    /// Poll to check if the TX timestamp for the given
    /// ID is available.
    pub fn poll_tx_timestamp(
        &self,
        packet_id: &PacketId,
    ) -> Poll<Result<Option<Timestamp>, PacketIdNotFound>> {
        self.tx_ring.poll_timestamp(packet_id)
    }

    /// Get the TX timestamp for the given ID.
    #[cfg(feature = "async-await")]
    pub async fn tx_timestamp(
        &mut self,
        packet_id: &PacketId,
    ) -> Result<Option<Timestamp>, PacketIdNotFound> {
        self.tx_ring.timestamp(packet_id).await
    }
}

/// A summary of the reasons for the interrupt
/// that occured
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy)]
pub struct InterruptReasonSummary {
    /// The interrupt was caused by an RX event.
    pub is_rx: bool,
    /// The interrupt was caused by an TX event.
    pub is_tx: bool,
    /// The interrupt was caused by an error event.
    pub is_error: bool,
}
