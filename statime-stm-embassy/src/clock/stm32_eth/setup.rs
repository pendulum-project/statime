//! Pin definitions and setup functionality.
//!
//! This module contains the unsafe traits that determine
//! which pins can have a specific function, and provides
//! functionality for setting up clocks and the MAC peripheral

use crate::clock::stm32_eth::peripherals::{ETHERNET_DMA, ETHERNET_MAC, ETHERNET_PTP};
use cortex_m::interrupt;
use stm32f7::stm32f7x9::{ETHERNET_MMC, RCC, SYSCFG};

use embassy_stm32::{
    gpio::{Input, Speed::VeryHigh},
    pac::{RCC, SYSCFG},
    peripherals::{PA1, PA7, PB11, PB12, PB13, PC4, PC5, PG11, PG13, PG14},
};

use crate::clock::stm32_eth::dma::EthernetDMA;

use crate::clock::stm32_eth::ptp::EthernetPTP;

// Enable syscfg and ethernet clocks. Reset the Ethernet MAC.
pub(crate) fn setup() {
    //stm32f7xx-hal does not currently have bitbanding
    interrupt::free(|_| unsafe {
        //NOTE(unsafe) Interrupt free and we only modify mac bits
        let rcc = &*RCC::ptr();
        let syscfg = &*SYSCFG::ptr();
        // enable syscfg clock
        rcc.apb2enr.modify(|_, w| w.syscfgen().set_bit());

        if rcc.ahb1enr.read().ethmacen().bit_is_set() {
            // pmc must be changed with the ethernet controller disabled or under reset
            rcc.ahb1enr.modify(|_, w| w.ethmacen().clear_bit());
        }

        // select MII or RMII mode
        // 0 = MII, 1 = RMII
        syscfg.pmc.modify(|_, w| w.mii_rmii_sel().set_bit());

        // enable ethernet clocks
        rcc.ahb1enr.modify(|_, w| {
            w.ethmacen()
                .set_bit()
                .ethmactxen()
                .set_bit()
                .ethmacrxen()
                .set_bit()
        });

        //reset pulse
        rcc.ahb1rstr.modify(|_, w| w.ethmacrst().set_bit());
        rcc.ahb1rstr.modify(|_, w| w.ethmacrst().clear_bit());
    });
}

macro_rules ! pin_trait {
    ($([$name:ident, $doc:literal, $rm_name:literal]),*) => {
        $(
        #[doc = concat!($doc, "\n# Safety\nOnly pins specified as `ETH_RMII_", $rm_name, "` in a part's Reference Manual\nmay implement this trait.")]
        pub unsafe trait $name {}
        )*
    }
}

pin_trait!(
    [RmiiRefClk, "RMII Reference Clock", "REF_CLK"],
    [RmiiCrsDv, "RMII Rx Data Valid", "CRS_DV"],
    [RmiiTxEN, "RMII TX Enable", "TX_EN"],
    [RmiiTxD0, "RMII TX Data Pin 0", "TXD0"],
    [RmiiTxD1, "RMII TX Data Pin 1", "TXD1"],
    [RmiiRxD0, "RMII RX Data Pin 0", "RXD0"],
    [RmiiRxD1, "RMII RX Data Pin 1", "RXD1"]
);

/// Trait needed to setup the pins for the Ethernet peripheral.
pub trait AlternateVeryHighSpeed {
    /// Puts the pin in the Alternate Function 11 with Very High Speed.
    fn into_af11_very_high_speed(self);
}

/// A struct that contains all peripheral parts required to configure
/// the ethernet peripheral.
#[allow(missing_docs)]
pub struct PartsIn {
    pub mac: ETHERNET_MAC,
    pub mmc: ETHERNET_MMC,
    pub dma: ETHERNET_DMA,
    pub ptp: ETHERNET_PTP,
}

impl From<(ETHERNET_MAC, ETHERNET_MMC, ETHERNET_DMA, ETHERNET_PTP)> for PartsIn {
    fn from(value: (ETHERNET_MAC, ETHERNET_MMC, ETHERNET_DMA, ETHERNET_PTP)) -> Self {
        Self {
            mac: value.0,
            mmc: value.1,
            dma: value.2,
            ptp: value.3,
        }
    }
}

/// Access to all configured parts of the ethernet peripheral.
pub struct Parts<'rx, 'tx, T> {
    /// Access to and control over the ethernet MAC.
    pub mac: T,
    /// Access to and control over the ethernet DMA.
    pub dma: EthernetDMA<'rx, 'tx>,
    /// Access to and control over the ethernet PTP module.
    pub ptp: EthernetPTP,
}

impl<'rx, 'tx, T> Parts<'rx, 'tx, T> {
    /// Split this [`Parts`] into its components.
    pub fn split(self) -> (T, EthernetDMA<'rx, 'tx>, EthernetPTP) {
        (self.mac, self.dma, self.ptp)
    }
}

/// A struct that represents a combination of pins to be used
/// as RMII pins for the ethernet peripheral(s)
// NOTE(missing_docs): all fields of this struct are self-explanatory
#[allow(missing_docs)]
pub struct EthPins<REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1> {
    pub ref_clk: REFCLK,
    pub crs: CRS,
    pub tx_en: TXEN,
    pub tx_d0: TXD0,
    pub tx_d1: TXD1,
    pub rx_d0: RXD0,
    pub rx_d1: RXD1,
}

impl<REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1> EthPins<REFCLK, CRS, TXEN, TXD0, TXD1, RXD0, RXD1>
where
    REFCLK: RmiiRefClk + AlternateVeryHighSpeed,
    CRS: RmiiCrsDv + AlternateVeryHighSpeed,
    TXEN: RmiiTxEN + AlternateVeryHighSpeed,
    TXD0: RmiiTxD0 + AlternateVeryHighSpeed,
    TXD1: RmiiTxD1 + AlternateVeryHighSpeed,
    RXD0: RmiiRxD0 + AlternateVeryHighSpeed,
    RXD1: RmiiRxD1 + AlternateVeryHighSpeed,
{
    /// Pin setup.
    ///
    /// Set RMII pins to
    /// * Alternate function 11
    /// * High-speed
    ///
    /// This function consumes the pins so that you cannot use them
    /// anywhere else by accident.
    pub fn setup_pins(self) {
        self.ref_clk.into_af11_very_high_speed();
        self.crs.into_af11_very_high_speed();
        self.tx_en.into_af11_very_high_speed();
        self.tx_d0.into_af11_very_high_speed();
        self.tx_d1.into_af11_very_high_speed();
        self.rx_d0.into_af11_very_high_speed();
        self.rx_d1.into_af11_very_high_speed();
    }
}

#[allow(unused_macros)]
macro_rules! impl_pins {
    ( $($traity:ident: [$($pin:ty,)+],)+ ) => {
        $(
            $(
                unsafe impl $traity for $pin {}

                impl AlternateVeryHighSpeed for $pin {
                    fn into_af11_very_high_speed(self) {
                        self.into_alternate::<11>().set_speed(VeryHigh);
                    }
                }
            )+
        )+
    };
}

impl_pins!(
    RmiiRefClk: [
        PA1,
    ],
    RmiiCrsDv: [
        PA7,
    ],
    RmiiTxEN: [
        PB11,
        PG11,
    ],
    RmiiTxD0: [
        PB12,
        PG13,
    ],
    RmiiTxD1: [
        PB13,
        PG14,
    ],
    RmiiRxD0: [
        PC4,
    ],
    RmiiRxD1: [
        PC5,
    ],
);
