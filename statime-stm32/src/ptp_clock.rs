use core::cell::RefCell;

use az::{Cast, SaturatingCast};
use critical_section::Mutex;
use fixed::types::{I33F31, U96F32};
use stm32_eth::ptp::{EthernetPTP, Timestamp};

pub struct PtpClock {
    inner: Mutex<RefCell<EthernetPTP>>,
    addend_starter_frequency: f64,
}

impl PtpClock {
    pub fn new(ethernet_ptp: EthernetPTP) -> Self {
        let start_addend = ethernet_ptp.addend();
        let addend_starter_frequency = u32::MAX as f64 / start_addend as f64;

        Self {
            inner: Mutex::new(RefCell::new(ethernet_ptp)),
            addend_starter_frequency,
        }
    }

    pub fn access<R>(&self, f: impl FnOnce(&mut EthernetPTP) -> R) -> R {
        critical_section::with(|cs| f(&mut self.inner.borrow_ref_mut(cs)))
    }
}

impl statime::Clock for &PtpClock {
    type Error = core::convert::Infallible;

    fn now(&self) -> statime::Time {
        let seconds: U96F32 = I33F31::from_bits(EthernetPTP::get_time().raw()).cast();
        let nanos = seconds * 1_000_000_000u128;
        statime::Time::from_fixed_nanos(nanos)
    }

    fn step_clock(&mut self, offset: statime::Duration) -> Result<statime::Time, Self::Error> {
        let seconds = offset.nanos() / 1_000_000_000i128;
        let seconds: I33F31 = seconds.cast();

        self.access(|clock| {
            clock.update_time(Timestamp::new_raw(seconds.to_bits()));
        });

        Ok(self.now())
    }

    fn set_frequency(&mut self, freq: f64) -> Result<statime::Time, Self::Error> {
        let total_frequency = self.addend_starter_frequency * freq;
        let addend = u32::MAX as f64 / total_frequency;

        self.access(|clock| {
            clock.set_addend(addend.saturating_cast());
        });

        Ok(self.now())
    }

    fn set_properties(
        &mut self,
        _time_properties_ds: &statime::TimePropertiesDS,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
