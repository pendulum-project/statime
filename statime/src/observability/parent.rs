use crate::{
    config::{ClockIdentity, ClockQuality},
    datastructures::{common::PortIdentity, datasets::InternalParentDS},
};

/// A concrete implementation of the PTP Parent dataset (IEEE1588-2019 section
/// 8.2.3)
///
/// These fields aren't implemented, because they are currently unused:
/// - parentStats
/// - observedParentOffsetScaledLogVariance
/// - observedParentClockPhaseChangeRate

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParentDS {
    /// See *IEEE1588-2019 section 8.2.3.2*.
    pub parent_port_identity: PortIdentity,
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
            grandmaster_identity: v.grandmaster_identity,
            grandmaster_clock_quality: v.grandmaster_clock_quality,
            grandmaster_priority_1: v.grandmaster_priority_1,
            grandmaster_priority_2: v.grandmaster_priority_2,
        }
    }
}
