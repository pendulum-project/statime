//! Serializable implementations of datastructures to be used for observability
#![allow(missing_docs)]
use crate::{
    config::{ClockIdentity, ClockQuality, TimePropertiesDS},
    datastructures::{
        common::PortIdentity,
        datasets::{InternalCurrentDS, InternalDefaultDS, InternalParentDS},
    },
};

/// Observable version of the InstanceState struct
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ObservableInstanceState {
    pub default_ds: DefaultDS,
    pub current_ds: CurrentDS,
    pub parent_ds: ParentDS,
    pub time_properties_ds: TimePropertiesDS,
}

/// Observable version of the DefaultDS struct
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DefaultDS {
    pub clock_identity: crate::config::ClockIdentity,
    pub number_ports: u16,
    pub clock_quality: crate::config::ClockQuality,
    pub priority_1: u8,
    pub priority_2: u8,
    pub domain_number: u8,
    pub slave_only: bool,
    pub sdo_id: crate::config::SdoId,
}

impl From<InternalDefaultDS> for DefaultDS {
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
pub struct CurrentDS {
    pub steps_removed: u16,
    pub offset_from_master: i128, // rounded nanos
    pub mean_delay: i128,         // rounded nanos
}

impl From<InternalCurrentDS> for CurrentDS {
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
pub struct ParentDS {
    pub parent_port_identity: PortIdentity,
    pub parent_stats: bool,
    pub observed_parent_offset_scaled_log_variance: u16,
    pub observed_parent_clock_phase_change_rate: u32,
    pub grandmaster_identity: ClockIdentity,
    pub grandmaster_clock_quality: ClockQuality,
    pub grandmaster_priority_1: u8,
    pub grandmaster_priority_2: u8,
}

impl From<InternalParentDS> for ParentDS {
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
