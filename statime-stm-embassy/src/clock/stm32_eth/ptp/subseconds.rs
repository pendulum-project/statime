/// The amount of nanoseconds per second.
pub const NANOS_PER_SECOND: u32 = 1_000_000_000;

/// The amount of subseconds per second.
pub const SUBSECONDS_PER_SECOND: u32 = 0x7FFF_FFFF;

/// The ratio to use to convert subseconds to seconds.
pub const SUBSECONDS_TO_SECONDS: f32 = 1.0 / (SUBSECONDS_PER_SECOND as f32);

const NS_PER_S: u64 = NANOS_PER_SECOND as u64;
const SUBS_PER_S: u64 = SUBSECONDS_PER_SECOND as u64;

/// A subsecond value as produced by the PTP peripheral
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Subseconds(u32);

impl Subseconds {
    /// The maximum possible value for [`Subseconds`]
    pub const MAX_VALUE: u32 = SUBSECONDS_PER_SECOND;

    /// The maximum possible [`Subseconds`]
    pub const MAX: Self = Self(SUBSECONDS_PER_SECOND);

    /// Zero [`Subseconds`]
    pub const ZERO: Self = Self(0);

    /// Create a new [`Subseconds`] from the provided value.
    ///
    /// The returned [`Subseconds`] represents a time of `value / 2^31` seconds.
    ///
    /// To obtain that representation in nanoseconds, see [`Subseconds::nanos`].
    ///
    /// To approximate a [`Subseconds`] from nanoseconds, see [`Subseconds::new_from_nanos`].
    ///
    /// Returns `None` if `value > SUBSECONDS_PER_SECOND`. (See [`SUBSECONDS_PER_SECOND`]).
    pub const fn new(value: u32) -> Option<Self> {
        if value > SUBSECONDS_PER_SECOND as u32 {
            None
        } else {
            Some(Self(value))
        }
    }

    /// Create a new [`Subseconds`] from the provided value, without verifying that `value`
    /// is less than or equal to [`Self::MAX_VALUE`]).
    ///
    /// The returned [`Subseconds`] represents a time of `value / 2^31` seconds.
    ///
    /// To obtain that representation in nanoseconds, see [`Subseconds::nanos`].
    ///
    /// To approximate a [`Subseconds`] from nanoseconds, see [`Subseconds::new_from_nanos`]
    pub(crate) const fn new_unchecked(value: u32) -> Self {
        Self(value)
    }

    /// Create a new [`Subseconds`] from the given amount of nanoseconds,
    /// using a round-to-nearest method.
    ///
    /// Returns [`None`] if `nanos >= NANOS_PER_SECOND`. (See [`NANOS_PER_SECOND`])
    pub const fn new_from_nanos(nanos: u32) -> Option<Self> {
        if nanos >= NANOS_PER_SECOND as u32 {
            return None;
        }

        let subseconds = ((nanos as u64 * SUBS_PER_S) + (NS_PER_S / 2)) / NS_PER_S;

        Some(Subseconds::new_unchecked(subseconds as u32))
    }

    /// Convert this [`Subseconds`] to nanoseconds, using a round-to-nearest method.
    pub const fn nanos(&self) -> u32 {
        let nanos = ((self.0 as u64 * NS_PER_S) + (SUBS_PER_S / 2)) / SUBS_PER_S;

        nanos as u32
    }

    /// Get the raw value of this [`Subseconds`]
    pub const fn raw(&self) -> u32 {
        self.0
    }

    /// Convert this [`Subseconds`] to Hertz
    pub(crate) const fn hertz(&self) -> u32 {
        SUBSECONDS_PER_SECOND as u32 / self.0
    }

    pub(crate) const fn nearest_increment(input_clk_hz: u32) -> Subseconds {
        let hclk_half_subs = (SUBSECONDS_PER_SECOND as u32 + (input_clk_hz / 2)) / input_clk_hz;

        Self::new_unchecked(hclk_half_subs)
    }
}

impl core::ops::Add<Subseconds> for Subseconds {
    type Output = Self;

    fn add(self, rhs: Subseconds) -> Self::Output {
        Self(self.0.wrapping_add(rhs.0) % (SUBSECONDS_PER_SECOND + 1))
    }
}

impl core::ops::AddAssign<Subseconds> for Subseconds {
    fn add_assign(&mut self, rhs: Subseconds) {
        *self = *self + rhs;
    }
}

impl core::ops::Sub<Subseconds> for Subseconds {
    type Output = Self;

    fn sub(self, rhs: Subseconds) -> Self::Output {
        Self(self.0.wrapping_sub(rhs.0) % (SUBSECONDS_PER_SECOND + 1))
    }
}

impl core::ops::SubAssign<Subseconds> for Subseconds {
    fn sub_assign(&mut self, rhs: Subseconds) {
        *self = *self - rhs;
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod test {

    use super::*;

    // Assert that values produced by [`Subseconds::nearest_increment`] for some
    // valid frequencies are within the correct span for `stssi`
    #[test]
    fn correct_subsecond_increment() {
        for i in (25_000..180_000).map(|v| v * 1_000) {
            let subs = Subseconds::nearest_increment(i).raw();
            assert!(subs > 0 && subs <= 255);
        }
    }

    #[test]
    fn from_nanos() {
        for i in [0, 1, 2, 3, NANOS_PER_SECOND - 1] {
            let subseconds = Subseconds::new_from_nanos(i).unwrap();
            assert!(subseconds.raw() < SUBSECONDS_PER_SECOND);
        }

        assert!(Subseconds::new_from_nanos(NANOS_PER_SECOND).is_none());
        assert!(Subseconds::new_from_nanos(u32::MAX).is_none());
    }

    #[test]
    fn subsecond_math() {
        let one = Subseconds::new(1).unwrap();
        let two = Subseconds::new(2).unwrap();
        let three = Subseconds::new(3).unwrap();
        let max = Subseconds::new(SUBSECONDS_PER_SECOND).unwrap();
        let zero = Subseconds::new(0).unwrap();

        assert_eq!(one + two, three);
        assert_eq!(two - one, one);

        assert_eq!(one - max + max, one);
        assert_eq!(one - two, max);
        assert_eq!(one + max, zero);
        assert_eq!(two + max, one);
    }
}
