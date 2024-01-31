//! Serializable implementations of datastructures to be used for observability
#![allow(missing_docs)]
use crate::datastructures::datasets::DefaultDS;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Observable version of the InstanceState struct
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct ObservableInstanceState {
    pub default_ds: ObservableDefaultDS
}

/// Observable version of the DefaultDS struct
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
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

impl From<DefaultDS> for ObservableDefaultDS {
    fn from(v: DefaultDS) -> Self {
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
