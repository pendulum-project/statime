//! Serializable implementations of datastructures to be used for observability
#![allow(missing_docs)]
use crate::{
    config::{ClockIdentity, ClockQuality, LeapIndicator, InternalTimePropertiesDS, TimeSource},
    datastructures::{
        common::PortIdentity,
        datasets::{InternalCurrentDS, InternalDefaultDS, InternalParentDS},
    },
};

/// Observable version of the InstanceState struct
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableInstanceState {
    pub default_ds: ObservableDefaultDS,
    pub current_ds: ObservableCurrentDS,
    pub parent_ds: ObservableParentDS,
    pub time_properties_ds: ObservableTimePropertiesDS,
}

/// Observable version of the DefaultDS struct
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableDefaultDS {
    pub clock_identity: crate::config::ClockIdentity,
    pub number_ports: u16,
    pub clock_quality: crate::config::ClockQuality,
    pub priority_1: u8,
    pub priority_2: u8,
    pub domain_number: u8,
    pub slave_only: bool,
    pub sdo_id: crate::config::SdoId,
}

impl From<InternalDefaultDS> for ObservableDefaultDS {
    fn from(v: InternalDefaultDS) -> Self {
        Self {
            clock_identity: v.clock_identity,
            number_ports: v.number_ports,
            clock_quality: v.clock_quality,
            priority_1: v.priority_1,
            priority_2: v.priority_2,
            domain_number: v.domain_number,
            slave_only: v.slave_only,
            sdo_id: v.sdo_id,
        }
    }
}

#[derive(Debug, Default, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableCurrentDS {
    pub steps_removed: u16,
    pub offset_from_master: i128, // rounded nanos
    pub mean_delay: i128,         // rounded nanos
}

impl From<InternalCurrentDS> for ObservableCurrentDS {
    fn from(v: InternalCurrentDS) -> Self {
        Self {
            steps_removed: v.steps_removed,
            offset_from_master: v.offset_from_master.nanos_rounded(),
            mean_delay: v.mean_delay.nanos_rounded(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableParentDS {
    pub parent_port_identity: PortIdentity,
    pub parent_stats: bool,
    pub observed_parent_offset_scaled_log_variance: u16,
    pub observed_parent_clock_phase_change_rate: u32,
    pub grandmaster_identity: ClockIdentity,
    pub grandmaster_clock_quality: ClockQuality,
    pub grandmaster_priority_1: u8,
    pub grandmaster_priority_2: u8,
}

impl From<InternalParentDS> for ObservableParentDS {
    fn from(v: InternalParentDS) -> Self {
        Self {
            parent_port_identity: v.parent_port_identity,
            parent_stats: v.parent_stats,
            observed_parent_offset_scaled_log_variance: v
                .observed_parent_offset_scaled_log_variance,
            observed_parent_clock_phase_change_rate: v.observed_parent_clock_phase_change_rate,
            grandmaster_identity: v.grandmaster_identity,
            grandmaster_clock_quality: v.grandmaster_clock_quality,
            grandmaster_priority_1: v.grandmaster_priority_1,
            grandmaster_priority_2: v.grandmaster_priority_2,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableTimePropertiesDS {
    pub current_utc_offset: Option<i16>,
    pub leap_indicator: LeapIndicator,
    pub time_traceable: bool,
    pub frequency_traceable: bool,
    pub ptp_timescale: bool,
    pub time_source: TimeSource,
}

impl From<InternalTimePropertiesDS> for ObservableTimePropertiesDS {
    fn from(v: InternalTimePropertiesDS) -> Self {
        Self {
            current_utc_offset: v.current_utc_offset,
            leap_indicator: v.leap_indicator,
            time_traceable: v.time_traceable,
            frequency_traceable: v.frequency_traceable,
            ptp_timescale: v.ptp_timescale,
            time_source: v.time_source,
        }
    }
}
