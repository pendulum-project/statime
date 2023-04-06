// The v1c ethernet driver was ported to embassy from the awesome stm32-eth project (https://github.com/stm32-rs/stm32-eth).

use core::sync::atomic::{fence, Ordering};

use defmt::info;
use embassy_stm32::{
    eth::{
        CRSPin, MDCPin, MDIOPin, RXD0Pin, RXD1Pin, RefClkPin, StationManagement, TXD0Pin, TXD1Pin,
        TXEnPin, PHY,
    },
    gpio::{
        low_level::{AFType, Pin},
        AnyPin,
    },
    interrupt::InterruptExt,
    into_ref,
    pac::{
        eth::vals::{
            Apcs, Cr, Dm, DmaomrSr, Edfe, Fes, Ftf, Ifg, MbProgress, Mw, Pbl, Rsf, St, Tsf, Tstim,
        },
        ETH, RCC, SYSCFG,
    },
    rcc::RccPeripheral,
    Peripheral, PeripheralRef,
};

pub(crate) use self::{
    rx_desc::{RDes, RDesRing},
    tx_desc::{TDes, TDesRing},
};
use super::*;

mod rx_desc;
mod tx_desc;

pub struct Ethernet<'d, T: Instance, P: PHY> {
    _peri: PeripheralRef<'d, T>,
    pub(crate) tx: TDesRing<'d>,
    pub(crate) rx: RDesRing<'d>,

    pins: [PeripheralRef<'d, AnyPin>; 9],
    _phy: P,
    clock_range: Cr,
    phy_addr: u8,
    pub(crate) mac_addr: [u8; 6],
}

pub struct PTPClock<'d> {
    pps_pin: PeripheralRef<'d, AnyPin>,
    base_addend: u32,
}

impl<'d> PTPClock<'d> {
    fn new<T: Instance + RccPeripheral>(pps: impl Peripheral<P = impl PPSPin> + 'd) -> Self {
        let pps = pps.into_ref();

        let hclk = T::frequency();
        let half_hclk_hz = hclk.0 / 2;
        // round to nearest
        let step = ((1_u32 << 31) + half_hclk_hz / 2) / half_hclk_hz;
        let stepclk = (step as u64) * (hclk.0 as u64);
        // round to nearest (note, guaranteed to fit in u32 by previous math)
        let addend = (((1_u64 << 63) + stepclk / 2) / stepclk) as u32;

        debug_assert!(step <= u8::MAX as u32);

        // Enable timestamping hardware
        unsafe {
            ETH.ethernet_mac().macimr().modify(|w| {
                w.set_tstim(Tstim::MASKED);
            });

            ETH.ethernet_ptp().ptptscr().modify(|w| {
                w.set_tssarfe(true);
                w.set_tse(true);
                w.set_tsfcu(true);
            });
        }

        let pps_af = pps.af_num();
        let mut this = PTPClock {
            pps_pin: pps.map_into(),
            base_addend: addend,
        };

        // Setup clock frequency
        unsafe {
            ETH.ethernet_ptp().ptpssir().modify(|w| {
                w.set_stssi(step as u8);
            });
        }
        this.set_addend(addend);

        // Set initial time
        this.set_time(0, 0);

        // And enable pps
        unsafe {
            this.pps_pin.set_as_af(
                pps_af,
                embassy_stm32::gpio::low_level::AFType::OutputPushPull,
            );
            this.pps_pin.set_speed(embassy_stm32::gpio::Speed::VeryHigh);

            let ptpppscr = ETH.ethernet_ptp().ptpppscr().ptr() as *mut u32;
            let val = core::ptr::read_volatile(ptpppscr) & !15;
            core::ptr::write_volatile(ptpppscr, val);
        }

        this
    }

    pub fn time(&self) -> (u32, u32) {
        // We cant do an atomic read of the entire timestamp.
        // To deal with this, we read the seconds part twice, once
        // before and once after reading the subseconds.
        // Then, if subseconds is low, the after is the accurate
        // value and returned, and if subseconds is high, before
        // is accurate.

        // Safety: reads are atomic and have no side effects
        let before = unsafe { ETH.ethernet_ptp().ptptshr().read().sts() };
        let sub = unsafe { ETH.ethernet_ptp().ptptslr().read().stss() };
        let after = unsafe { ETH.ethernet_ptp().ptptshr().read().sts() };

        if sub < (1 << 30) {
            (after, sub)
        } else {
            (before, sub)
        }
    }

    pub fn set_time(&mut self, secs: u32, subsecs: u32) {
        debug_assert!(subsecs < (1 << 31));

        unsafe {
            critical_section::with(|_| {
                ETH.ethernet_ptp().ptptshur().modify(|w| {
                    w.set_tsus(secs);
                });
                ETH.ethernet_ptp().ptptslur().modify(|w| {
                    w.set_tsupns(false);
                    w.set_tsuss(subsecs)
                });

                while ETH.ethernet_ptp().ptptscr().read().tssti() {}
                ETH.ethernet_ptp().ptptscr().modify(|w| {
                    w.set_tssti(true);
                });
                while ETH.ethernet_ptp().ptptscr().read().tssti() {}
            })
        }
    }

    pub fn jump_time(&mut self, substract: bool, secs: u32, subsecs: u32) {
        debug_assert!(subsecs < (1 << 31));

        unsafe {
            critical_section::with(|_| {
                ETH.ethernet_ptp().ptptshur().modify(|w| {
                    w.set_tsus(secs);
                });
                ETH.ethernet_ptp().ptptslur().modify(|w| {
                    w.set_tsupns(substract);
                    w.set_tsuss(subsecs)
                });

                let read_status = || {
                    let scr = ETH.ethernet_ptp().ptptscr().read();
                    scr.tssti() || scr.tsstu()
                };

                while read_status() {}
                ETH.ethernet_ptp().ptptscr().modify(|w| {
                    w.set_tsstu(true);
                });
                while ETH.ethernet_ptp().ptptscr().read().tsstu() {}
            })
        }
    }

    fn set_addend(&mut self, addend: u32) {
        unsafe {
            critical_section::with(|_| {
                ETH.ethernet_ptp().ptptsar().modify(|w| {
                    w.set_tsa(addend);
                });
                while ETH.ethernet_ptp().ptptscr().read().ttsaru() {}
                ETH.ethernet_ptp().ptptscr().modify(|w| {
                    w.set_ttsaru(true);
                });
                while ETH.ethernet_ptp().ptptscr().read().ttsaru() {}
            })
        }
    }

    pub fn set_freq(&mut self, ppm_offset: f32) {
        // casting order is on purpose, this makes negative numbers such that adding
        // with wrapping results in an effective substraction on
        // self.base_addend later
        let offset = (ppm_offset * (self.base_addend as f32)) as i32;
        debug_assert!((offset.abs() as u32) < self.base_addend);
        self.set_addend(self.base_addend.wrapping_add(offset as u32));
    }
}

macro_rules! config_pins {
    ($($pin:ident),*) => {
        // NOTE(unsafe) Exclusive access to the registers
        critical_section::with(|_| {
            $(
                $pin.set_as_af($pin.af_num(), AFType::OutputPushPull);
                $pin.set_speed(embassy_stm32::gpio::Speed::VeryHigh);
            )*
        })
    };
}

impl<'d, T: Instance + RccPeripheral, P: PHY> Ethernet<'d, T, P> {
    /// safety: the returned instance is not leak-safe
    pub fn new_with_ptp<const TX: usize, const RX: usize>(
        queue: &'d mut PacketQueue<TX, RX>,
        peri: impl Peripheral<P = T> + 'd,
        interrupt: impl Peripheral<P = embassy_stm32::interrupt::ETH> + 'd,
        ref_clk: impl Peripheral<P = impl RefClkPin<T>> + 'd,
        mdio: impl Peripheral<P = impl MDIOPin<T>> + 'd,
        mdc: impl Peripheral<P = impl MDCPin<T>> + 'd,
        crs: impl Peripheral<P = impl CRSPin<T>> + 'd,
        rx_d0: impl Peripheral<P = impl RXD0Pin<T>> + 'd,
        rx_d1: impl Peripheral<P = impl RXD1Pin<T>> + 'd,
        tx_d0: impl Peripheral<P = impl TXD0Pin<T>> + 'd,
        tx_d1: impl Peripheral<P = impl TXD1Pin<T>> + 'd,
        tx_en: impl Peripheral<P = impl TXEnPin<T>> + 'd,
        pps: impl Peripheral<P = impl PPSPin> + 'd,
        phy: P,
        mac_addr: [u8; 6],
        phy_addr: u8,
    ) -> (Self, PTPClock<'d>) {
        into_ref!(peri, interrupt, ref_clk, mdio, mdc, crs, rx_d0, rx_d1, tx_d0, tx_d1, tx_en);

        unsafe {
            // Enable the necessary Clocks
            // NOTE(unsafe) We have exclusive access to the registers
            critical_section::with(|_| {
                RCC.apb2enr().modify(|w| w.set_syscfgen(true));
                RCC.ahb1enr().modify(|w| {
                    w.set_ethen(true);
                    w.set_ethtxen(true);
                    w.set_ethrxen(true);
                });

                // RMII (Reduced Media Independent Interface)
                SYSCFG.pmc().modify(|w| w.set_mii_rmii_sel(true));
            });

            config_pins!(ref_clk, mdio, mdc, crs, rx_d0, rx_d1, tx_d0, tx_d1, tx_en);

            // NOTE(unsafe) We have exclusive access to the registers
            let dma = ETH.ethernet_dma();
            let mac = ETH.ethernet_mac();

            // Reset and wait
            dma.dmabmr().modify(|w| w.set_sr(true));
            while dma.dmabmr().read().sr() {}

            mac.maccr().modify(|w| {
                w.set_ifg(Ifg::IFG96); // inter frame gap 96 bit times
                w.set_apcs(Apcs::STRIP); // automatic padding and crc stripping
                w.set_fes(Fes::FES100); // fast ethernet speed
                w.set_dm(Dm::FULLDUPLEX); // full duplex
                                          // TODO: Carrier sense ? ECRSFD
            });

            // Note: Writing to LR triggers synchronisation of both LR and HR into the MAC
            // core, so the LR write must happen after the HR write.
            mac.maca0hr()
                .modify(|w| w.set_maca0h(u16::from(mac_addr[4]) | (u16::from(mac_addr[5]) << 8)));
            mac.maca0lr().write(|w| {
                w.set_maca0l(
                    u32::from(mac_addr[0])
                        | (u32::from(mac_addr[1]) << 8)
                        | (u32::from(mac_addr[2]) << 16)
                        | (u32::from(mac_addr[3]) << 24),
                )
            });

            // pause time
            mac.macfcr().modify(|w| w.set_pt(0x100));

            // Transfer and Forward, Receive and Forward
            dma.dmaomr().modify(|w| {
                w.set_tsf(Tsf::STOREFORWARD);
                w.set_rsf(Rsf::STOREFORWARD);
            });

            dma.dmabmr().modify(|w| {
                w.set_pbl(Pbl::PBL32); // programmable burst length - 32 ?
                w.set_edfe(Edfe::ENABLED);
            });

            // TODO MTU size setting not found for v1 ethernet, check if correct

            // NOTE(unsafe) We got the peripheral singleton, which means that `rcc::init`
            // was called
            let hclk = T::frequency();
            let hclk_mhz = hclk.0 / 1_000_000;
            info!("Hclk: {}", hclk.0);

            // Set the MDC clock frequency in the range 1MHz - 2.5MHz
            let clock_range = match hclk_mhz {
                0..=24 => panic!("Invalid HCLK frequency - should be at least 25 MHz."),
                25..=34 => Cr::CR_20_35,     // Divide by 16
                35..=59 => Cr::CR_35_60,     // Divide by 26
                60..=99 => Cr::CR_60_100,    // Divide by 42
                100..=149 => Cr::CR_100_150, // Divide by 62
                150..=216 => Cr::CR_150_168, // Divide by 102
                _ => {
                    panic!(
                        "HCLK results in MDC clock > 2.5MHz even for the highest CSR clock divider"
                    )
                }
            };

            let pins = [
                ref_clk.map_into(),
                mdio.map_into(),
                mdc.map_into(),
                crs.map_into(),
                rx_d0.map_into(),
                rx_d1.map_into(),
                tx_d0.map_into(),
                tx_d1.map_into(),
                tx_en.map_into(),
            ];

            let mut this = Self {
                _peri: peri,
                pins,
                _phy: phy,
                clock_range,
                phy_addr,
                mac_addr,
                tx: TDesRing::new(&mut queue.tx_desc, &mut queue.tx_buf),
                rx: RDesRing::new(&mut queue.rx_desc, &mut queue.rx_buf),
            };

            fence(Ordering::SeqCst);

            let mac = ETH.ethernet_mac();
            let dma = ETH.ethernet_dma();

            mac.maccr().modify(|w| {
                w.set_re(true);
                w.set_te(true);
            });
            dma.dmaomr().modify(|w| {
                w.set_ftf(Ftf::FLUSH); // flush transmit fifo (queue)
                w.set_st(St::STARTED); // start transmitting channel
                w.set_sr(DmaomrSr::STARTED); // start receiving channel
            });

            this.rx.demand_poll();

            // Enable interrupts
            dma.dmaier().modify(|w| {
                w.set_nise(true);
                w.set_rie(true);
                w.set_tie(true);
            });

            P::phy_reset(&mut this);
            P::phy_init(&mut this);

            interrupt.set_handler(Self::on_interrupt);
            interrupt.enable();

            (this, PTPClock::new::<T>(pps))
        }
    }

    fn on_interrupt(_cx: *mut ()) {
        WAKER.wake();

        // TODO: Check and clear more flags
        unsafe {
            let dma = ETH.ethernet_dma();

            dma.dmasr().modify(|w| {
                w.set_ts(true);
                w.set_rs(true);
                w.set_nis(true);
            });
            // Delay two peripheral's clock
            dma.dmasr().read();
            dma.dmasr().read();
        }
    }
}

unsafe impl<'d, T: Instance, P: PHY> StationManagement for Ethernet<'d, T, P> {
    fn smi_read(&mut self, reg: u8) -> u16 {
        // NOTE(unsafe) These registers aren't used in the interrupt and we have `&mut
        // self`
        unsafe {
            let mac = ETH.ethernet_mac();

            mac.macmiiar().modify(|w| {
                w.set_pa(self.phy_addr);
                w.set_mr(reg);
                w.set_mw(Mw::READ); // read operation
                w.set_cr(self.clock_range);
                w.set_mb(MbProgress::BUSY); // indicate that operation is in
                                            // progress
            });
            while mac.macmiiar().read().mb() == MbProgress::BUSY {}
            mac.macmiidr().read().md()
        }
    }

    fn smi_write(&mut self, reg: u8, val: u16) {
        // NOTE(unsafe) These registers aren't used in the interrupt and we have `&mut
        // self`
        unsafe {
            let mac = ETH.ethernet_mac();

            mac.macmiidr().write(|w| w.set_md(val));
            mac.macmiiar().modify(|w| {
                w.set_pa(self.phy_addr);
                w.set_mr(reg);
                w.set_mw(Mw::WRITE); // write
                w.set_cr(self.clock_range);
                w.set_mb(MbProgress::BUSY);
            });
            while mac.macmiiar().read().mb() == MbProgress::BUSY {}
        }
    }
}

impl<'d, T: Instance, P: PHY> Drop for Ethernet<'d, T, P> {
    fn drop(&mut self) {
        // NOTE(unsafe) We have `&mut self` and the interrupt doesn't use this registers
        unsafe {
            let dma = ETH.ethernet_dma();
            let mac = ETH.ethernet_mac();

            // Disable the TX DMA and wait for any previous transmissions to be completed
            dma.dmaomr().modify(|w| w.set_st(St::STOPPED));

            // Disable MAC transmitter and receiver
            mac.maccr().modify(|w| {
                w.set_re(false);
                w.set_te(false);
            });

            dma.dmaomr().modify(|w| w.set_sr(DmaomrSr::STOPPED));
        }

        // NOTE(unsafe) Exclusive access to the regs
        critical_section::with(|_| unsafe {
            for pin in self.pins.iter_mut() {
                pin.set_as_disconnected();
            }
        })
    }
}

pub trait PPSPin: embassy_stm32::gpio::Pin {
    fn af_num(&self) -> u8;
}

impl PPSPin for embassy_stm32::peripherals::PB5 {
    fn af_num(&self) -> u8 {
        11
    }
}

impl PPSPin for embassy_stm32::peripherals::PG8 {
    fn af_num(&self) -> u8 {
        11
    }
}
