use crate::config::{ClockIdentity, SdoId};

// TODO: Could this implement Default? or have a with_id or similar?
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct InstanceConfig {
    pub clock_identity: ClockIdentity,
    pub priority_1: u8,
    pub priority_2: u8,
    pub domain_number: u8,
    pub slave_only: bool,
    pub sdo_id: SdoId,
}
