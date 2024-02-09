use crate::datastructures::common::{LeapIndicator, TimeSource};

/// A concrete implementation of the PTP Time Properties dataset
///
/// This dataset describes the timescale currently in use, as well as any
/// upcoming leap seconds on that timescale.
///
/// For more details see *IEEE1588-2019 section 8.2.4*.
#[derive(Default, Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimePropertiesDS {
    /// The offset off UTC time compared to TAI time in seconds.
    pub current_utc_offset: Option<i16>,
    /// Describes upcoming leap seconds.
    pub leap_indicator: LeapIndicator,
    /// Wheter the timescale is tracable to a primary reference
    pub time_traceable: bool,
    /// Wheter the frequence determining the timescale is tracable to a primary
    /// reference. True when the timescale is PTP, false when the timescale is
    /// ARB.
    pub frequency_traceable: bool,
    /// Wheter the timescale of the Grandmaster PTP Instance is PTP.
    pub ptp_timescale: bool,
    /// The time source used by the Grandmaster PTP instance.
    pub time_source: TimeSource,
}

impl TimePropertiesDS {
    /// Create a Time Properties data set for the PTP timescale.
    ///
    /// This creates a dataset for the default PTP timescale, which is UTC
    /// seconds since the PTP epoch excluding leap seconds. The traceability
    /// properties indicate whether the current clock time and frequency can be
    /// traced back to an internationally recognized standard in the metrology
    /// sense of the word. When in doubt, just set these to false.
    pub fn new_ptp_time(
        current_utc_offset: Option<i16>,
        leap_indicator: LeapIndicator,
        time_traceable: bool,
        frequency_traceable: bool,
        time_source: TimeSource,
    ) -> Self {
        TimePropertiesDS {
            current_utc_offset,
            leap_indicator,
            time_traceable,
            frequency_traceable,
            ptp_timescale: true,
            time_source,
        }
    }

    /// Create a Time Properties data set for an Arbitrary timescale
    ///
    /// The arbitrary timescale can be used when wanting to synchronize multiple
    /// computers using PTP to a timescale that is unrelated to UTC. The
    /// traceability properties indicate whether the current clock time and
    /// frequency can be traced back to an internationally recognized standard
    /// in the metrology sense of the word. When in doubt, just set these to
    /// false.
    pub fn new_arbitrary_time(
        time_traceable: bool,
        frequency_traceable: bool,
        time_source: TimeSource,
    ) -> Self {
        TimePropertiesDS {
            current_utc_offset: None,
            leap_indicator: LeapIndicator::NoLeap,
            time_traceable,
            frequency_traceable,
            ptp_timescale: false,
            time_source,
        }
    }

    /// Is this timescale a ptp (utc-derived) timescale?
    pub fn is_ptp(&self) -> bool {
        self.ptp_timescale
    }

    /// Information on upcoming leap seconds
    pub fn leap_indicator(&self) -> LeapIndicator {
        self.leap_indicator
    }

    /// Current offset to UTC caused by leap seconds
    ///
    /// Returns `None` if this time scale is not referenced to UTC
    pub fn utc_offset(&self) -> Option<i16> {
        self.current_utc_offset
    }
}
