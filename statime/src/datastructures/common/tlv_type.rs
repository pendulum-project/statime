/// See 14.1.1 / Table 52
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlvType {
    Reserved,
    Management,
    ManagementErrorStatus,
    OrganizationExtension,
    RequestUnicastTransmission,
    GrantUnicastTransmission,
    CancelUnicastTransmission,
    AcknowledgeCancelUnicastTransmission,
    PathTrace,
    AlternateTimeOffsetIndicator,
    Legacy,
    Experimental,
    OrganizationExtensionPropagate,
    EnhancedAccuracyMetrics,
    OrganizationExtensionDoNotPropagate,
    L1Sync,
    PortCommunicationAvailability,
    ProtocolAddress,
    SlaveRxSyncTimingData,
    SlaveRxSyncComputedData,
    SlaveTxEventTimestamps,
    CumulativeRateRatio,
    Pad,
    Authentication,
}

impl TlvType {
    pub fn to_primitive(&self) -> u16 {
        match self {
            Self::Reserved => 0x0000,
            Self::Management => 0x0001,
            Self::ManagementErrorStatus => 0x0002,
            Self::OrganizationExtension => 0x0003,
            Self::RequestUnicastTransmission => 0x0004,
            Self::GrantUnicastTransmission => 0x0005,
            Self::CancelUnicastTransmission => 0x0006,
            Self::AcknowledgeCancelUnicastTransmission => 0x0007,
            Self::PathTrace => 0x0008,
            Self::AlternateTimeOffsetIndicator => 0x0009,
            Self::Legacy => 0x2000,
            Self::Experimental => 0x2004,
            Self::OrganizationExtensionPropagate => 0x4000,
            Self::EnhancedAccuracyMetrics => 0x4001,
            Self::OrganizationExtensionDoNotPropagate => 0x8000,
            Self::L1Sync => 0x8001,
            Self::PortCommunicationAvailability => 0x8002,
            Self::ProtocolAddress => 0x8003,
            Self::SlaveRxSyncTimingData => 0x8004,
            Self::SlaveRxSyncComputedData => 0x8005,
            Self::SlaveTxEventTimestamps => 0x8006,
            Self::CumulativeRateRatio => 0x8007,
            Self::Pad => 0x8008,
            Self::Authentication => 0x8009,
        }
    }

    pub fn from_primitive(value: u16) -> Self {
        match value {
            0x0000
            | 0x000A..=0x1FFF
            | 0x2030..=0x3FFF
            | 0x4002..=0x7EFF
            | 0x800A..=0xFFEF
            | 0xFFF0..=0xFFFF => Self::Reserved,
            0x2000..=0x2003 => Self::Legacy,
            0x2004..=0x202F | 0x7F00..=0x7FFF => Self::Experimental,
            0x0001 => Self::Management,
            0x0002 => Self::ManagementErrorStatus,
            0x0003 => Self::OrganizationExtension,
            0x0004 => Self::RequestUnicastTransmission,
            0x0005 => Self::GrantUnicastTransmission,
            0x0006 => Self::CancelUnicastTransmission,
            0x0007 => Self::AcknowledgeCancelUnicastTransmission,
            0x0008 => Self::PathTrace,
            0x0009 => Self::AlternateTimeOffsetIndicator,
            0x4000 => Self::OrganizationExtensionPropagate,
            0x4001 => Self::EnhancedAccuracyMetrics,
            0x8000 => Self::OrganizationExtensionDoNotPropagate,
            0x8001 => Self::L1Sync,
            0x8002 => Self::PortCommunicationAvailability,
            0x8003 => Self::ProtocolAddress,
            0x8004 => Self::SlaveRxSyncTimingData,
            0x8005 => Self::SlaveRxSyncComputedData,
            0x8006 => Self::SlaveTxEventTimestamps,
            0x8007 => Self::CumulativeRateRatio,
            0x8008 => Self::Pad,
            0x8009 => Self::Authentication,
        }
    }
}

impl Default for TlvType {
    fn default() -> Self {
        Self::Management
    }
}
