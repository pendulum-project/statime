use embassy_net::driver::{Clock as NetClock, ScaledPpm};
use statime::{
    Clock as StatimeClock,
    config::TimePropertiesDS,
    time::{Duration, Time},
};

use crate::time_from;

/// A Statime clock backed by an Embassy network driver's clock.
#[derive(Debug)]
pub struct EmbassyClock<T> {
    inner: T,
}

impl<T> EmbassyClock<T> {
    /// Wrap an initialized Embassy network clock.
    pub const fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Borrow the underlying network clock.
    pub const fn inner(&self) -> &T {
        &self.inner
    }

    /// Mutably borrow the underlying network clock.
    pub const fn inner_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Unwrap the underlying network clock.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

/// Error returned by [`EmbassyClock`].
#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum EmbassyClockError<E> {
    /// The network driver's clock rejected an operation.
    Clock(E),
    /// Statime requested a frequency adjustment that is not finite.
    NonFiniteFrequency,
}

impl<E: core::error::Error> core::fmt::Display for EmbassyClockError<E> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Clock(error) => write!(formatter, "network clock: {error}"),
            Self::NonFiniteFrequency => formatter.write_str("non-finite frequency adjustment"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for EmbassyClockError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Clock(error) => Some(error),
            Self::NonFiniteFrequency => None,
        }
    }
}

impl<T: NetClock> StatimeClock for EmbassyClock<T> {
    type Error = EmbassyClockError<T::Error>;

    fn now(&self) -> Time {
        time_from(self.inner.now())
    }

    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
        // Embassy clocks step in whole nanoseconds; preserve Statime's lossy
        // conversion and clamp values outside the driver's signed range.
        let nanos = offset
            .nanos_rounded()
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        self.inner
            .step(nanos)
            .map(time_from)
            .map_err(EmbassyClockError::Clock)
    }

    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
        if !ppm.is_finite() {
            return Err(EmbassyClockError::NonFiniteFrequency);
        }

        // The cast saturates. Sub-LSB truncation is integrated away by the
        // servo instead of requiring a separate floating-point rounding step.
        let adjustment = ScaledPpm::from_raw((ppm * (1i32 << 16) as f64) as i32);
        self.inner
            .set_frequency(adjustment)
            .map(time_from)
            .map_err(EmbassyClockError::Clock)
    }

    fn set_properties(
        &mut self,
        _time_properties_ds: &TimePropertiesDS,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use embassy_net::driver::Timestamp;

    use super::*;

    #[derive(Debug, Default)]
    struct TestClock {
        fail: bool,
        step: i64,
        frequency: ScaledPpm,
    }

    #[derive(Debug)]
    #[cfg_attr(feature = "defmt", derive(defmt::Format))]
    struct TestError;

    impl core::fmt::Display for TestError {
        fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter.write_str("test clock failure")
        }
    }

    impl core::error::Error for TestError {}

    impl NetClock for TestClock {
        type Error = TestError;

        fn now(&self) -> Timestamp {
            Timestamp::from_seconds_and_nanos(1, 2)
        }

        fn step(&mut self, offset_nanos: i64) -> Result<Timestamp, Self::Error> {
            if self.fail {
                return Err(TestError);
            }
            self.step = offset_nanos;
            Ok(self.now())
        }

        fn set_frequency(&mut self, adjustment: ScaledPpm) -> Result<Timestamp, Self::Error> {
            if self.fail {
                return Err(TestError);
            }
            self.frequency = adjustment;
            Ok(self.now())
        }
    }

    #[test]
    fn converts_statime_adjustments_to_driver_units() {
        let mut clock = EmbassyClock::new(TestClock::default());

        clock.step_clock(Duration::from_fixed_nanos(1.6)).unwrap();
        assert_eq!(clock.inner().step, 1);

        clock.set_frequency(-0.25).unwrap();
        assert_eq!(clock.inner().frequency, ScaledPpm::from_raw(-16_384));

        clock.set_frequency(f64::MAX).unwrap();
        assert_eq!(clock.inner().frequency, ScaledPpm::from_raw(i32::MAX));
    }

    #[test]
    fn clamps_steps_to_the_driver_range() {
        let mut clock = EmbassyClock::new(TestClock::default());

        clock
            .step_clock(Duration::from_fixed_nanos(i64::MAX as i128 + 1))
            .unwrap();
        assert_eq!(clock.inner().step, i64::MAX);

        clock
            .step_clock(Duration::from_fixed_nanos(i64::MIN as i128 - 1))
            .unwrap();
        assert_eq!(clock.inner().step, i64::MIN);
    }

    #[test]
    fn rejects_non_finite_frequency() {
        let mut clock = EmbassyClock::new(TestClock::default());

        assert!(matches!(
            clock.set_frequency(f64::NAN),
            Err(EmbassyClockError::NonFiniteFrequency)
        ));
        assert_eq!(clock.inner().frequency, ScaledPpm::ZERO);
    }

    #[test]
    fn propagates_driver_errors() {
        #[cfg(feature = "defmt")]
        fn assert_format<T: defmt::Format>() {}
        #[cfg(feature = "defmt")]
        assert_format::<EmbassyClockError<TestError>>();

        let mut clock = EmbassyClock::new(TestClock {
            fail: true,
            ..Default::default()
        });

        assert!(matches!(
            clock.set_frequency(0.0),
            Err(EmbassyClockError::Clock(TestError))
        ));
    }
}
