//! Implementation of the abstract clock for the linux platform

use fixed::traits::LossyInto;
pub use raw::RawLinuxClock;
use statime::{
    clock::{Clock, TimeProperties, Timer},
    datastructures::common::ClockQuality,
    time::{Duration, Instant},
};

mod raw;
mod timex;

#[derive(Debug, Clone)]
pub enum Error {
    LinuxError(i32),
}

pub struct LinuxClock {
    clock: RawLinuxClock,
}

impl LinuxClock {
    pub fn new(clock: RawLinuxClock) -> Self {
        Self {
            clock: clock.clone(),
        }
    }
}

impl Clock for LinuxClock {
    type E = Error;

    fn now(&self) -> Instant {
        self.clock.get_time().unwrap()
    }

    fn quality(&self) -> ClockQuality {
        self.clock.quality()
    }

    fn adjust(
        &mut self,
        time_offset: Duration,
        frequency_multiplier: f64,
        time_properties: TimeProperties,
    ) -> Result<bool, Self::E> {
        if let TimeProperties::PtpTime {
            leap_61, leap_59, ..
        } = time_properties
        {
            self.clock
                .set_leap_seconds(leap_61, leap_59)
                .map_err(Error::LinuxError)?;
        }

        let time_offset_float: f64 = time_offset.nanos().lossy_into();
        let adjust_result = self
            .clock
            .adjust_clock(time_offset_float / 1e9, frequency_multiplier);

        match adjust_result {
            Ok(_) => Ok(true),
            Err(e) => Err(Error::LinuxError(e)),
        }
    }
}

pub struct LinuxTimer;

impl Timer for LinuxTimer {
    async fn after(&self, duration: Duration) {
        tokio::time::sleep(duration.into()).await
    }
}

pub fn timespec_into_instant(spec: nix::sys::time::TimeSpec) -> Instant {
    Instant::from_fixed_nanos(spec.tv_sec() as i128 * 1_000_000_000i128 + spec.tv_nsec() as i128)
}
