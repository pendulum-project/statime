use arrayvec::ArrayVec;

use crate::datastructures::{common::ClockIdentity, messages::MAX_DATA_LEN};

/// A concrete implementation of the PTP Path Trace dataset
/// (IEEE1588-2019 section 16.2.2)
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PathTraceDS {
    /// See *IEEE1588-2019 section 16.2.2.2.1*.
    pub list: ArrayVec<ClockIdentity, { MAX_DATA_LEN / 8 }>,
    /// See *IEEE1588-2019 section 16.2.2.3.1*.
    pub enable: bool,
}

impl PathTraceDS {
    pub(crate) fn new(enable: bool) -> Self {
        PathTraceDS {
            list: Default::default(),
            enable,
        }
    }
}
