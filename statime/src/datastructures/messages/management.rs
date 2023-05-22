use super::Header;
use crate::datastructures::{
    common::{PortIdentity, Tlv},
    WireFormat, WireFormatError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagementMessage {
    pub(super) header: Header,
    pub(super) target_port_identity: PortIdentity,
    pub(super) starting_boundary_hops: u8,
    pub(super) boundary_hops: u8,
    pub(super) action: ManagementAction,
    pub(super) management_tlv: Tlv,
}

impl ManagementMessage {
    pub fn content_size(&self) -> usize {
        10
    }

    pub fn serialize_content(
        &self,
        buffer: &mut [u8],
    ) -> Result<(), crate::datastructures::WireFormatError> {
        self.target_port_identity.serialize(&mut buffer[0..10])?;
        buffer[11] = self.starting_boundary_hops;
        buffer[12] = self.boundary_hops;
        buffer[13] = self.action.to_primitive();
        Tlv::serialize(&self.management_tlv, &mut buffer[14..])?;

        Ok(())
    }

    pub fn deserialize_content(
        header: Header,
        buffer: &[u8],
    ) -> Result<Self, crate::datastructures::WireFormatError> {
        if buffer.len() < 14 {
            return Err(WireFormatError::BufferTooShort);
        }
        Ok(Self {
            header,
            target_port_identity: PortIdentity::deserialize(&buffer[0..10])?,
            starting_boundary_hops: buffer[11],
            boundary_hops: buffer[12],
            action: ManagementAction::from_primitive(buffer[13]),
            management_tlv: Tlv::deserialize(&buffer[13..])?,
        })
    }
}

/// See: 15.4.1.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum ManagementAction {
    Reserved,
    GET,
    SET,
    RESPONSE,
    COMMAND,
    ACKNOWLEDGE,
}

impl ManagementAction {
    pub fn to_primitive(self) -> u8 {
        match self {
            Self::GET => 0x0,
            Self::SET => 0x1,
            Self::RESPONSE => 0x2,
            Self::COMMAND => 0x3,
            Self::ACKNOWLEDGE => 0x4,
            Self::Reserved => 0x5,
        }
    }

    pub fn from_primitive(value: u8) -> Self {
        match value {
            0x0 => Self::GET,
            0x1 => Self::SET,
            0x2 => Self::RESPONSE,
            0x3 => Self::COMMAND,
            0x4 => Self::ACKNOWLEDGE,
            0x5..=u8::MAX => Self::Reserved,
        }
    }
}
