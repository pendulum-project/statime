use crate::datastructures::{WireFormat, WireFormatError};

use super::TlvType;

use arrayvec::ArrayVec;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TLV {
    pub tlv_type: TlvType,
    pub length: u16,

    // TODO: Determine the best max value
    pub value: ArrayVec<u8, 128>,
}

impl WireFormat for TLV {
    fn wire_size(&self) -> usize {
        (4 + self.length).into()
    }

    fn serialize(&self, buffer: &mut [u8]) -> Result<(), WireFormatError> {
        buffer[0..1].copy_from_slice(&self.tlv_type.to_primitive().to_be_bytes());
        buffer[2..3].copy_from_slice(&self.length.to_be_bytes());
        buffer[4..(4 + self.length).into()].copy_from_slice(&self.value.as_slice());

        Ok(())
    }

    fn deserialize(buffer: &[u8]) -> Result<Self, WireFormatError> {
        if buffer.len() < 5 {
            return Err(WireFormatError::BufferTooShort);
        }

        // Parse length
        let lengthBytes: Result<[u8; 2],_> = buffer[2..3].try_into();
        if lengthBytes.is_err() {
            return Err(WireFormatError::SliceError);
        }
        let length = u16::from_be_bytes(lengthBytes.unwrap());

        // Parse TLV content / value
        if buffer.len() < (5 + length) as usize {
            return Err(WireFormatError::BufferTooShort);
        }

        let mut vec = ArrayVec::<u8, 128>::new();
        for byte in &buffer[4..(4 + length).into()] {
            if !vec.try_push(*byte).is_ok() {
                return Err(WireFormatError::CapacityError);
            }
        }

        // Parse TLV type
        let typeBytes = buffer[0..1].try_into();
        if typeBytes.is_err() {
            return Err(WireFormatError::SliceError);
        }

        Ok(Self {
            tlv_type: TlvType::from_primitive(
                u16::from_be_bytes(typeBytes.unwrap())
            ),
            length: length,
            value: vec,
        })
    }
}
