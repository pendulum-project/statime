use crate::datastructures::datasets::InternalDefaultDS;

/// A concrete implementation of the PTP Current dataset (IEEE1588-2019 section
/// 8.2.1)
///
/// See [InternalDefaultDS](crate::datastructures::datasets::InternalDefaultDS).
#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DefaultDS {
    /// The identity of a PTP node.
    /// See *IEEE1588-2019 section 8.2.1.2.2*.
    pub clock_identity: crate::config::ClockIdentity,
    /// The amount of PTP ports on this PTP instance.
    /// See *IEEE1588-2019 section 8.2.1.2.3*.
    pub number_ports: u16,
    /// A description of the accuracy and type of a clock.
    pub clock_quality: crate::config::ClockQuality,
    /// See *IEEE1588-2019 section 8.2.1.4.1*.
    pub priority_1: u8,
    /// See *IEEE1588-2019 section 8.2.1.4.2*.
    pub priority_2: u8,
    /// See *IEEE1588-2019 section 8.2.1.4.3*.
    pub domain_number: u8,
    /// See *IEEE1588-2019 section  8.2.1.4.4*.
    pub slave_only: bool,
    /// See *IEEE1588-2019 section 7.1.4 table 2*.
    pub sdo_id: crate::config::SdoId,
}

impl From<&InternalDefaultDS> for DefaultDS {
    fn from(v: &InternalDefaultDS) -> Self {
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
