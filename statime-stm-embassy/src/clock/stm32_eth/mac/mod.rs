//! Ethernet MAC access and configuration.

use core::ops::{Deref, DerefMut};

use embassy_stm32::rcc::Clocks;
use stm32f7::stm32f7x9::ETHERNET_MMC;

pub use miim::*;

use crate::clock::stm32_eth::{dma::EthernetDMA, peripherals::ETHERNET_MAC};

use self::consts::*;

mod miim;

/// Speeds at which this MAC can be configured
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    /// 10Base-T half duplex
    HalfDuplexBase10T,
    /// 10Base-T full duplex
    FullDuplexBase10T,
    /// 100Base-Tx half duplex
    HalfDuplexBase100Tx,
    /// 100Base-Tx full duplex
    FullDuplexBase100Tx,
}

mod consts {
    /* For HCLK 60-100 MHz */
    pub const ETH_MACMIIAR_CR_HCLK_DIV_42: u8 = 0;
    /* For HCLK 100-150 MHz */
    pub const ETH_MACMIIAR_CR_HCLK_DIV_62: u8 = 1;
    /* For HCLK 20-35 MHz */
    pub const ETH_MACMIIAR_CR_HCLK_DIV_16: u8 = 2;
    /* For HCLK 35-60 MHz */
    pub const ETH_MACMIIAR_CR_HCLK_DIV_26: u8 = 3;
    /* For HCLK over 150 MHz */
    pub const ETH_MACMIIAR_CR_HCLK_DIV_102: u8 = 4;
}

/// HCLK must be at least 25MHz to use the ethernet peripheral.
/// This (empty) struct is returned to indicate that it is not set
/// correctly
#[derive(Debug)]
pub struct WrongClock;

/// Ethernet media access control (MAC).
///
// impl note: access to the MACIMR register should _only_ be performed
// atomically.
pub struct EthernetMAC {
    eth_mac: ETHERNET_MAC,
}

impl EthernetMAC {
    /// Create a new EthernetMAC that does not own its MDIO and MDC pins.
    ///     
    /// HCLK must be at least 25MHz, else this function will return `Err(WrongClock)`.
    ///
    /// This method does not initialise the external PHY. However, you can access SMI
    /// `read` and `write` functions through the [`Self::mii`] and [`Self::with_miim`] functions.
    ///
    /// Additionally, an `impl` of the [`ieee802_3_miim::Miim`] trait is available
    /// for PHY communication.
    pub(crate) fn new(
        eth_mac: ETHERNET_MAC,
        eth_mmc: ETHERNET_MMC,
        clocks: Clocks,
        initial_speed: Speed,
        // Note(_dma): this field exists to ensure that the MAC is not
        // initialized before the DMA. If MAC is started before the DMA,
        // it doesn't work.
        _dma: &EthernetDMA,
    ) -> Result<Self, WrongClock> {
        let clock_frequency = clocks.hclk().to_Hz();

        let clock_range = match clock_frequency {
            0..=24_999_999 => return Err(WrongClock),
            25_000_000..=34_999_999 => ETH_MACMIIAR_CR_HCLK_DIV_16,
            35_000_000..=59_999_999 => ETH_MACMIIAR_CR_HCLK_DIV_26,
            60_000_000..=99_999_999 => ETH_MACMIIAR_CR_HCLK_DIV_42,
            100_000_000..=149_999_999 => ETH_MACMIIAR_CR_HCLK_DIV_62,
            _ => ETH_MACMIIAR_CR_HCLK_DIV_102,
        };

        // Set clock range in MAC MII address register
        eth_mac
            .macmiiar
            .modify(|_, w| unsafe { w.cr().bits(clock_range) });

        // Configuration Register
        eth_mac.maccr.modify(|_, w| {
            // CRC stripping for Type frames. STM32F1xx do not have this bit.
            let w = w.cstf().set_bit();

            // Fast Ethernet speed
            w.fes()
                .set_bit()
                // Duplex mode
                .dm()
                .set_bit()
                // IPv4 checksum offload
                .ipco()
                .set_bit()
                // Automatic pad/CRC stripping
                .apcs()
                .set_bit()
                // Retry disable in half-duplex mode
                .rd()
                .set_bit()
                // Receiver enable
                .re()
                .set_bit()
                // Transmitter enable
                .te()
                .set_bit()
        });

        // Frame filter register
        eth_mac.macffr.modify(|_, w| {
            // Receive All
            w.ra()
                .set_bit()
                // Promiscuous mode
                .pm()
                .set_bit()
        });

        // Flow Control Register
        eth_mac.macfcr.modify(|_, w| {
            // Pause time
            w.pt().bits(0x100)
        });

        // Disable all MMC RX interrupts
        eth_mmc
            .mmcrimr
            .write(|w| w.rgufm().set_bit().rfaem().set_bit().rfcem().set_bit());

        // Disable all MMC TX interrupts
        eth_mmc
            .mmctimr
            .write(|w| w.tgfm().set_bit().tgfmscm().set_bit().tgfscm().set_bit());

        // Fix incorrect TGFM bit position until https://github.com/stm32-rs/stm32-rs/pull/689
        // is released and used by HALs.
        eth_mmc
            .mmctimr
            .modify(|r, w| unsafe { w.bits(r.bits() | (1 << 21)) });

        let mut me = Self { eth_mac };

        me.set_speed(initial_speed);

        Ok(me)
    }

    /// Borrow access to the MAC's SMI.
    ///
    /// Allows for controlling and monitoring any PHYs that may be accessible via the MDIO/MDC
    /// pins.
    ///
    /// Exclusive access to the `MDIO` and `MDC` is required to ensure that are not used elsewhere
    /// for the duration of Mii communication.
    pub fn mii<'eth, 'pins, Mdio, Mdc>(
        &'eth mut self,
        mdio: &'pins mut Mdio,
        mdc: &'pins mut Mdc,
    ) -> Stm32Mii<'eth, 'pins, Mdio, Mdc>
    where
        Mdio: MdioPin,
        Mdc: MdcPin,
    {
        Stm32Mii::new(self, mdio, mdc)
    }

    /// Turn this [`EthernetMAC`] into an [`EthernetMACWithMii`]
    pub fn with_mii<MDIO, MDC>(self, mdio: MDIO, mdc: MDC) -> EthernetMACWithMii<MDIO, MDC>
    where
        MDIO: MdioPin,
        MDC: MdcPin,
    {
        EthernetMACWithMii {
            eth_mac: self,
            mdio,
            mdc,
        }
    }

    /// Set the Ethernet Speed at which the MAC communicates
    ///
    /// Note that this does _not_ affect the PHY in any way. To
    /// configure the PHY, use [`EthernetMACWithMii`] (see: [`Self::with_mii`])
    /// or [`Stm32Mii`] (see: [`Self::mii`])
    pub fn set_speed(&mut self, speed: Speed) {
        self.eth_mac.maccr.modify(|_, w| match speed {
            Speed::HalfDuplexBase10T => w.fes().clear_bit().dm().clear_bit(),
            Speed::FullDuplexBase10T => w.fes().clear_bit().dm().set_bit(),
            Speed::HalfDuplexBase100Tx => w.fes().set_bit().dm().clear_bit(),
            Speed::FullDuplexBase100Tx => w.fes().set_bit().dm().set_bit(),
        });
    }

    /// Get the Ethernet Speed at which the MAC communicates
    pub fn get_speed(&self) -> Speed {
        let cr = self.eth_mac.maccr.read();
        match (cr.fes().bit_is_set(), cr.dm().bit_is_set()) {
            (false, false) => Speed::HalfDuplexBase10T,
            (false, true) => Speed::FullDuplexBase10T,
            (true, false) => Speed::HalfDuplexBase100Tx,
            (true, true) => Speed::FullDuplexBase100Tx,
        }
    }

    pub(crate) fn mask_timestamp_trigger_interrupt() {
        // SAFETY: MACIMR only receives atomic writes.
        let mac = &unsafe { &*ETHERNET_MAC::ptr() };
        mac.macimr.write(|w| w.tstim().set_bit());
    }

    // NOTE(allow): only used on F4 and F7
    #[allow(dead_code)]
    pub(crate) fn unmask_timestamp_trigger_interrupt() {
        // SAFETY: MACIMR only receives atomic writes.
        let macimr = &unsafe { &*ETHERNET_MAC::ptr() }.macimr;
        macimr.write(|w| w.tstim().clear_bit());
    }
}

/// Ethernet media access control (MAC) with owned MII
///
/// This version of the struct owns it's MII pins,
/// allowing it to be used directly, instead of requiring
/// that a  [`Miim`] is created.
pub struct EthernetMACWithMii<MDIO, MDC>
where
    MDIO: MdioPin,
    MDC: MdcPin,
{
    pub(crate) eth_mac: EthernetMAC,
    mdio: MDIO,
    mdc: MDC,
}

impl<MDIO, MDC> EthernetMACWithMii<MDIO, MDC>
where
    MDIO: MdioPin,
    MDC: MdcPin,
{
    /// Create a new EthernetMAC with owned MDIO and MDC pins.
    ///
    /// To interact with a connected Phy, use the `read` and `write` functions.
    ///
    /// Functionality for interacting with PHYs from the `ieee802_3_miim` crate
    /// is available.
    pub fn new(eth_mac: EthernetMAC, mdio: MDIO, mdc: MDC) -> Self {
        Self { eth_mac, mdio, mdc }
    }

    /// Release the owned MDIO and MDC pins, and return an EthernetMAC that
    /// has to borrow the MDIO and MDC pins.
    pub fn release_pins(self) -> (EthernetMAC, MDIO, MDC) {
        (self.eth_mac, self.mdio, self.mdc)
    }
}

impl<MDIO, MDC> Deref for EthernetMACWithMii<MDIO, MDC>
where
    MDIO: MdioPin,
    MDC: MdcPin,
{
    type Target = EthernetMAC;

    fn deref(&self) -> &Self::Target {
        &self.eth_mac
    }
}

impl<MDIO, MDC> DerefMut for EthernetMACWithMii<MDIO, MDC>
where
    MDIO: MdioPin,
    MDC: MdcPin,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.eth_mac
    }
}

impl<MDIO, MDC> EthernetMACWithMii<MDIO, MDC>
where
    MDIO: MdioPin,
    MDC: MdcPin,
{
    /// Read MII register `reg` from the PHY at address `phy`
    pub fn read(&mut self, phy: u8, reg: u8) -> u16 {
        self.eth_mac
            .mii(&mut self.mdio, &mut self.mdc)
            .read(phy, reg)
    }

    /// Write the value `data` to MII register `reg` to the PHY at address `phy`
    pub fn write(&mut self, phy: u8, reg: u8, data: u16) {
        self.eth_mac
            .mii(&mut self.mdio, &mut self.mdc)
            .write(phy, reg, data)
    }
}

impl<MDIO, MDC> miim::Miim for EthernetMACWithMii<MDIO, MDC>
where
    MDIO: MdioPin,
    MDC: MdcPin,
{
    fn read(&mut self, phy: u8, reg: u8) -> u16 {
        self.read(phy, reg)
    }

    fn write(&mut self, phy: u8, reg: u8, data: u16) {
        self.write(phy, reg, data)
    }
}
