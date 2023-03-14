//! PTP access and configuration.
//!
//! See [`EthernetPTP`] for a more details.

use embassy_stm32::rcc::Clocks;

pub use pps_pin::PPSPin;
pub use subseconds::{Subseconds, NANOS_PER_SECOND, SUBSECONDS_PER_SECOND, SUBSECONDS_TO_SECONDS};
pub use timestamp::Timestamp;
use {core::task::Poll, futures::task::AtomicWaker};

use crate::clock::stm32_eth::{dma::EthernetDMA, mac::EthernetMAC, peripherals::ETHERNET_PTP};

mod pps_pin;
mod subseconds;
mod timestamp;

/// Access to the IEEE 1508v2 PTP peripheral present on the ethernet peripheral.
///
/// On STM32FXXX's, the PTP peripheral has/uses the following important parts:
/// * HCLK (the chip's high speed clock, configured externally).
/// * The global timestamp (`global_time`, a [`Timestamp`]).
/// * A subsecond increment register (`subsecond_increment`, a [`Subseconds`] with a value of 0 to 255, see [`EthernetPTP::subsecond_increment`]).
/// * An accumulator register (`accumulator`, an [`u32`]).
/// * An addend register (`addend`, an [`u32`], see [`EthernetPTP::addend`] and [`EthernetPTP::set_addend`]).
///
/// To ensure that `global_time` advances at the correct rate, the system performs the following steps:
/// 1. On every clock of HCLK, `addend` is added to `accumulator`.
/// 2. If `accumulator` overflows during step 1, add `subsecond_increment` to `global_time`.
///
/// When a new [`EthernetPTP`] is created, it is assumed that the frequency of HCLK is exactly correct.
/// Using HCLK, values for `subsecond_increment` and `addend` are calculated so that `global_time` represents
/// real-time.
///
/// Subsequently, `addend` can be adjusted to compensate for possible errors in HCLK, using [`EthernetPTP::addend`] and [`EthernetPTP::set_addend`]
///
/// To assess the correctness of the current speed at which `global_time` is running, one can use the
/// following equation:
///
/// ```no_compile
/// clock_ratio = ((2^31 / subsecond_increment) / (HCLK_HZ * (addend / 2^32)))
/// ```
/// Values greater than 1 indicate that the provided `HCLK_HZ` is less than the actual frequency of HCLK, which should
/// be compensated by increasing `addend`. Values less than 1 indicate that the provided `HCLK_HZ` is greater than the
/// actual frequency of HCLK, which should be compensated by decreasing `addend`.
///
/// [`NonZeroU8`]: core::num::NonZeroU8
pub struct EthernetPTP {
    eth_ptp: ETHERNET_PTP,
}

impl EthernetPTP {
    // Calculate the `addend` required for running `global_time` at
    // the correct rate
    const fn calculate_regs(hclk: u32) -> (Subseconds, u32) {
        let half_hclk = hclk / 2;

        // Calculate the closest `subsecond_increment` we can use if we want to update at a
        // frequency of `half_hclk`
        let stssi = Subseconds::nearest_increment(half_hclk);
        let half_rate_subsec_increment_hz = stssi.hertz();

        // Calculate the `addend` required for running `global_time` at
        // the correct rate, given that we increment `global_time` by `stssi` every
        // time `accumulator` overflows.
        let tsa = ((half_rate_subsec_increment_hz as u64 * u32::MAX as u64) / hclk as u64) as u32;
        (stssi, tsa)
    }

    pub(crate) fn new(
        eth_ptp: ETHERNET_PTP,
        clocks: Clocks,
        // Note(_dma): this field exists to ensure that the PTP is not
        // initialized before the DMA. If PTP is started before the DMA,
        // it doesn't work.
        _dma: &EthernetDMA,
    ) -> Self {
        // Mask timestamp interrupt register
        EthernetMAC::mask_timestamp_trigger_interrupt();

        let hclk = clocks.hclk().to_Hz();

        let (stssi, tsa) = Self::calculate_regs(hclk);

        // Setup PTP timestamping in fine mode.
        eth_ptp.ptptscr.write(|w| {
            // Enable snapshots for all frames.
            let w = w.tssarfe().set_bit();

            w.tse().set_bit().tsfcu().set_bit()
        });

        // Set up subsecond increment
        eth_ptp
            .ptpssir
            .write(|w| unsafe { w.stssi().bits(stssi.raw() as u8) });

        let mut me = Self { eth_ptp };

        me.set_addend(tsa);
        me.set_time(Timestamp::new_unchecked(false, 0, 0));

        me
    }

    /// Get the configured subsecond increment.
    pub fn subsecond_increment(&self) -> Subseconds {
        Subseconds::new_unchecked(self.eth_ptp.ptpssir.read().stssi().bits() as u32)
    }

    /// Get the currently configured PTP clock addend.
    pub fn addend(&self) -> u32 {
        self.eth_ptp.ptptsar.read().bits()
    }

    /// Set the PTP clock addend.
    #[inline(always)]
    pub fn set_addend(&mut self, rate: u32) {
        let ptp = &self.eth_ptp;
        ptp.ptptsar.write(|w| unsafe { w.bits(rate) });

        {
            while ptp.ptptscr.read().ttsaru().bit_is_set() {}
            ptp.ptptscr.modify(|_, w| w.ttsaru().set_bit());
            while ptp.ptptscr.read().ttsaru().bit_is_set() {}
        }
    }

    /// Set the current time.
    pub fn set_time(&mut self, time: Timestamp) {
        let ptp = &self.eth_ptp;

        let seconds = time.seconds();
        let subseconds = time.subseconds_signed();

        ptp.ptptshur.write(|w| unsafe { w.bits(seconds) });
        ptp.ptptslur.write(|w| unsafe { w.bits(subseconds) });

        // Initialise timestamp
        while ptp.ptptscr.read().tssti().bit_is_set() {}
        ptp.ptptscr.modify(|_, w| w.tssti().set_bit());
        while ptp.ptptscr.read().tssti().bit_is_set() {}
    }

    /// Add the provided time to the current time, atomically.
    ///
    /// If `time` is negative, it will instead be subtracted from the
    /// system time.
    pub fn update_time(&mut self, time: Timestamp) {
        let ptp = &self.eth_ptp;

        let seconds = time.seconds();
        let subseconds = time.subseconds_signed();

        ptp.ptptshur.write(|w| unsafe { w.bits(seconds) });
        ptp.ptptslur.write(|w| unsafe { w.bits(subseconds) });

        // Add timestamp to global time

        let read_status = || {
            let scr = ptp.ptptscr.read();
            scr.tsstu().bit_is_set() || scr.tssti().bit_is_set()
        };

        while read_status() {}
        ptp.ptptscr.modify(|_, w| w.tsstu().set_bit());
        while ptp.ptptscr.read().tsstu().bit_is_set() {}
    }

    /// Get the current time
    pub fn now() -> Timestamp {
        Self::get_time()
    }

    /// Get the current time.
    pub fn get_time() -> Timestamp {
        let try_read_time = || {
            // SAFETY: we only atomically read registers.
            let eth_ptp = unsafe { &*ETHERNET_PTP::ptr() };

            let seconds = eth_ptp.ptptshr.read().bits();
            let subseconds = eth_ptp.ptptslr.read().bits();
            let seconds_after = eth_ptp.ptptshr.read().bits();

            if seconds == seconds_after {
                Ok(Timestamp::from_parts(seconds, subseconds))
            } else {
                Err(())
            }
        };

        loop {
            if let Ok(res) = try_read_time() {
                return res;
            }
        }
    }

    /// Enable the PPS output on the provided pin.
    pub fn enable_pps<P>(&mut self, pin: P) -> P::Output
    where
        P: PPSPin,
    {
        pin.enable()
    }
}

/// Setting and configuring target time interrupts on the STM32F107 does not
/// make any sense: we can generate the interrupt, but it is impossible to
/// clear the flag as the register required to do so does not exist.
impl EthernetPTP {
    fn waker() -> &'static AtomicWaker {
        static WAKER: AtomicWaker = AtomicWaker::new();
        &WAKER
    }

    /// Configure the target time.
    fn set_target_time(&mut self, timestamp: Timestamp) {
        let (high, low) = (timestamp.seconds(), timestamp.subseconds_signed());
        self.eth_ptp
            .ptptthr
            .write(|w| unsafe { w.ttsh().bits(high) });
        self.eth_ptp
            .ptpttlr
            .write(|w| unsafe { w.ttsl().bits(low) });
    }

    /// Configure the target time interrupt.
    ///
    /// You must call [`EthernetPTP::interrupt_handler`] in the `ETH`
    /// interrupt to detect (and clear) the correct status bits.
    pub fn configure_target_time_interrupt(&mut self, timestamp: Timestamp) {
        self.set_target_time(timestamp);
        self.eth_ptp.ptptscr.modify(|_, w| w.tsite().set_bit());
        EthernetMAC::unmask_timestamp_trigger_interrupt();
    }

    /// Wait until the specified time.
    pub async fn wait_until(&mut self, timestamp: Timestamp) {
        self.configure_target_time_interrupt(timestamp);
        core::future::poll_fn(|ctx| {
            if EthernetPTP::read_and_clear_interrupt_flag() {
                Poll::Ready(())
            } else if EthernetPTP::get_time().raw() >= timestamp.raw() {
                Poll::Ready(())
            } else {
                EthernetPTP::waker().register(ctx.waker());
                Poll::Pending
            }
        })
        .await;
    }

    #[inline(always)]
    fn read_and_clear_interrupt_flag() -> bool {
        let eth_ptp = unsafe { &*ETHERNET_PTP::ptr() };
        eth_ptp.ptptssr.read().tsttr().bit_is_set()
    }

    /// Handle the PTP parts of the `ETH` interrupt.
    ///
    /// Returns a boolean indicating whether or not the interrupt
    /// was caused by a Timestamp trigger and clears the interrupt
    /// flag.
    pub fn interrupt_handler() -> bool {
        // SAFETY: we only perform one atomic read.
        let eth_mac = unsafe { &*crate::clock::stm32_eth::peripherals::ETHERNET_MAC::ptr() };

        let is_tsint = eth_mac.macsr.read().tsts().bit_is_set();
        if is_tsint {
            EthernetMAC::mask_timestamp_trigger_interrupt();
        }

        if let Some(waker) = EthernetPTP::waker().take() {
            waker.wake();
        } else {
            EthernetPTP::read_and_clear_interrupt_flag();
        }

        is_tsint
    }

    /// Configure the PPS output frequency.
    ///
    /// The PPS output frequency becomes `2 ^ pps_freq`. `pps_freq` is
    /// clamped to `[0..31]`.
    pub fn set_pps_freq(&mut self, pps_freq: u8) {
        let pps_freq = pps_freq.max(31);

        // SAFETY: we atomically write to the PTPPPSCR register, which is
        // not read or written to anywhere else. The SVD files are incorrectly
        // saying that the bits in this register are read-only.
        unsafe {
            let ptpppscr = self.eth_ptp.ptpppscr.as_ptr() as *mut u32;
            core::ptr::write_volatile(ptpppscr, pps_freq as u32);
        }
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod test {
    use super::*;

    // Test that we get accurate addend and subsecond_increment values
    // with the provided clock speeds.
    #[test]
    fn hclk_to_regs() {
        for hclk_hz in (25..180).map(|v| v * 1_000_000) {
            let (stssi, tsa) = EthernetPTP::calculate_regs(hclk_hz);

            let stssi = stssi.raw() as f64;
            let tsa = tsa as f64;

            // calculate the clock ratio
            let clock_ratio = (SUBSECONDS_PER_SECOND as f64 / stssi)
                / (hclk_hz as f64 * (tsa / 0xFFFF_FFFFu32 as f64));

            let ppm = (clock_ratio - 1f64) * 1_000_000f64;

            assert!(ppm <= 0.06, "{} at {}", ppm, hclk_hz);
        }
    }
}
