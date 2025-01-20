use crate::datastructures::{common::WireTimestampV1, WireFormat, WireFormatError};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FollowUpMessage {
    pub(crate) associated_sequence_id: u16,
    pub(crate) precise_origin_timestamp: WireTimestampV1,
}
impl FollowUpMessage {
    pub(crate) fn content_size(&self) -> usize {
        12
    }
    pub(crate) fn serialize_content(&self, buffer: &mut [u8]) -> Result<(), WireFormatError> {
        buffer[0..2].copy_from_slice(&self.associated_sequence_id.to_be_bytes());
        self.precise_origin_timestamp
            .serialize(&mut buffer[2..10])?;
        Ok(())
    }
    pub(crate) fn deserialize_content(buffer: &[u8]) -> Result<Self, WireFormatError> {
        let slice = buffer.get(4..12).ok_or(WireFormatError::BufferTooShort)?;
        let precise_origin_timestamp = WireTimestampV1::deserialize(slice)?;
        Ok(Self {
            associated_sequence_id: u16::from_be_bytes(buffer[2..4].try_into().unwrap()),
            precise_origin_timestamp,
        })
    }
}