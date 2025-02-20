use crate::datastructures::common::{PortIdentity, TimeInterval};

/// Type for `[PortDS].port_state`, see also *IEEE1588-2019 section 8.2.15.3.1
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
#[allow(missing_docs)]
pub enum PortState {
    Initializing = 1,
    Faulty = 2,
    Disabled = 3,
    Listening = 4,
    PreMaster = 5,
    Master = 6,
    Passive = 7,
    Uncalibrated = 8,
    Slave = 9,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
#[allow(missing_docs)]
pub enum DelayMechanism {
    E2E {
        /// See *IEEE1588-2019 section 8.2.15.3.2*
        log_min_delay_req_interval: i8,
    } = 1,
    P2P {
        /// See *IEEE1588-2019 section 8.2.15.4.5*
        log_min_p_delay_req_interval: i8,
        /// See *IEEE1588-2019 section 8.2.15.3.3*
        mean_link_delay: TimeInterval,
    } = 2,
    NoMechanism = 0xfe,
    CommonP2P {
        /// See *IEEE1588-2019 section 8.2.15.3.3*
        mean_link_delay: TimeInterval,
    } = 3,
    Special = 4,
}

/// A concrete implementation of the PTP Port dataset (IEEE1588-2019 section
/// 8.2.15)
///
/// meanLinkDelay, logMinDelayReqInterval and logMinPDelayReqInterval are
/// exposed through the delay mechanism type when the relevant delay mechanism
/// is in use.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PortDS {
    /// See *IEEE1588-2019 section 8.2.15.2.1*
    pub port_identity: PortIdentity,
    /// See *IEEE1588-2019 section 8.2.15.3.1*
    pub port_state: PortState,
    /// See *IEEE1588-2019 section 8.2.15.4.1*
    pub log_announce_interval: i8,
    /// See *IEEE1588-2019 section 8.2.15.4.2*
    pub announce_receipt_timeout: u8,
    /// See *IEEE1588-2019 section 8.2.15.4.3*
    pub log_sync_interval: i8,
    /// See *IEEE1588-2019 section 8.2.15.4.4*
    pub delay_mechanism: DelayMechanism,
    /// See *IEEE1588-2019 section 8.2.15.4.6*
    pub version_number: u8,
    /// See *IEEE1588-2019 section 8.2.15.4.7*
    pub minor_version_number: u8,
    /// See *IEEE1588-2019 section 8.2.15.4.8*
    pub delay_asymmetry: TimeInterval,
    /// See *IEEE1588-2019 section 8.2.15.5.2*
    pub master_only: bool,
}
