//! Definitions and implementations of the abstract clock types

use crate::{
    datastructures::common::ClockQuality,
    time::{Duration, Instant},
};

/// Clock type for use in the PTP stack
pub trait Clock {
    type E: std::fmt::Debug;

    /// Get the current time of the clock
    fn now(&self) -> Instant;

    /// Get the quality of the clock
    fn quality(&self) -> ClockQuality;

    /// Adjust the clock with the given time offset and frequency multiplier.
    /// The adjustment is based on the given time properties.
    ///
    /// The adjustment that is actually being done to the clock doesn't have to be exactly what is being given.
    /// The clock can (and should) do some filtering.
    fn adjust(
        &mut self,
        time_offset: Duration,
        frequency_multiplier: f64,
        time_properties: TimeProperties,
    ) -> Result<bool, Self::E>;
}

/// A timer let's you get the current time and wait for durations
pub trait Timer {
    /// Wait for the given amount of time
    async fn after(&self, duration: Duration);
}

#[derive(Debug, Clone, Copy)]
pub enum TimeProperties {
    /// The time is synchronized as a UTC time
    PtpTime {
        /// The amount of seconds the time is away from UTC
        current_utc_offset: Option<u16>,
        /// Indicates that the last minute of this day will have 61 seconds
        leap_61: bool,
        /// Indicates that the last minute of this day will have 59 seconds
        leap_59: bool,
        /// Indicates that the time is traceable to the primary source.
        /// This may have an effect on how the time is filtered.
        time_traceable: bool,
        /// Indicates that the frequency is traceable to the primary source.
        /// This may have an effect on how the frequency is filtered.
        frequency_traceable: bool,
    },
    /// The time is synchronized with an arbitrary start point
    ArbitraryTime {
        /// Indicates that the time is traceable to the primary source.
        /// This may have an effect on how the time is filtered.
        time_traceable: bool,
        /// Indicates that the frequency is traceable to the primary source.
        /// This may have an effect on how the frequency is filtered.
        frequency_traceable: bool,
    },
}

impl TimeProperties {
    /// Returns `true` if the time properties is [`PtpTime`].
    ///
    /// [`PtpTime`]: TimeProperties::PtpTime
    pub fn is_ptp_time(&self) -> bool {
        matches!(self, Self::PtpTime { .. })
    }

    /// Returns `true` if the time properties is [`ArbitraryTime`].
    ///
    /// [`ArbitraryTime`]: TimeProperties::ArbitraryTime
    pub fn is_arbitrary_time(&self) -> bool {
        matches!(self, Self::ArbitraryTime { .. })
    }

    pub fn time_traceable(&self) -> bool {
        match self {
            TimeProperties::PtpTime { time_traceable, .. } => *time_traceable,
            TimeProperties::ArbitraryTime { time_traceable, .. } => *time_traceable,
        }
    }

    pub fn frequency_traceable(&self) -> bool {
        match self {
            TimeProperties::PtpTime {
                frequency_traceable,
                ..
            } => *frequency_traceable,
            TimeProperties::ArbitraryTime {
                frequency_traceable,
                ..
            } => *frequency_traceable,
        }
    }
}
