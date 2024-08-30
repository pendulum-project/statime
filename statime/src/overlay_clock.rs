use crate::{time::Duration, time::Time, Clock};

/// An overlay over other, read-only clock, frequency-locked to it.
/// In other words, a virtual clock which can be tuned in software without affecting
/// the underlying system or hardware clock.
#[derive(Debug)]
pub struct OverlayClock<C: Clock> {
    roclock: C,
    last_sync: Time,
    shift: Duration,
    freq_scale_ppm_diff: f64,
}

impl<C: Clock> OverlayClock<C> {
    /// Creates new OverlayClock based on given clock
    pub fn new(underlying_clock: C) -> Self {
        let now = underlying_clock.now();
        Self {
            roclock: underlying_clock,
            last_sync: now,
            shift: Duration::from_fixed_nanos(0),
            freq_scale_ppm_diff: 0.0,
        }
    }

    /// Converts (shifts and scales) `Time` in underlying clock's timescale to overlay clock timescale
    pub fn time_from_underlying(&self, roclock_time: Time) -> Time {
        let elapsed = roclock_time - self.last_sync;
        let corr = elapsed * self.freq_scale_ppm_diff / 1_000_000;

        roclock_time + self.shift + corr
        // equals self.last_sync + self.shift + elapsed + corr
    }

    /// Returns reference to underlying clock
    pub fn underlying(&self) -> &C {
        &self.roclock
    }
}

impl<C: Clock> Clock for OverlayClock<C> {
    type Error = C::Error;
    fn now(&self) -> Time {
        self.time_from_underlying(self.roclock.now())
    }
    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
        // save current shift:
        let now_roclock = self.roclock.now();
        let now_local = self.time_from_underlying(now_roclock);
        self.shift = now_local - now_roclock;
        self.last_sync = now_roclock;

        self.freq_scale_ppm_diff = ppm;
        debug_assert_eq!(self.time_from_underlying(self.last_sync), now_local);
        Ok(now_local)
    }
    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
        self.last_sync = self.roclock.now();
        let multiplier = 1_000_000f64 + self.freq_scale_ppm_diff;
        let reciprocal = 1_000_000f64 / multiplier;
        self.shift += offset * reciprocal;
        Ok(self.time_from_underlying(self.last_sync))
    }
    fn set_properties(
        &mut self,
        _time_properties_ds: &crate::config::TimePropertiesDS,
    ) -> Result<(), Self::Error> {
        // we can ignore the properies - they are just metadata
        Ok(())
    }
}
