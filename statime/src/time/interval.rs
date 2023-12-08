#[allow(unused_imports)]
use crate::float_polyfill::FloatPolyfill;

/// A log2 representation of seconds used to describe the pacing of events in
/// PTP
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Interval(i8);

impl core::fmt::Debug for Interval {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Interval")
            .field("seconds", &self.as_f64())
            .field("log_base_2", &self.0)
            .finish()
    }
}

impl Interval {
    /// An Interval of one second
    pub const ONE_SECOND: Self = Self(0);

    /// An Interval of two seconds
    pub const TWO_SECONDS: Self = Self(1);

    /// Construct an [`Interval`] from log2 seconds.
    ///
    /// # Example
    /// ```
    /// # use std::time::Duration;
    /// # use statime::time::Interval;
    /// assert_eq!(Interval::from_log_2(2).as_core_duration(), Duration::from_secs(4));
    /// assert_eq!(Interval::from_log_2(-2).as_core_duration(), Duration::from_millis(250));
    /// ```
    pub const fn from_log_2(log_2: i8) -> Self {
        Self(log_2)
    }

    /// Turn `self` into a number of seconds as [`f64`]
    ///
    /// # Example
    /// ```
    /// # use statime::time::Interval;
    /// assert_eq!(Interval::from_log_2(1).seconds(), 2.0);
    /// assert_eq!(Interval::from_log_2(-1).seconds(), 0.5);
    /// ```
    pub fn seconds(self) -> f64 {
        self.as_f64()
    }

    /// Turn this into a [`statime::time::Duration`](`crate::time::Duration`)
    ///
    /// # Example
    /// ```
    /// # use statime::time::{Duration, Interval};
    /// assert_eq!(Interval::from_log_2(3).as_duration(), Duration::from_secs(8));
    /// assert_eq!(Interval::from_log_2(-3).as_duration(), Duration::from_millis(125));
    /// ```
    pub fn as_duration(self) -> super::Duration {
        super::Duration::from_interval(self)
    }

    /// Turn this into a [`core::time::Duration`]
    ///
    /// # Example
    /// ```
    /// # use statime::time::{Interval};
    /// use core::time::Duration;
    /// assert_eq!(Interval::from_log_2(3).as_core_duration(), Duration::from_secs(8));
    /// assert_eq!(Interval::from_log_2(-3).as_core_duration(), Duration::from_millis(125));
    /// ```
    pub fn as_core_duration(self) -> core::time::Duration {
        core::time::Duration::from_secs_f64(self.seconds())
    }

    fn as_f64(self) -> f64 {
        2.0f64.powi(self.0 as i32)
    }

    /// Get the log2 of the numbers of seconds of this [`Interval`]
    ///
    /// # Example
    /// ```
    /// # use statime::time::{Interval};
    /// use core::time::Duration;
    /// assert_eq!(Interval::ONE_SECOND.as_log_2(), 0);
    /// assert_eq!(Interval::TWO_SECONDS.as_log_2(), 1);
    /// ```
    pub fn as_log_2(self) -> i8 {
        self.0
    }
}

impl From<i8> for Interval {
    fn from(value: i8) -> Self {
        Self::from_log_2(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two() {
        assert_eq!(Interval::TWO_SECONDS.as_f64(), 2.0f64)
    }
}
