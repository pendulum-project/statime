use crate::{
    config::{ClockIdentity, ClockQuality},
    datastructures::{common::PortIdentity, datasets::InternalParentDS},
};

/// A concrete implementation of the PTP Current dataset (IEEE1588-2019 section
/// 8.2.3)
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParentDS {
    /// See *IEEE1588-2019 section 8.2.3.2*.
    pub parent_port_identity: PortIdentity,
    /// See *IEEE1588-2019 section 8.2.3.3*.
    pub parent_stats: bool,
    /// See *IEEE1588-2019 section 8.2.3.4*.
    pub observed_parent_offset_scaled_log_variance: u16,
    /// See *IEEE1588-2019 section 8.2.3.5*.
    pub observed_parent_clock_phase_change_rate: u32,
    /// See *IEEE1588-2019 section 8.2.3.6*.
    pub grandmaster_identity: ClockIdentity,
    /// See *IEEE1588-2019 section 8.2.3.7*.
    pub grandmaster_clock_quality: ClockQuality,
    /// See *IEEE1588-2019 section 8.2.3.8*.
    pub grandmaster_priority_1: u8,
    /// See *IEEE1588-2019 section 8.2.3.9*.
    pub grandmaster_priority_2: u8,
}

impl From<&InternalParentDS> for ParentDS {
    fn from(v: &InternalParentDS) -> Self {
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
