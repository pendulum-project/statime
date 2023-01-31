use crate::datastructures::{common::Timestamp, WireFormat, WireFormatError};
use getset::CopyGetters;

use super::Header;

#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters)]
#[getset(get_copy = "pub")]
pub struct PDelayRespFollowUpMessage {
    pub(super) header: Header,
}

impl PDelayRespFollowUpMessage {
    pub fn content_size(&self) -> usize {
        10
    }
}

