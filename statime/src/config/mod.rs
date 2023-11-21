//! Configuration structures
//!
//! Configurations for a [`PtpInstance`](`crate::PtpInstance`):
//! * [`InstanceConfig`]
//! * [`TimePropertiesDS`]
//!
//! Configurations for a [`Port`](`crate::port::Port`):
//! * [`PortConfig`]
//!
//! And types used within those configurations.

mod instance;
mod port;

pub use instance::InstanceConfig;
pub use port::{DelayMechanism, PortConfig};

pub use crate::{
    bmc::acceptable_master::{AcceptAnyMaster, AcceptableMasterList},
    datastructures::{
        common::{ClockAccuracy, ClockIdentity, ClockQuality, LeapIndicator, TimeSource},
        datasets::TimePropertiesDS,
        messages::SdoId,
    },
};
