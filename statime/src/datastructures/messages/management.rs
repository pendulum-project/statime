use crate::datastructures::{common::{PortIdentity, Timestamp}, WireFormat, WireFormatError};
use getset::CopyGetters;

use super::Header;

#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters)]
#[getset(get_copy = "pub")]
pub struct ManagementMessage {
    pub(super) header: Header,
    pub(super) target_port_identity: PortIdentity,
}

impl ManagementMessage {
    pub fn content_size(&self) -> usize {
        10
    }

    pub fn serialize_content(
        &self,
        buffer: &mut [u8],
    ) -> Result<(), crate::datastructures::WireFormatError> {
        if buffer.len() < 11 {
            return Err(WireFormatError::BufferTooShort);
        }

        self.target_port_identity
            .serialize(&mut buffer[0..10])?;

        Ok(())
    }

    pub fn deserialize_content(
        header: Header,
        buffer: &[u8],
    ) -> Result<Self, crate::datastructures::WireFormatError> {
        if buffer.len() < 11 {
            return Err(WireFormatError::BufferTooShort);
        }
        Ok(Self {
            header,
            target_port_identity: PortIdentity::deserialize(&buffer[0..10])?,
        })
    }
}
