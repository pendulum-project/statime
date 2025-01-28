//! PTPv1 network messages
pub(crate) use sync_or_delay_req::*;
pub(crate) use delay_resp::*;
pub(crate) use follow_up::*;
pub use header::*;
use crate::PtpInstanceState;

use super::{common::{PortIdentity, WireTimestampV1}, datasets::{InternalDefaultDS, InternalParentDS}, messages::EnumConversionError, WireFormatError};
/* use self::{
    management::ManagementMessage,
}; */
/* use super::{
    WireFormatError,
};
use crate::config::ClockIdentity; */
mod sync_or_delay_req;
mod control_field;
use control_field::ControlField;
mod delay_resp;
mod follow_up;
mod header;

/// Maximum length of a packet
///
/// This can be used to preallocate buffers that can always fit packets send by
/// `statime`.
pub const MAX_DATA_LEN: usize = 255;


/// Checks whether message is PTPv1
pub fn is_compatible(buffer: &[u8]) -> bool {
    // this ensures that versionPTP in the header is 1
    (buffer.len() >= 2) && (buffer[0] == 0) && (buffer[1] == 1)
}


/// Type of message, used to differentiate low-delay and other messages.
/// `Event` is low-delay.
/// 
/// To avoid confusion with PTPv2 MessageType which is functionally equivalent
/// to ControlField in PTPv1, PTPv1's messageType is called here PortType
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PortType {
    Event = 0x1,
    General = 0x2,
}
impl TryFrom<u8> for PortType {
    type Error = EnumConversionError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        use PortType::*;
        match value {
            0x1 => Ok(Event),
            0x2 => Ok(General),
            _ => Err(EnumConversionError),
        }
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) header: Header,
    pub(crate) body: MessageBody,
}

impl Message {
    pub(crate) fn is_event(&self) -> bool {
        use MessageBody::*;
        match self.body {
            Sync(_) | DelayReq(_) => true,
            FollowUp(_) | DelayResp(_) /* | Management(_) */ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MessageBody {
    Sync(SyncMessage),
    DelayReq(DelayReqMessage),
    FollowUp(FollowUpMessage),
    DelayResp(DelayRespMessage),
    //Management(ManagementMessage), // TODO
}
impl MessageBody {
    fn wire_size(&self) -> usize {
        match &self {
            MessageBody::Sync(m) => m.content_size(),
            MessageBody::DelayReq(m) => m.content_size(),
            MessageBody::FollowUp(m) => m.content_size(),
            MessageBody::DelayResp(m) => m.content_size(),
            /* MessageBody::Management(m) => m.content_size(), */
        }
    }
    fn content_type(&self) -> ControlField {
        match self {
            MessageBody::Sync(_) => ControlField::Sync,
            MessageBody::DelayReq(_) => ControlField::DelayReq,
            MessageBody::FollowUp(_) => ControlField::FollowUp,
            MessageBody::DelayResp(_) => ControlField::DelayResp,
            /* MessageBody::Management(_) => MessageType::Management, */
        }
    }
    pub(crate) fn serialize(&self, buffer: &mut [u8]) -> Result<usize, super::WireFormatError> {
        match &self {
            MessageBody::Sync(m) => m.serialize_content(buffer)?,
            MessageBody::DelayReq(m) => m.serialize_content(buffer)?,
            MessageBody::FollowUp(m) => m.serialize_content(buffer)?,
            MessageBody::DelayResp(m) => m.serialize_content(buffer)?,
            /* MessageBody::Management(m) => m.serialize_content(buffer)?, */
        }
        Ok(self.wire_size())
    }
    pub(crate) fn deserialize(
        message_type: ControlField,
        header: &Header,
        buffer: &[u8],
    ) -> Result<Self, super::WireFormatError> {
        let body = match message_type {
            ControlField::Sync => MessageBody::Sync(SyncMessage::deserialize_content(*header, buffer)?),
            ControlField::DelayReq => {
                MessageBody::DelayReq(DelayReqMessage::deserialize_content(*header, buffer)?)
            }
            ControlField::FollowUp => {
                MessageBody::FollowUp(FollowUpMessage::deserialize_content(buffer)?)
            }
            ControlField::DelayResp => {
                MessageBody::DelayResp(DelayRespMessage::deserialize_content(buffer)?)
            }
            /* ControlField::Management => {
                MessageBody::Management(ManagementMessage::deserialize_content(buffer)?)
            } */
        };
        Ok(body)
    }
}

fn base_header(
    _default_ds: &InternalDefaultDS,
    port_identity: PortIdentity,
    sequence_id: u16,
) -> Header {
    Header {
        source_uuid: port_identity.clock_identity.0[0..6].try_into().unwrap(),
        source_port_id: port_identity.port_number,
        sequence_id,
        ..Default::default()
    }
}

impl Message {
    pub(crate) fn header(&self) -> &Header {
        &self.header
    }
    /// The byte size on the wire of this message
    pub(crate) fn wire_size(&self) -> usize {
        self.header.wire_size() + self.body.wire_size()
    }
    /// Serializes the object into the PTP wire format.
    ///
    /// Returns the used buffer size that contains the message or an error.
    pub(crate) fn serialize(&self, buffer: &mut [u8]) -> Result<usize, super::WireFormatError> {
        let (header, rest) = buffer.split_at_mut(40);
        let (body, _tlv) = rest.split_at_mut(self.body.wire_size());
        self.header
            .serialize_header(
                self.body.content_type(),
                self.body.wire_size(),
                header,
            )
            .unwrap();
        self.body.serialize(body).unwrap();
        Ok(self.wire_size())
    }
    /// Deserializes a message from the PTP wire format.
    ///
    /// Returns the message or an error.
    pub(crate) fn deserialize(buffer: &[u8]) -> Result<Self, super::WireFormatError> {
        let header_data = Header::deserialize_header(buffer)?;
        /* if header_data.message_length < 34 {
            return Err(WireFormatError::Invalid);
        } */
        // Ensure we have the entire message and ignore potential padding
        // Skip the header bytes and only keep the content
        let content_buffer = buffer
            .get(40..buffer.len())
            .ok_or(WireFormatError::BufferTooShort)?;
        let body = MessageBody::deserialize(
            header_data.control,
            &header_data.header,
            content_buffer,
        )?;
        Ok(Message {
            header: header_data.header,
            body,
        })
    }

    pub(crate) fn delay_req(
        global: &PtpInstanceState,
        port_identity: PortIdentity,
        sequence_id: u16,
    ) -> Self {
        let header = Header {
            ..base_header(&global.default_ds, port_identity, sequence_id)
        };

        Message {
            header,
            body: MessageBody::DelayReq(DelayReqMessage {
                header,
                origin_timestamp: WireTimestampV1::default(),
                epoch_number: 0, // FIXME
                current_utc_offset: global.time_properties_ds.current_utc_offset.unwrap_or(0),
                // FIXME: what if grandmaster is acquired via PTPv2?
                grandmaster: global.parent_ds.grandmaster_v1.expect("parent_ds.grandmaster_v1 not filled but trying to send DelayReq"),
                sync_interval: 0x7f,
                // TODO really? looks like Dante does it this way but is it correct?
                local_clock_variance: global.default_ds.clock_quality.offset_scaled_log_variance as i16,
                local_steps_removed: global.current_ds.steps_removed,
                local_clock_stratum: 255, // TODO but enough for slave-only
                local_clock_identifier: *b"DFLT", // FIXME
                parent_communication_technology: 1,
                parent_uuid: global.parent_ds.parent_port_identity.clock_identity.0[0..6].try_into().unwrap(),
                parent_port_field: global.parent_ds.parent_port_identity.port_number,
                estimated_master_variance: 0,
                estimated_master_drift: 0,
                utc_reasonable: global.time_properties_ds.current_utc_offset.is_some(),
            }),
        }
    }
}
