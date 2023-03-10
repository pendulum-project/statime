//! Pin definitions and setup functionality.
//!
//! This module contains the unsafe traits that determine
//! which pins can have a specific function, and provides
//! functionality for setting up clocks and the MAC peripheral

#[cfg(feature = "stm32f4xx-hal")]
use stm32f4xx_hal::{
    bb,
    gpio::{
        gpioa::{PA1, PA7},
        gpiob::{PB11, PB12, PB13},
        gpioc::{PC4, PC5},
        gpiog::{PG11, PG13, PG14},
        Input,
        Speed::VeryHigh,
    },
    pac::{RCC, SYSCFG},
};

#[cfg(feature = "stm32f7xx-hal")]
use cortex_m::interrupt;

#[cfg(feature = "stm32f7xx-hal")]
use stm32f7xx_hal::{
    gpio::{
        gpioa::{PA1, PA7},
        gpiob::{PB11, PB12, PB13},
        gpioc::{PC4, PC5},
        gpiog::{PG11, PG13, PG14},
        Input,
        Speed::VeryHigh,
    },
    pac::{RCC, SYSCFG},
};

use crate::clock::stm32_eth::{
    dma::EthernetDMA,
    stm32::{ETHERNET_DMA, ETHERNET_MAC, ETHERNET_MMC},
};

#[cfg(feature = "ptp")]
use crate::{ptp::EthernetPTP, stm32::ETHERNET_PTP};

// Enable syscfg and ethernet clocks. Reset the Ethernet MAC.
pub(crate) fn setup() {
    #[cfg(feature = "stm32f4xx-hal")]
    unsafe {
        const SYSCFG_BIT: u8 = 14;
        const ETH_MAC_BIT: u8 = 25;
        const ETH_TX_BIT: u8 = 26;
        const ETH_RX_BIT: u8 = 27;
        const MII_RMII_BIT: u8 = 23;

        //NOTE(unsafe) This will only be used for atomic writes with no side-effects
        let rcc = &*RCC::ptr();
        let syscfg = &*SYSCFG::ptr();

        // Enable syscfg clock
        bb::set(&rcc.apb2enr, SYSCFG_BIT);

        if rcc.ahb1enr.read().ethmacen().bit_is_set() {
            // pmc must be changed with the ethernet controller disabled or under reset
            bb::clear(&rcc.ahb1enr, ETH_MAC_BIT);
        }
        // select MII or RMII mode
        // 0 = MII, 1 = RMII
        bb::set(&syscfg.pmc, MII_RMII_BIT);

        // enable ethernet clocks
        bb::set(&rcc.ahb1enr, ETH_MAC_BIT);
        bb::set(&rcc.ahb1enr, ETH_TX_BIT);
        bb::set(&rcc.ahb1enr, ETH_RX_BIT);

        // reset pulse
        bb::set(&rcc.ahb1rstr, ETH_MAC_BIT);
        bb::clear(&rcc.ahb1rstr, ETH_MAC_BIT);
    }
    #[cfg(feature = "stm32f7xx-hal")]
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

    #[cfg(feature = "stm32f1xx-hal")]
    cortex_m::interrupt::free(|_| unsafe {
        let afio = &*crate::stm32::AFIO::ptr();
        let rcc = &*crate::stm32::RCC::ptr();

        // enable AFIO clock
        rcc.apb2enr.modify(|_, w| w.afioen().set_bit());

        if rcc.ahbenr.read().ethmacen().bit_is_set() {
            // ethernet controller must be disabled when configuring mapr
            rcc.ahbenr.modify(|_, w| w.ethmacen().clear_bit());
        }

        // select MII or RMII mode
        // 0 = MII, 1 = RMII
        afio.mapr.modify(|_, w| w.mii_rmii_sel().set_bit());

        // enable ethernet clocks
        rcc.ahbenr.modify(|_, w| {
            w.ethmacen()
                .set_bit()
                .ethmactxen()
                .set_bit()
                .ethmacrxen()
                .set_bit()
                .ethmacen()
                .set_bit()
        });

        // Reset pulse.
        rcc.ahbrstr.modify(|_, w| w.ethmacrst().set_bit());
        rcc.ahbrstr.modify(|_, w| w.ethmacrst().clear_bit());

        // Workaround for the issue mentioned in the Errata (2.20.11) related to wfi:
        //
        // "
        // If a WFI/WFE instruction is executed to put the system in sleep mode while the Ethernet
        // MAC master clock on the AHB bus matrix is ON and all remaining masters clocks are OFF,
        // the Ethernet DMA is unable to perform any AHB master accesses during sleep mode.
        //
        // Workaround: Enable DMA1 or DMA2 clocks in the RCC_AHBENR register before executing the
        // WFI/WFE instruction.
        // "
        if rcc.ahbenr.read().dma1en().is_disabled() && rcc.ahbenr.read().dma2en().is_disabled() {
            rcc.ahbenr.modify(|_, w| w.dma2en().enabled());
            while rcc.ahbenr.read().dma2en().is_disabled() {}
        }
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
    #[cfg(feature = "ptp")]
    pub ptp: ETHERNET_PTP,
}

#[cfg(feature = "ptp")]
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

#[cfg(not(feature = "ptp"))]
impl From<(ETHERNET_MAC, ETHERNET_MMC, ETHERNET_DMA)> for PartsIn {
    fn from(value: (ETHERNET_MAC, ETHERNET_MMC, ETHERNET_DMA)) -> Self {
        Self {
            mac: value.0,
            mmc: value.1,
            dma: value.2,
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
    #[cfg(feature = "ptp")]
    pub ptp: EthernetPTP,
}

#[cfg(feature = "ptp")]
impl<'rx, 'tx, T> Parts<'rx, 'tx, T> {
    /// Split this [`Parts`] into its components.
    pub fn split(self) -> (T, EthernetDMA<'rx, 'tx>, EthernetPTP) {
        (self.mac, self.dma, self.ptp)
    }
}

#[cfg(not(feature = "ptp"))]
impl<'rx, 'tx, T> Parts<'rx, 'tx, T> {
    /// Split this [`Parts`] into its components.
    pub fn split(self) -> (T, EthernetDMA<'rx, 'tx>) {
        (self.mac, self.dma)
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

#[cfg(any(feature = "stm32f4xx-hal", feature = "stm32f7xx-hal"))]
impl_pins!(
    RmiiRefClk: [
        PA1<Input>,
    ],
    RmiiCrsDv: [
        PA7<Input>,
    ],
    RmiiTxEN: [
        PB11<Input>,
        PG11<Input>,
    ],
    RmiiTxD0: [
        PB12<Input>,
        PG13<Input>,
    ],
    RmiiTxD1: [
        PB13<Input>,
        PG14<Input>,
    ],
    RmiiRxD0: [
        PC4<Input>,
    ],
    RmiiRxD1: [
        PC5<Input>,
    ],
);

#[cfg(feature = "stm32f1xx-hal")]
mod stm32f1 {
    use super::*;
    use stm32f1xx_hal::gpio::{
        gpioa::*, gpiob::*, gpioc::*, gpiod::*, Alternate, Floating, IOPinSpeed, Input,
        OutputSpeed, PushPull,
    };

    // STM32F1xx's require access to the CRL/CRH registers to change pin mode. As a result, we
    // require that pins are already in the necessary mode before constructing `EthPins` as it
    // would be inconvenient to pass CRL and CRH through to the `AlternateVeryHighSpeed` callsite.

    macro_rules! impl_pins {
        ($($type:ident: [$(($PIN:ty, $is_input:literal)),+]),*) => {
            $(
                $(
                    unsafe impl $type for $PIN {}
                    impl AlternateVeryHighSpeed for $PIN {
                        fn into_af11_very_high_speed(self) {
                            // Within this critical section, modifying the `CRL` register can
                            // only be unsound if this critical section preempts other code
                            // that is modifying the same register
                            cortex_m::interrupt::free(|_| {
                                // SAFETY: this is sound as long as the API of the HAL and structure of the CRL
                                // struct does not change. In case the size of the `CRL` struct is changed, compilation
                                // will fail as `mem::transmute` can only convert between types of the same size.
                                //
                                // This guards us from unsound behaviour introduced by point releases of the f1 hal
                                let cr: &mut _ = &mut unsafe { core::mem::transmute(()) };
                                // The speed can only be changed on output pins
                                let mut pin = self.into_alternate_push_pull(cr);
                                pin.set_speed(cr, IOPinSpeed::Mhz50);

                                if $is_input {
                                    pin.into_floating_input(cr);
                                }
                            });
                        }
                    }
                )+
            )*
        };
    }

    impl_pins!(
        RmiiRefClk: [(PA1<Input<Floating>>, true)],
        RmiiCrsDv: [(PA7<Input<Floating>>, true), (PD8<Input<Floating>>, true)],
        RmiiTxEN: [(PB11<Alternate<PushPull>>, false)],
        RmiiTxD0: [(PB12<Alternate<PushPull>>, false)],
        RmiiTxD1: [(PB13<Alternate<PushPull>>, false)],
        RmiiRxD0: [(PC4<Input<Floating>>, true), (PD9<Input<Floating>>, true)],
        RmiiRxD1: [(PC5<Input<Floating>>, true), (PD10<Input<Floating>>, true)]
    );
}
