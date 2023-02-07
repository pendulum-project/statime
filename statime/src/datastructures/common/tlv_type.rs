/// See 14.1.1 / Table 52
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlvType {
    Reserved,
    MANAGEMENT,
    MANAGEMENT_ERROR_STATUS,
    ORGANIZATION_EXTENSION,
    REQUEST_UNICAST_TRANSMISSION,
    GRANT_UNICAST_TRANSMISSION,
    CANCEL_UNICAST_TRANSMISSION,
    ACKNOWLEDGE_CANCEL_UNICAST_TRANSMISSION,
    PATH_TRACE,
    ALTERNATE_TIME_OFFSET_INDICATOR,
    Legacy,
    Experimental,
    ORGANIZATION_EXTENSION_DO_NOT_PROPAGATE,
    ORGANIZATION_EXTENSION_PROPAGATE,
    ENHANCED_ACCURACY_METRICS,
    L1_SYNC,
    PORT_COMMUNICATION_AVAILABILITY,
    PROTOCOL_ADDRESS,
    SLAVE_RX_SYNC_TIMING_DATA,
    SLAVE_RX_SYNC_COMPUTED_DATA,
    SLAVE_TX_EVENT_TIMESTAMPS,
    CUMULATIVE_RATE_RATIO,
    PAD,
    AUTHENTICATION,
}

impl TlvType {

    pub fn to_primitive(&self) -> u16 {
        match self {
            Self::Reserved => 0x0000,
            Self::MANAGEMENT => 0x0001,
            Self::MANAGEMENT_ERROR_STATUS => 0x0002,
            Self::ORGANIZATION_EXTENSION => 0x0003,
            Self::REQUEST_UNICAST_TRANSMISSION => 0x0004,
            Self::GRANT_UNICAST_TRANSMISSION => 0x0005,
            Self::CANCEL_UNICAST_TRANSMISSION => 0x0006,
            Self::ACKNOWLEDGE_CANCEL_UNICAST_TRANSMISSION => 0x0007,
            Self::PATH_TRACE => 0x0008,
            Self::ALTERNATE_TIME_OFFSET_INDICATOR => 0x0009,
            Self::Legacy => 0x2000,
            Self::Experimental => 0x2004,
            Self::ORGANIZATION_EXTENSION_PROPAGATE => 0x4000,
            Self::ENHANCED_ACCURACY_METRICS => 0x4001,
            Self::ORGANIZATION_EXTENSION_DO_NOT_PROPAGATE => 0x8000,
            Self::L1_SYNC => 0x8001,
            Self::PORT_COMMUNICATION_AVAILABILITY => 0x8002,
            Self::PROTOCOL_ADDRESS => 0x8003,
            Self::SLAVE_RX_SYNC_TIMING_DATA => 0x8004,
            Self::SLAVE_RX_SYNC_COMPUTED_DATA => 0x8005,
            Self::SLAVE_TX_EVENT_TIMESTAMPS => 0x8006,
            Self::CUMULATIVE_RATE_RATIO => 0x8007,
            Self::PAD => 0x8008,
            Self::AUTHENTICATION => 0x8009,
        }
    }

    pub fn from_primitive(value: u16) -> Self {
        match value {
            0x0000 | 0x000A..=0x1FFF | 0x2030..=0x3FFF | 0x4002..=0x7EFF | 0x800A..=0xFFEF | 0xFFF0..=0xFFFF => Self::Reserved,
            0x2000..=0x2003 => Self::Legacy,
            0x2004..=0x202F | 0x7F00..=0x7FFF => Self::Experimental,
            0x0001 => Self::MANAGEMENT,
            0x0002 => Self::MANAGEMENT_ERROR_STATUS,
            0x0003 => Self::ORGANIZATION_EXTENSION,
            0x0004 => Self::REQUEST_UNICAST_TRANSMISSION,
            0x0005 => Self::GRANT_UNICAST_TRANSMISSION,
            0x0006 => Self::CANCEL_UNICAST_TRANSMISSION,
            0x0007 => Self::ACKNOWLEDGE_CANCEL_UNICAST_TRANSMISSION,
            0x0008 => Self::PATH_TRACE,
            0x0009 => Self::ALTERNATE_TIME_OFFSET_INDICATOR,
            0x4000 => Self::ORGANIZATION_EXTENSION_PROPAGATE,
            0x4001 => Self::ENHANCED_ACCURACY_METRICS,
            0x8000 => Self::ORGANIZATION_EXTENSION_DO_NOT_PROPAGATE,
            0x8001 => Self::L1_SYNC,
            0x8002 => Self::PORT_COMMUNICATION_AVAILABILITY,
            0x8003 => Self::PROTOCOL_ADDRESS,
            0x8004 => Self::SLAVE_RX_SYNC_TIMING_DATA,
            0x8005 => Self::SLAVE_RX_SYNC_COMPUTED_DATA,
            0x8006 => Self::SLAVE_TX_EVENT_TIMESTAMPS,
            0x8007 => Self::CUMULATIVE_RATE_RATIO,
            0x8008 => Self::PAD,
            0x8009 => Self::AUTHENTICATION,
        }
    }
}

impl Default for TlvType {
    fn default() -> Self { Self::MANAGEMENT }
}
