use crate::datastructures::{
    common::{PortIdentity, TLV},
    WireFormat, WireFormatError,
};

use super::Header;
use arrayvec::ArrayVec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalingMessage {
    pub(super) header: Header,
    pub(super) target_port_identity: PortIdentity,

    // TODO: determine the best max length value
    pub(super) value: ArrayVec<TLV, 256>,
}

impl SignalingMessage {
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

        self.target_port_identity.serialize(&mut buffer[0..10])?;

        // TODO: value

        Ok(())
    }

    pub fn deserialize_content(
        header: Header,
        buffer: &[u8],
    ) -> Result<Self, crate::datastructures::WireFormatError> {
        if buffer.len() < 11 {
            return Err(WireFormatError::BufferTooShort);
        }

        let mut index = 11;
        let mut tlvs = ArrayVec::<TLV, 256>::new();
        while buffer.len() > index + 4 {
            // Parse length
            let length_bytes: Result<[u8; 2], _> = buffer[(index + 2)..(index + 4)].try_into();
            if length_bytes.is_err() {
                return Err(WireFormatError::SliceError);
            }
            let length = u16::from_be_bytes(length_bytes.unwrap()) as usize;

            if buffer.len() < index + 4 + length {
                return Err(WireFormatError::BufferTooShort);
            }

            // Parse TLV
            let tlv = TLV::deserialize(&buffer[index..(index + 4 + length)]);
            if tlv.is_err() {
                return Err(WireFormatError::SliceError);
            }

            tlvs.push(tlv.unwrap());
            index = index + 4 + length;
        }

        Ok(Self {
            header,
            target_port_identity: PortIdentity::deserialize(&buffer[0..10])?,
            value: tlvs,
        })
    }
}
