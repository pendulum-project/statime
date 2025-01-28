use crate::datastructures::{WireFormat, WireFormatError};

// TODO make them configurable instead of constants
pub(crate) const V2_COMPAT_PRIORITY1: u8 = 128;
pub(crate) const V2_COMPAT_PRIORITY1_PREFERRED: u8 = 64;
pub(crate) const V2_COMPAT_PRIORITY2: u8 = 128;


#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
pub(crate) struct GrandmasterPropertiesV1 {
    pub(crate) communication_technology: u8,
    pub(crate) clock_uuid: [u8; 6],
    pub(crate) port_id: u16,
    pub(crate) sequence_id: u16,
    pub(crate) clock_stratum: u8,
    pub(crate) clock_identifier: [u8; 4],
    pub(crate) clock_variance: i16,
    pub(crate) preferred: bool,
    pub(crate) is_boundary_clock: bool
}

impl WireFormat for GrandmasterPropertiesV1 {
    /* fn wire_size(&self) -> usize {
        28
    } */
    fn serialize(&self, buffer: &mut [u8]) -> Result<(), WireFormatError> {
        buffer[0] = 0;
        buffer[1] = self.communication_technology;
        buffer[2..8].copy_from_slice(&self.clock_uuid);
        buffer[8..10].copy_from_slice(&self.port_id.to_be_bytes());
        buffer[10..12].copy_from_slice(&self.sequence_id.to_be_bytes());
        buffer[12] = 0;
        buffer[13] = 0;
        buffer[14] = 0;
        buffer[15] = self.clock_stratum;
        buffer[16..20].copy_from_slice(&self.clock_identifier);
        buffer[20] = 0;
        buffer[21] = 0;
        buffer[22..24].copy_from_slice(&self.clock_variance.to_be_bytes());
        buffer[24] = 0;
        buffer[25] = self.preferred as u8;
        buffer[26] = 0;
        buffer[27] = self.is_boundary_clock as u8;
        Ok(())
    }
    fn deserialize(buffer: &[u8]) -> Result<Self, WireFormatError> {
        match buffer.get(0..28) {
            None => Err(WireFormatError::BufferTooShort),
            Some(buf) => Ok(Self {
                communication_technology: buf[1],
                clock_uuid: buf[2..8].try_into().unwrap(),
                port_id: u16::from_be_bytes(buf[8..10].try_into().unwrap()),
                sequence_id: u16::from_be_bytes(buf[10..12].try_into().unwrap()),
                clock_stratum: buf[15],
                clock_identifier: buf[16..20].try_into().unwrap(),
                clock_variance: i16::from_be_bytes(buf[22..24].try_into().unwrap()),
                preferred: buf[25] > 0,
                is_boundary_clock: buf[27] > 0
            })
        }
    }
}