use crate::datastructures::{
    common::{PortIdentity, TLV},
    WireFormat, WireFormatError,
};

use super::{Header, ManagementAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagementMessage {
    pub(super) header: Header,
    pub(super) target_port_identity: PortIdentity,
    pub(super) starting_boundary_hops: u8,
    pub(super) boundary_hops: u8,
    pub(super) action: ManagementAction,
    pub(super) management_tlv: TLV,
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
        TLV::serialize(&self.management_tlv, &mut buffer[14..])?;

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
            management_tlv: TLV::deserialize(&buffer[13..])?,
        })
    }
}
