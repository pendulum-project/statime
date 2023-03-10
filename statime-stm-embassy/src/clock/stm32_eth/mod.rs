//! An abstraction layer for ethernet periperhals embedded in STM32 processors.
//!
//! For initialisation, see [`new`], and [`new_with_mii`]

use embassy_stm32::rcc::Clocks;

use ptp::EthernetPTP;
#[doc(inline)]
pub use setup::{EthPins, Parts, PartsIn};
use {
    dma::{EthernetDMA, RxRingEntry, TxRingEntry},
    mac::{EthernetMAC, EthernetMACWithMii, MdcPin, MdioPin, Speed, WrongClock},
    setup::*,
};

pub mod dma;
pub mod mac;
pub mod setup;

pub(crate) mod peripherals;
pub mod ptp;

/// A summary of the reasons for the occurence of an interrupt
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterruptReason {
    /// A packet has arrived and is ready for processing.
    pub rx: bool,
    /// A packet was sent, and a TX slot has freed up.
    pub tx: bool,
    /// A DMA error occured.
    pub dma_error: bool,
    /// The target time configured for PTP has passed.
    pub time_passed: bool,
}

/// Handle the `ETH` interrupt.
///
/// This function wakes wakers and resets
/// interrupt bits relevant in that interrupt.
pub fn eth_interrupt_handler() -> InterruptReason {
    let dma = EthernetDMA::interrupt_handler();

    let is_time_trigger = EthernetPTP::interrupt_handler();

    InterruptReason {
        rx: dma.is_rx,
        tx: dma.is_tx,
        dma_error: dma.is_error,
        time_passed: is_time_trigger,
    }
}

/// Create and initialise the ethernet driver.
///
/// Initialize and start tx and rx DMA engines.
/// Sets up the peripheral clocks and GPIO configuration,
/// and configures the ETH MAC and DMA peripherals.
/// Automatically sets slew rate to VeryHigh.
///
/// The speed of the MAC is set to [`Speed::FullDuplexBase100Tx`].
/// This can be changed using [`EthernetMAC::set_speed`].
///
/// This method does not initialise the external PHY. Interacting with a PHY
/// can be done by using the struct returned from [`EthernetMAC::mii`].
///
/// # Note
/// - Make sure that the buffers reside in a memory region that is
/// accessible by the peripheral. Core-Coupled Memory (CCM) is
/// usually not accessible.
/// - HCLK must be at least 25 MHz.
pub fn new<'rx, 'tx, REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1>(
    parts: PartsIn,
    rx_buffer: &'rx mut [RxRingEntry],
    tx_buffer: &'tx mut [TxRingEntry],
    clocks: Clocks,
    pins: EthPins<REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1>,
) -> Result<Parts<'rx, 'tx, EthernetMAC>, WrongClock>
where
    REFCLK: RmiiRefClk + AlternateVeryHighSpeed,
    CRS: RmiiCrsDv + AlternateVeryHighSpeed,
    TXEN: RmiiTxEN + AlternateVeryHighSpeed,
    TXD0: RmiiTxD0 + AlternateVeryHighSpeed,
    TXD1: RmiiTxD1 + AlternateVeryHighSpeed,
    RXD0: RmiiRxD0 + AlternateVeryHighSpeed,
    RXD1: RmiiRxD1 + AlternateVeryHighSpeed,
{
    // Configure all of the pins correctly
    pins.setup_pins();

    // Set up the clocks and reset the MAC periperhal
    setup::setup();

    let eth_mac = parts.mac.into();

    // Congfigure and start up the ethernet DMA.
    let dma = EthernetDMA::new(parts.dma.into(), rx_buffer, tx_buffer);

    // Configure the ethernet PTP
    let ptp = EthernetPTP::new(parts.ptp.into(), clocks, &dma);

    // Configure the ethernet MAC
    let mac = EthernetMAC::new(eth_mac, parts.mmc, clocks, Speed::FullDuplexBase100Tx, &dma)?;

    let parts = Parts { mac, dma, ptp };

    Ok(parts)
}

/// Create and initialise the ethernet driver.
///
/// Initialize and start tx and rx DMA engines.
/// Sets up the peripheral clocks and GPIO configuration,
/// and configures the ETH MAC and DMA peripherals.
/// Automatically sets slew rate to VeryHigh.
///
/// This method does not initialise the external PHY.
///
/// The speed of the MAC is set to [`Speed::FullDuplexBase100Tx`].
/// This can be changed using [`EthernetMAC::set_speed`].
///
/// The MII for the external PHY can be accessed through the
/// returned [`EthernetMACWithMii`], .
///
/// # Note
/// - Make sure that the buffers reside in a memory region that is
/// accessible by the peripheral. Core-Coupled Memory (CCM) is
/// usually not accessible.
/// - HCLK must be at least 25 MHz.
pub fn new_with_mii<'rx, 'tx, REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1, MDIO, MDC>(
    parts: PartsIn,
    rx_buffer: &'rx mut [RxRingEntry],
    tx_buffer: &'tx mut [TxRingEntry],
    clocks: Clocks,
    pins: EthPins<REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1>,
    mdio: MDIO,
    mdc: MDC,
) -> Result<Parts<'rx, 'tx, EthernetMACWithMii<MDIO, MDC>>, WrongClock>
where
    REFCLK: RmiiRefClk + AlternateVeryHighSpeed,
    CRS: RmiiCrsDv + AlternateVeryHighSpeed,
    TXEN: RmiiTxEN + AlternateVeryHighSpeed,
    TXD0: RmiiTxD0 + AlternateVeryHighSpeed,
    TXD1: RmiiTxD1 + AlternateVeryHighSpeed,
    RXD0: RmiiRxD0 + AlternateVeryHighSpeed,
    RXD1: RmiiRxD1 + AlternateVeryHighSpeed,
    MDIO: MdioPin,
    MDC: MdcPin,
{
    // Configure all of the pins correctly
    pins.setup_pins();

    // Set up the clocks and reset the MAC periperhal
    setup::setup();

    let eth_mac = parts.mac.into();

    // Congfigure and start up the ethernet DMA.
    let dma = EthernetDMA::new(parts.dma.into(), rx_buffer, tx_buffer);

    // Configure the ethernet PTP
    let ptp = EthernetPTP::new(parts.ptp.into(), clocks, &dma);

    // Configure the ethernet MAC
    let mac = EthernetMAC::new(eth_mac, parts.mmc, clocks, Speed::FullDuplexBase100Tx, &dma)?
        .with_mii(mdio, mdc);

    let parts = Parts { mac, dma, ptp };

    Ok(parts)
}
