use rand::Rng;

use crate::time::{Duration, Interval};
#[cfg(doc)]
use crate::{config::AcceptableMasterList, port::Port};

/// Which delay mechanism a port is using.
///
/// Currently, statime only supports the end to end (E2E) delay mechanism.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum DelayMechanism {
    /// End to end delay mechanism. Delay measurement is done directly to the
    /// chosen master, across potential transparent nodes in between.
    E2E {
        /// The time between sending two delay requests
        interval: Interval,
    },
    /// Peer to peer delay mechanism. Delay measurement is done on the
    /// individaul links.
    P2P {
        /// The time between sending two peer delay requests
        interval: Interval,
    },
    // No support for other delay mechanisms
}

/// Configuration items of the PTP PortDS dataset. Dynamical fields are kept
/// as part of [crate::port::Port].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct PortConfig<A> {
    /// A list that contains all nodes that this [`Port`] will accept as a
    /// master.
    ///
    /// This should implement the [`AcceptableMasterList`] trait.
    pub acceptable_master_list: A,

    /// The mechanism used to measure the delay at this [`Port`].
    pub delay_mechanism: DelayMechanism,

    /// The time between announcements.
    pub announce_interval: Interval,

    /// Specifies how many [`announce_interval`](`Self::announce_interval`)s to
    /// wait until the announce message expires.
    pub announce_receipt_timeout: u8,

    /// Time between two sync messages when this [`Port`] is in master mode.
    pub sync_interval: Interval,

    /// Never let this [`Port`] become a slave.
    pub master_only: bool,

    /// The estimated asymmetry in the link connected to this [`Port`]
    pub delay_asymmetry: Duration,
    // Notes:
    // Fields specific for delay mechanism are kept as part of [DelayMechanism].
    // Version is always 2.1, so not stored (versionNumber, minorVersionNumber)
}

impl<A> PortConfig<A> {
    /// Minimum time between two delay request messages
    pub fn min_delay_req_interval(&self) -> Interval {
        match self.delay_mechanism {
            DelayMechanism::E2E { interval } => interval,
            DelayMechanism::P2P { interval } => interval,
        }
    }

    /// Time between two announce messages
    ///
    /// For more information see *IEEE1588-2019 section 9.2.6.12*
    pub fn announce_duration(&self, rng: &mut impl Rng) -> core::time::Duration {
        // add some randomness so that not all timers expire at the same time
        let factor = 1.0 + rng.sample::<f64, _>(rand::distributions::Open01);
        let duration = self.announce_interval.as_core_duration();

        duration.mul_f64(factor * self.announce_receipt_timeout as u32 as f64)
    }
}
