use crate::datastructures::{
    common::{PortIdentity, Timestamp},
    WireFormat, WireFormatError,
};
use getset::CopyGetters;

use super::Header;

#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters)]
#[getset(get_copy = "pub")]
pub struct PDelayRespFollowUpMessage {
    pub(super) header: Header,
    pub(super) response_origin_timestamp: Timestamp,
    pub(super) requesting_port_identity: PortIdentity,
}

impl PDelayRespFollowUpMessage {
    pub fn content_size(&self) -> usize {
        20
    }

    pub fn serialize_content(
        &self,
        buffer: &mut [u8],
    ) -> Result<(), crate::datastructures::WireFormatError> {
        if buffer.len() < 21 {
            return Err(WireFormatError::BufferTooShort);
        }

        self.response_origin_timestamp
            .serialize(&mut buffer[0..10])?;
        self.requesting_port_identity
            .serialize(&mut buffer[10..20])?;

        Ok(())
    }

    pub fn deserialize_content(
        header: Header,
        buffer: &[u8],
    ) -> Result<Self, crate::datastructures::WireFormatError> {
        if buffer.len() < 21 {
            return Err(WireFormatError::BufferTooShort);
        }
        Ok(Self {
            header,
            response_origin_timestamp: Timestamp::deserialize(&buffer[0..10])?,
            requesting_port_identity: PortIdentity::deserialize(&buffer[10..20])?,
        })
    }
}
