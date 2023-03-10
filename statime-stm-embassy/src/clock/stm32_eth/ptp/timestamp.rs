use crate::clock::stm32_eth::dma::desc::Descriptor;

use super::{Subseconds, NANOS_PER_SECOND};

/// A timestamp produced by the PTP periperhal
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp(i64);

#[cfg(feature = "defmt")]
impl defmt::Format for Timestamp {
    fn format(&self, fmt: defmt::Formatter) {
        if self.is_positive() {
            defmt::write!(fmt, "{}.{:09}", self.seconds(), self.nanos());
        } else {
            defmt::write!(fmt, "-{}.{:09}", self.seconds(), self.nanos());
        }
    }
}

impl Timestamp {
    // The bit that represents the signedness of the timestamp in the
    // subseconds value.
    const SIGN_BIT: u32 = 0x8000_0000;

    /// Create a new [`Timestamp`]
    pub const fn new(negative: bool, seconds: u32, subseconds: Subseconds) -> Self {
        Self::new_unchecked(negative, seconds, subseconds.raw())
    }

    /// Create a new [`Timestamp`] from the given raw value.
    pub const fn new_raw(value: i64) -> Self {
        Self(value)
    }

    /// Get the raw value of this [`Timestamp`]
    pub const fn raw(&self) -> i64 {
        self.0
    }

    /// Check whether this timestamp is negative or not.
    pub const fn is_negative(&self) -> bool {
        self.0.is_negative()
    }

    /// Check whether this timestamp is positive or not.
    pub const fn is_positive(&self) -> bool {
        !self.is_negative()
    }

    pub(crate) const fn new_unchecked(negative: bool, seconds: u32, subseconds: u32) -> Self {
        let seconds: i64 = (seconds as i64) << 31;
        let subseconds: i64 = subseconds as i64;

        let mut total = seconds + subseconds;

        if negative {
            total = -total;
        };

        Self(total)
    }

    /// Get the second component of this timestamp
    pub const fn seconds(&self) -> u32 {
        (self.0.abs() >> 31) as u32
    }

    /// Get the raw subsecond value of this timestamp.
    pub const fn subseconds(&self) -> Subseconds {
        Subseconds::new_unchecked(self.0.abs() as u32 & Subseconds::MAX_VALUE)
    }

    /// Get the signed subsecond value of this timestamp.
    ///
    /// Note that this is _not_ an i32: it is, technically,
    /// a u31 with a leading sign bit.
    pub const fn subseconds_signed(&self) -> u32 {
        let mut subseconds = self.subseconds().raw();

        if self.0.is_negative() {
            subseconds |= Self::SIGN_BIT;
        }

        subseconds
    }

    /// Get the nanosecond component of this timestamp
    pub const fn nanos(&self) -> u32 {
        self.subseconds().nanos()
    }

    /// Get the total amount of nanoseconds in this [`Timestamp`].
    ///
    /// Example:
    /// ```rust
    /// # use stm32_eth::ptp::{Subseconds, Timestamp};
    /// let timestamp = Timestamp::new(false, 500, Subseconds::new_from_nanos(500_000).unwrap());
    /// assert_eq!(timestamp.total_nanos(), 500 * 1_000_000_000 + 500_000);
    ///
    ///
    /// let timestamp_neg = Timestamp::new(true, 500, Subseconds::new_from_nanos(500_000).unwrap());
    /// assert_eq!(timestamp_neg.total_nanos(), -1 * (500 * 1_000_000_000 + 500_000));
    /// ```
    pub const fn total_nanos(&self) -> i64 {
        let nanos = self.seconds() as i64 * NANOS_PER_SECOND as i64 + self.nanos() as i64;

        if self.is_positive() {
            nanos
        } else {
            -nanos
        }
    }

    /// Create a new timestamp from the provided register values.
    pub const fn from_parts(high: u32, low: u32) -> Timestamp {
        let negative = (low & Self::SIGN_BIT) == Self::SIGN_BIT;
        let subseconds = low & !(Self::SIGN_BIT);

        Timestamp::new_unchecked(negative, high, subseconds)
    }

    /// Create a timestamp from the given descriptor
    pub fn from_descriptor(desc: &Descriptor) -> Option<Self> {
        #[cfg(not(feature = "stm32f1xx-hal"))]
        {
            let (high, low) = { (desc.read(7), desc.read(6)) };
            Some(Self::from_parts(high, low))
        }

        #[cfg(feature = "stm32f1xx-hal")]
        {
            let (high, low) = { (desc.read(3), desc.read(2)) };

            // The timestamp registers are written to all-ones if
            // timestamping was no succesfull
            if high == 0xFFFF_FFFF && low == 0xFFFF_FFFF {
                None
            } else {
                Some(Self::from_parts(high, low))
            }
        }
    }
}

impl core::ops::Add<Timestamp> for Timestamp {
    type Output = Self;

    fn add(self, rhs: Timestamp) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl core::ops::AddAssign<Timestamp> for Timestamp {
    fn add_assign(&mut self, rhs: Timestamp) {
        self.0 += rhs.0;
    }
}

impl core::ops::Sub<Timestamp> for Timestamp {
    type Output = Self;

    fn sub(self, rhs: Timestamp) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl core::ops::SubAssign<Timestamp> for Timestamp {
    fn sub_assign(&mut self, rhs: Timestamp) {
        self.0 -= rhs.0
    }
}

#[cfg(all(test, not(target_os = "none")))]
mod test {
    use crate::clock::stm32_eth::ptp::SUBSECONDS_PER_SECOND;

    use super::{Subseconds, Timestamp};

    fn subs(val: u32) -> Subseconds {
        Subseconds::new(val).unwrap()
    }

    #[test]
    fn timestamp_add() {
        let one = Timestamp::new(false, 1, subs(1));
        let one_big = Timestamp::new(false, 1, subs(SUBSECONDS_PER_SECOND - 1));
        let two = Timestamp::new(false, 2, subs(2));
        let three = Timestamp::new(false, 3, subs(3));

        let one_neg = Timestamp::new(true, 1, subs(1));
        let one_big_neg = Timestamp::new(true, 1, subs(SUBSECONDS_PER_SECOND - 1));
        let two_neg = Timestamp::new(true, 2, subs(2));
        let three_neg = Timestamp::new(true, 3, subs(3));

        let one_minus_two = Timestamp::new(true, 1, subs(1));
        let one_big_plus_two = Timestamp::new(false, 4, subs(0));
        let two_minus_one_big = Timestamp::new(false, 0, subs(4));
        let one_big_neg_plus_two_neg = Timestamp::new(true, 4, subs(0));

        // +self + +rhs
        assert_eq!(one + two, three);
        assert_eq!(two + one, three);
        assert_eq!(one_big + two, one_big_plus_two);
        assert_eq!(two + one_big, one_big_plus_two);

        // +self + -rhs
        assert_eq!(one + two_neg, one_minus_two);
        assert_eq!(two + one_big_neg, two_minus_one_big);

        // -self + rhs
        assert_eq!(one_neg + two, one);
        assert_eq!(two + one_neg, one);

        // -self + -rhs
        assert_eq!(one_neg + two_neg, three_neg);
        assert_eq!(two_neg + one_neg, three_neg);
        assert_eq!(one_big_neg + two_neg, one_big_neg_plus_two_neg);
        assert_eq!(two_neg + one_big_neg, one_big_neg_plus_two_neg);
    }

    #[test]
    fn timestamp_sub() {
        let one = Timestamp::new(false, 1, subs(1));
        let one_big = Timestamp::new(false, 1, subs(SUBSECONDS_PER_SECOND - 1));
        let two = Timestamp::new(false, 2, subs(2));
        let three = Timestamp::new(false, 3, subs(3));

        let one_neg = Timestamp::new(true, 1, subs(1));
        let two_neg = Timestamp::new(true, 2, subs(2));
        let three_neg = Timestamp::new(true, 3, subs(3));

        let one_minus_two = Timestamp::new(true, 1, subs(1));
        let one_minus_one_big = Timestamp::new(true, 0, subs(SUBSECONDS_PER_SECOND - 2));

        assert_eq!(one - one_big, one_minus_one_big);

        // +self - +rhs
        assert_eq!(two - one, one);
        assert_eq!(one - two, one_minus_two);

        // +self - -rhs
        assert_eq!(two - one_neg, three);
        assert_eq!(one_neg - two, three_neg);

        // -self - +rhs
        assert_eq!(one_neg - two, three_neg);
        assert_eq!(two - one_neg, three);

        // -self - -rhs
        assert_eq!(one_neg - two_neg, one);
        assert_eq!(two_neg - one_neg, one_minus_two);
    }
}
