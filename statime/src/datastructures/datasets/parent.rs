use super::InternalDefaultDS;
use crate::datastructures::common::{ClockIdentity, ClockQuality, PortIdentity};

// TODO: Discuss moving this (and TimePropertiesDS, ...) to slave?
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InternalParentDS {
    pub(crate) parent_port_identity: PortIdentity,
    pub(crate) grandmaster_identity: ClockIdentity,
    pub(crate) grandmaster_clock_quality: ClockQuality,
    pub(crate) grandmaster_priority_1: u8,
    pub(crate) grandmaster_priority_2: u8,
}

impl InternalParentDS {
    pub(crate) fn new(default_ds: InternalDefaultDS) -> Self {
        InternalParentDS {
            parent_port_identity: PortIdentity {
                clock_identity: default_ds.clock_identity,
                port_number: 0,
            },
            grandmaster_identity: default_ds.clock_identity,
            grandmaster_clock_quality: default_ds.clock_quality,
            grandmaster_priority_1: default_ds.priority_1,
            grandmaster_priority_2: default_ds.priority_2,
        }
    }
}
