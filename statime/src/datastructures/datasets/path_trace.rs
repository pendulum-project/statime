use arrayvec::ArrayVec;

use crate::datastructures::{common::ClockIdentity, messages::MAX_DATA_LEN};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct InternalPathTraceDS {
    pub(crate) list: ArrayVec<ClockIdentity, { MAX_DATA_LEN / 8 }>,
    pub(crate) enable: bool,
}

impl InternalPathTraceDS {
    pub(crate) fn new(enable: bool) -> Self {
        InternalPathTraceDS {
            list: Default::default(),
            enable,
        }
    }
}
