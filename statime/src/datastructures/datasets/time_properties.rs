use crate::datastructures::common::TimeSource;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TimePropertiesDS {
    pub current_utc_offset: i16,
    pub current_utc_offset_valid: bool,
    pub leap59: bool,
    pub leap61: bool,
    pub time_traceable: bool,
    pub frequency_traceable: bool,
    pub ptp_timescale: bool,
    pub time_source: TimeSource,
}

impl TimePropertiesDS {
    pub fn new_ptp(
        current_utc_offset: i16,
        current_utc_offset_valid: bool,
        leap59: bool,
        leap61: bool,
        time_traceable: bool,
        frequency_traceable: bool,
        time_source: TimeSource,
    ) -> Self {
        TimePropertiesDS {
            current_utc_offset,
            current_utc_offset_valid,
            leap59,
            leap61,
            time_traceable,
            frequency_traceable,
            ptp_timescale: true,
            time_source,
        }
    }

    pub fn new_arbitrary(
        time_traceable: bool,
        frequency_traceable: bool,
        time_source: TimeSource,
    ) -> Self {
        TimePropertiesDS {
            current_utc_offset: 0,
            current_utc_offset_valid: false,
            leap59: false,
            leap61: false,
            time_traceable,
            frequency_traceable,
            ptp_timescale: false,
            time_source,
        }
    }
}