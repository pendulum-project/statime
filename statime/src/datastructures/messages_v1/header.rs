use super::{control_field::ControlField, PortType};
use crate::datastructures::WireFormatError;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub(crate) version_ptp: u16,
    pub(crate) version_network: u16,
    pub(crate) subdomain: [u8; 16],
    // port_type specially handled
    pub(crate) source_communication_technology: u8,
    pub(crate) source_uuid: [u8; 6],
    pub(crate) source_port_id: u16,
    pub(crate) sequence_id: u16,
    // control specially handled
    pub(crate) leap61: bool,
    pub(crate) leap59: bool,
    pub(crate) ptp_boundary_clock: bool,
    pub(crate) ptp_assist: bool,
    pub(crate) ptp_ext_sync: bool,
    pub(crate) parent_stats: bool,
    pub(crate) ptp_sync_burst: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeserializedHeader {
    pub(crate) header: Header,
    pub(crate) control: ControlField,
    //pub(crate) message_length: usize,
}
impl Header {
    pub(super) fn new() -> Self {
        Self {
            version_ptp: 1,
            version_network: 1,
            subdomain: [b'_', b'D', b'F', b'L', b'T', 0,0,0,0,0,0,0,0,0,0,0],
            source_communication_technology: 1,
            source_uuid: [0,0,0,0,0,0],
            source_port_id: 0,
            sequence_id: 0,
            leap59: false,
            leap61: false,
            ptp_boundary_clock: false,
            ptp_assist: false,
            ptp_ext_sync: false,
            parent_stats: false,
            ptp_sync_burst: false
        }
    }
    pub(crate) fn wire_size(&self) -> usize {
        40
    }
    pub(crate) fn serialize_header(
        &self,
        control: ControlField,
        _content_length: usize,
        buffer: &mut [u8],
    ) -> Result<(), WireFormatError> {
        let port_type = match control {
            ControlField::Sync | ControlField::DelayReq => PortType::Event,
            _ => PortType::General
        };
        buffer[0..2].copy_from_slice(&self.version_ptp.to_be_bytes());
        buffer[2..4].copy_from_slice(&self.version_network.to_be_bytes());
        buffer[4..20].copy_from_slice(&self.subdomain);
        buffer[20] = port_type as u8;
        buffer[21] = self.source_communication_technology;
        buffer[22..28].copy_from_slice(&self.source_uuid);
        buffer[28..30].copy_from_slice(&self.source_port_id.to_be_bytes());
        buffer[30..32].copy_from_slice(&self.sequence_id.to_be_bytes());
        buffer[32] = control.to_primitive();
        buffer[33] = 0;
        buffer[34] = 0;
        buffer[35] = 0;
        buffer[35] |= self.leap61 as u8;
        buffer[35] |= (self.leap59 as u8) << 1;
        buffer[35] |= (self.ptp_boundary_clock as u8) << 2;
        buffer[35] |= (self.ptp_assist as u8) << 3;
        buffer[35] |= (self.ptp_ext_sync as u8) << 4;
        buffer[35] |= (self.parent_stats as u8) << 5;
        buffer[35] |= (self.ptp_sync_burst as u8) << 6;
        buffer[36] = 0;
        buffer[37] = 0;
        buffer[38] = 0;
        buffer[39] = 0;
        Ok(())
    }
    pub(crate) fn deserialize_header(buffer: &[u8]) -> Result<DeserializedHeader, WireFormatError> {
        if buffer.len() < 40 {
            return Err(WireFormatError::BufferTooShort);
        }
        Ok(DeserializedHeader {
            header: Self {
                version_ptp: u16::from_be_bytes([buffer[0], buffer[1]]),
                version_network: u16::from_be_bytes([buffer[2], buffer[3]]),
                subdomain: buffer[4..20].try_into().unwrap(),
                source_communication_technology: buffer[21],
                source_uuid: buffer[22..28].try_into().unwrap(),
                source_port_id: u16::from_be_bytes([buffer[28], buffer[29]]),
                sequence_id: u16::from_be_bytes([buffer[30], buffer[31]]),
                leap61: (buffer[35] & (1 << 0)) > 0,
                leap59: (buffer[35] & (1 << 1)) > 0,
                ptp_boundary_clock: (buffer[35] & (1 << 2)) > 0,
                ptp_assist: (buffer[35] & (1 << 3)) > 0,
                ptp_ext_sync: (buffer[35] & (1 << 4)) > 0,
                parent_stats: (buffer[35] & (1 << 5)) > 0,
                ptp_sync_burst: (buffer[35] & (1 << 6)) > 0
            },
            control: buffer[32].try_into()?,
            //message_length: buffer.len() - 40,
        })
    }
}
impl Default for Header {
    fn default() -> Self {
        Self::new()
    }
}