use crate::config::{ClockIdentity, SdoId};
#[cfg(doc)]
use crate::PtpInstance;

/// Configuration for a [`PtpInstance`]
///
/// # Example
/// A configuration with common default values:
/// ```
/// # use statime::config::{ClockIdentity, InstanceConfig, SdoId};
/// let config = InstanceConfig {
///     clock_identity: ClockIdentity::from_mac_address([1,2,3,4,5,6]),
///     priority_1: 128,
///     priority_2: 128,
///     domain_number: 0,
///     sdo_id: SdoId::default(),
///     slave_only: false,
///     path_trace: false,
/// };
/// ```
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct InstanceConfig {
    /// The unique identifier for this device within the PTP network.
    pub clock_identity: ClockIdentity,

    /// Priority of this clock while selecting a master clock.
    ///
    /// Lower values assign a higher priority.
    pub priority_1: u8,

    /// Tie-breaker priority during master clock selection on otherwise
    /// identical instances.
    ///
    /// Lower values assign a higher priority.
    pub priority_2: u8,

    /// This and [`InstanceConfig::sdo_id`] together identify which domain a
    /// [`PtpInstance`] belongs to.
    ///
    /// In general nodes will only communicate within their domain. See *IEEE
    /// 1588-2019 table 2* for permitted combinations.
    pub domain_number: u8,

    /// See [`InstanceConfig::domain_number`].
    pub sdo_id: SdoId,

    /// Whether this node may never become a master in the network
    pub slave_only: bool,

    /// Whether the path trace option is enabled
    pub path_trace: bool,
}
