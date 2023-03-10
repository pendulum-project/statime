mod stm32_eth;

use statime::clock::{Clock, Timer};
use statime::datastructures::common::ClockQuality;
use statime::datastructures::datasets::TimePropertiesDS;
use statime::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Stm32Clock;

impl Stm32Clock {
    pub fn new() -> Self {
        Stm32Clock
    }
}

impl Clock for Stm32Clock {
    type Error = Stm32Error;

    fn now(&self) -> Instant {
        todo!()
    }

    fn quality(&self) -> ClockQuality {
        todo!()
    }

    fn adjust(
        &mut self,
        time_offset: Duration,
        frequency_multiplier: f64,
        time_properties_ds: &TimePropertiesDS,
    ) -> Result<(), Self::Error> {
        todo!()
    }
}

pub struct Stm32Timer;

impl Timer for Stm32Timer {
    async fn after(&self, duration: Duration) {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub enum Stm32Error {
    LinuxError(i32),
}
