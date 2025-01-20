use crate::datastructures::{
    common::WireTimestampV1,
    WireFormat, WireFormatError,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DelayRespMessage {
    pub(crate) receive_timestamp: WireTimestampV1,
    pub(crate) requesting_source_communication_technology: u8,
    pub(crate) requesting_source_uuid: [u8; 6],
    pub(crate) requesting_source_port_id: u16,
    pub(crate) requesting_source_sequence_id: u16,
}
impl DelayRespMessage {
    pub(crate) fn content_size(&self) -> usize {
        20
    }
    pub(crate) fn serialize_content(&self, buffer: &mut [u8]) -> Result<(), WireFormatError> {
        self.receive_timestamp.serialize(&mut buffer[0..8])?;
        buffer[8] = 0;
        buffer[9] = self.requesting_source_communication_technology;
        buffer[10..16].copy_from_slice(&self.requesting_source_uuid);
        buffer[16..18].copy_from_slice(&self.requesting_source_port_id.to_be_bytes());
        buffer[18..20].copy_from_slice(&self.requesting_source_sequence_id.to_be_bytes());
        Ok(())
    }
    pub(crate) fn deserialize_content(buffer: &[u8]) -> Result<Self, WireFormatError> {
        let buf = buffer.get(0..20).ok_or(WireFormatError::BufferTooShort)?;
        let receive_timestamp = WireTimestampV1::deserialize(&buf[0..8])?;
        Ok(Self {
            receive_timestamp,
            requesting_source_communication_technology: buf[9],
            requesting_source_uuid: buf[10..16].try_into().unwrap(),
            requesting_source_port_id: u16::from_be_bytes(buf[16..18].try_into().unwrap()),
            requesting_source_sequence_id: u16::from_be_bytes(buf[18..20].try_into().unwrap()),
        })
    }
}