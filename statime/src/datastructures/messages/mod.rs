//! Ptp network messages

pub(crate) use announce::*;
pub(crate) use delay_req::*;
pub(crate) use delay_resp::*;
pub(crate) use follow_up::*;
pub use header::*;
pub(crate) use sync::*;

use self::{
    management::ManagementMessage, p_delay_req::PDelayReqMessage, p_delay_resp::PDelayRespMessage,
    p_delay_resp_follow_up::PDelayRespFollowUpMessage, signalling::SignalingMessage,
};
use super::{
    common::{PortIdentity, TimeInterval, Tlv, TlvSet, TlvType, WireTimestamp},
    datasets::DefaultDS,
    WireFormatError,
};
use crate::{
    config::LeapIndicator,
    crypto::{NoSpaceError, SecurityAssociation, SecurityAssociationProvider, SenderIdentificaton},
    ptp_instance::PtpInstanceState,
    time::{Interval, Time},
};

mod announce;
mod control_field;
mod delay_req;
mod delay_resp;
mod follow_up;
mod header;
mod management;
mod p_delay_req;
mod p_delay_resp;
mod p_delay_resp_follow_up;
mod signalling;
mod sync;

/// Maximum length of a packet
///
/// This can be used to preallocate buffers that can always fit packets send by
/// `statime`.
pub const MAX_DATA_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum MessageType {
    Sync = 0x0,
    DelayReq = 0x1,
    PDelayReq = 0x2,
    PDelayResp = 0x3,
    FollowUp = 0x8,
    DelayResp = 0x9,
    PDelayRespFollowUp = 0xa,
    Announce = 0xb,
    Signaling = 0xc,
    Management = 0xd,
}

pub struct EnumConversionError;

impl TryFrom<u8> for MessageType {
    type Error = EnumConversionError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        use MessageType::*;

        match value {
            0x0 => Ok(Sync),
            0x1 => Ok(DelayReq),
            0x2 => Ok(PDelayReq),
            0x3 => Ok(PDelayResp),
            0x8 => Ok(FollowUp),
            0x9 => Ok(DelayResp),
            0xa => Ok(PDelayRespFollowUp),
            0xb => Ok(Announce),
            0xc => Ok(Signaling),
            0xd => Ok(Management),
            _ => Err(EnumConversionError),
        }
    }
}

#[cfg(feature = "fuzz")]
pub use fuzz::FuzzMessage;

#[cfg(feature = "fuzz")]
mod fuzz {
    #![allow(missing_docs)] // These are only used for internal fuzzing
    use super::*;
    use crate::datastructures::{common::Tlv, WireFormatError};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FuzzMessage<'a> {
        inner: Message<'a>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct FuzzTlv<'a>(Tlv<'a>);

    impl<'a> FuzzMessage<'a> {
        pub fn deserialize(buffer: &'a [u8]) -> Result<Self, impl std::error::Error> {
            Ok::<FuzzMessage, WireFormatError>(FuzzMessage {
                inner: Message::deserialize(buffer)?,
            })
        }

        pub fn serialize(&self, buffer: &mut [u8]) -> Result<usize, impl std::error::Error> {
            self.inner.serialize(buffer)
        }

        pub fn tlv(&self) -> impl Iterator<Item = FuzzTlv<'_>> + '_ {
            self.inner.suffix.tlv().map(FuzzTlv)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message<'a> {
    pub(crate) header: Header,
    pub(crate) body: MessageBody,
    pub(crate) suffix: TlvSet<'a>,
}

impl<'a> Message<'a> {
    pub(crate) fn is_event(&self) -> bool {
        use MessageBody::*;
        match self.body {
            Sync(_) | DelayReq(_) | PDelayReq(_) | PDelayResp(_) => true,
            FollowUp(_)
            | DelayResp(_)
            | PDelayRespFollowUp(_)
            | Announce(_)
            | Signaling(_)
            | Management(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MessageBody {
    Sync(SyncMessage),
    DelayReq(DelayReqMessage),
    PDelayReq(PDelayReqMessage),
    PDelayResp(PDelayRespMessage),
    FollowUp(FollowUpMessage),
    DelayResp(DelayRespMessage),
    PDelayRespFollowUp(PDelayRespFollowUpMessage),
    Announce(AnnounceMessage),
    Signaling(SignalingMessage),
    Management(ManagementMessage),
}

impl MessageBody {
    fn wire_size(&self) -> usize {
        match &self {
            MessageBody::Sync(m) => m.content_size(),
            MessageBody::DelayReq(m) => m.content_size(),
            MessageBody::PDelayReq(m) => m.content_size(),
            MessageBody::PDelayResp(m) => m.content_size(),
            MessageBody::FollowUp(m) => m.content_size(),
            MessageBody::DelayResp(m) => m.content_size(),
            MessageBody::PDelayRespFollowUp(m) => m.content_size(),
            MessageBody::Announce(m) => m.content_size(),
            MessageBody::Signaling(m) => m.content_size(),
            MessageBody::Management(m) => m.content_size(),
        }
    }

    fn content_type(&self) -> MessageType {
        match self {
            MessageBody::Sync(_) => MessageType::Sync,
            MessageBody::DelayReq(_) => MessageType::DelayReq,
            MessageBody::PDelayReq(_) => MessageType::PDelayReq,
            MessageBody::PDelayResp(_) => MessageType::PDelayResp,
            MessageBody::FollowUp(_) => MessageType::FollowUp,
            MessageBody::DelayResp(_) => MessageType::DelayResp,
            MessageBody::PDelayRespFollowUp(_) => MessageType::PDelayRespFollowUp,
            MessageBody::Announce(_) => MessageType::Announce,
            MessageBody::Signaling(_) => MessageType::Signaling,
            MessageBody::Management(_) => MessageType::Management,
        }
    }

    pub(crate) fn serialize(&self, buffer: &mut [u8]) -> Result<usize, super::WireFormatError> {
        match &self {
            MessageBody::Sync(m) => m.serialize_content(buffer)?,
            MessageBody::DelayReq(m) => m.serialize_content(buffer)?,
            MessageBody::PDelayReq(m) => m.serialize_content(buffer)?,
            MessageBody::PDelayResp(m) => m.serialize_content(buffer)?,
            MessageBody::FollowUp(m) => m.serialize_content(buffer)?,
            MessageBody::DelayResp(m) => m.serialize_content(buffer)?,
            MessageBody::PDelayRespFollowUp(m) => m.serialize_content(buffer)?,
            MessageBody::Announce(m) => m.serialize_content(buffer)?,
            MessageBody::Signaling(m) => m.serialize_content(buffer)?,
            MessageBody::Management(m) => m.serialize_content(buffer)?,
        }

        Ok(self.wire_size())
    }

    pub(crate) fn deserialize(
        message_type: MessageType,
        header: &Header,
        buffer: &[u8],
    ) -> Result<Self, super::WireFormatError> {
        let body = match message_type {
            MessageType::Sync => MessageBody::Sync(SyncMessage::deserialize_content(buffer)?),
            MessageType::DelayReq => {
                MessageBody::DelayReq(DelayReqMessage::deserialize_content(buffer)?)
            }
            MessageType::PDelayReq => {
                MessageBody::PDelayReq(PDelayReqMessage::deserialize_content(buffer)?)
            }
            MessageType::PDelayResp => {
                MessageBody::PDelayResp(PDelayRespMessage::deserialize_content(buffer)?)
            }
            MessageType::FollowUp => {
                MessageBody::FollowUp(FollowUpMessage::deserialize_content(buffer)?)
            }
            MessageType::DelayResp => {
                MessageBody::DelayResp(DelayRespMessage::deserialize_content(buffer)?)
            }
            MessageType::PDelayRespFollowUp => MessageBody::PDelayRespFollowUp(
                PDelayRespFollowUpMessage::deserialize_content(buffer)?,
            ),
            MessageType::Announce => {
                MessageBody::Announce(AnnounceMessage::deserialize_content(*header, buffer)?)
            }
            MessageType::Signaling => {
                MessageBody::Signaling(SignalingMessage::deserialize_content(buffer)?)
            }
            MessageType::Management => {
                MessageBody::Management(ManagementMessage::deserialize_content(buffer)?)
            }
        };

        Ok(body)
    }
}

fn base_header(default_ds: &DefaultDS, port_identity: PortIdentity, sequence_id: u16) -> Header {
    Header {
        sdo_id: default_ds.sdo_id,
        domain_number: default_ds.domain_number,
        source_port_identity: port_identity,
        sequence_id,
        ..Default::default()
    }
}

impl<'a> Message<'a> {
    #[allow(unused)]
    pub(crate) fn add_signature(
        &mut self,
        spp: u8,
        provider: &impl SecurityAssociationProvider,
        backing_buffer: &'a mut [u8],
    ) -> Result<(), NoSpaceError> {
        let association = provider.lookup(spp).ok_or(NoSpaceError)?;
        let (key_id, key) = association.signing_mac();

        // partial authentication tlv
        let mut temp_tlv_value = [0; MAX_DATA_LEN];
        temp_tlv_value[0] = spp;
        temp_tlv_value[1] = 0;
        temp_tlv_value[2..6].copy_from_slice(&key_id.to_be_bytes());

        // Regenerate the tlv set with partial authentication tlv
        let mut temp_message = self.clone();
        temp_message.suffix = self
            .suffix
            .extend_with(
                Tlv {
                    tlv_type: TlvType::Authentication,
                    value: temp_tlv_value[0..6 + key.output_size()].into(),
                },
                backing_buffer,
            )
            .ok_or(NoSpaceError)?;

        // generate tag
        let mut temp_buffer = [0; MAX_DATA_LEN];
        temp_message
            .serialize(&mut temp_buffer)
            .map_err(|_| NoSpaceError)?;
        if association.policy_data().ignore_correction {
            // zero out correction field.
            temp_buffer[8..16].fill(0)
        }
        key.sign(
            &temp_buffer[..self.wire_size() + 10],
            &mut temp_tlv_value[6..],
        )?;

        // Generate final tlv set
        self.suffix = self
            .suffix
            .extend_with(
                Tlv {
                    tlv_type: TlvType::Authentication,
                    value: temp_tlv_value[0..6 + key.output_size()].into(),
                },
                backing_buffer,
            )
            .ok_or(NoSpaceError)?;

        Ok(())
    }
}

impl Message<'_> {
    #[allow(unused)]
    pub(crate) fn verify_signed(&self, provider: &impl SecurityAssociationProvider) -> bool {
        log::trace!("Validation message");
        let mut tlv_offset = 0;
        for tlv in self.suffix.tlv() {
            if tlv.tlv_type == TlvType::Authentication {
                // Check we have at least the SPP, params and key id
                if tlv.value.len() < 6 {
                    log::trace!("Rejected: Incorrect authentication tlv length");
                    return false;
                }

                let spp = tlv.value[0];
                let params = tlv.value[1];
                let key_id = u32::from_be_bytes(tlv.value[2..6].try_into().unwrap());

                // we dont support presence of any of the optional bits, so not valid if those
                // are present
                if params != 0 {
                    log::trace!("Rejected: Unexpected optional bits");
                    return false;
                }

                // get the security association and key
                let Some(mut association) = provider.lookup(spp) else {
                    log::trace!("Rejected: Invalid spp");
                    return false;
                };
                let Some(key) = association.mac(key_id) else {
                    log::trace!("Rejected: Invalid key id");
                    return false;
                };

                // Ensure we have a complete ICV
                if tlv.value.len() < 6 + key.output_size() {
                    log::trace!("Rejected: TLV too short");
                    return false;
                }

                // We need the raw packet data for the signature, so serialize again
                // TODO: bad practice to reserialize for checking signatures, should be fixed
                // before production ready
                let mut buffer = [0; MAX_DATA_LEN];
                if self.serialize(&mut buffer).is_err() {
                    log::trace!("Rejected: cannot reserialize");
                    return false;
                }
                if association.policy_data().ignore_correction {
                    // zero out correction field.
                    buffer[8..16].fill(0)
                }

                if !key.verify(
                    &buffer[..self.header.wire_size() + self.body.wire_size() + tlv_offset + 10],
                    &tlv.value[6..6 + key.output_size()],
                ) {
                    log::trace!("Rejected: signature invalid");
                    return false;
                }

                // Check sequence id is acceptable
                association.register_sequence_id(
                    key_id,
                    SenderIdentificaton {
                        message_type: self.body.content_type(),
                        source_port_id: self.header.source_port_identity,
                    },
                    self.header.sequence_id,
                );

                log::trace!("Accepted");
                return true;
            }

            tlv_offset += tlv.wire_size();
        }

        // No authentication tlv found, so not signed
        log::trace!("Rejected: no TLV present");
        false
    }

    pub(crate) fn sync(
        default_ds: &DefaultDS,
        port_identity: PortIdentity,
        sequence_id: u16,
    ) -> Self {
        let header = Header {
            two_step_flag: true,
            ..base_header(default_ds, port_identity, sequence_id)
        };

        Message {
            header,
            body: MessageBody::Sync(SyncMessage {
                origin_timestamp: Default::default(),
            }),
            suffix: TlvSet::default(),
        }
    }

    pub(crate) fn follow_up(
        default_ds: &DefaultDS,
        port_identity: PortIdentity,
        sequence_id: u16,
        timestamp: Time,
    ) -> Self {
        let header = Header {
            correction_field: timestamp.subnano(),
            ..base_header(default_ds, port_identity, sequence_id)
        };

        Message {
            header,
            body: MessageBody::FollowUp(FollowUpMessage {
                precise_origin_timestamp: timestamp.into(),
            }),
            suffix: TlvSet::default(),
        }
    }

    pub(crate) fn announce(
        global: &PtpInstanceState,
        port_identity: PortIdentity,
        sequence_id: u16,
    ) -> Self {
        let time_properties_ds = &global.time_properties_ds;

        let header = Header {
            leap59: time_properties_ds.leap_indicator == LeapIndicator::Leap59,
            leap61: time_properties_ds.leap_indicator == LeapIndicator::Leap61,
            current_utc_offset_valid: time_properties_ds.current_utc_offset.is_some(),
            ptp_timescale: time_properties_ds.ptp_timescale,
            time_tracable: time_properties_ds.time_traceable,
            frequency_tracable: time_properties_ds.frequency_traceable,
            ..base_header(&global.default_ds, port_identity, sequence_id)
        };

        let body = MessageBody::Announce(AnnounceMessage {
            header,
            origin_timestamp: Default::default(),
            current_utc_offset: time_properties_ds.current_utc_offset.unwrap_or_default(),
            grandmaster_priority_1: global.parent_ds.grandmaster_priority_1,
            grandmaster_clock_quality: global.parent_ds.grandmaster_clock_quality,
            grandmaster_priority_2: global.parent_ds.grandmaster_priority_2,
            grandmaster_identity: global.parent_ds.grandmaster_identity,
            steps_removed: global.current_ds.steps_removed,
            time_source: time_properties_ds.time_source,
        });

        Message {
            header,
            body,
            suffix: TlvSet::default(),
        }
    }

    pub(crate) fn delay_req(
        default_ds: &DefaultDS,
        port_identity: PortIdentity,
        sequence_id: u16,
    ) -> Self {
        let header = Header {
            log_message_interval: 0x7f,
            ..base_header(default_ds, port_identity, sequence_id)
        };

        Message {
            header,
            body: MessageBody::DelayReq(DelayReqMessage {
                origin_timestamp: WireTimestamp::default(),
            }),
            suffix: TlvSet::default(),
        }
    }

    pub(crate) fn delay_resp(
        request_header: Header,
        request: DelayReqMessage,
        port_identity: PortIdentity,
        min_delay_req_interval: Interval,
        timestamp: Time,
    ) -> Self {
        // TODO is it really correct that we don't use any of the data?
        let _ = request;

        let header = Header {
            two_step_flag: false,
            source_port_identity: port_identity,
            correction_field: TimeInterval(
                request_header.correction_field.0 + timestamp.subnano().0,
            ),
            log_message_interval: min_delay_req_interval.as_log_2(),
            ..request_header
        };

        let body = MessageBody::DelayResp(DelayRespMessage {
            receive_timestamp: timestamp.into(),
            requesting_port_identity: request_header.source_port_identity,
        });

        Message {
            header,
            body,
            suffix: TlvSet::default(),
        }
    }
}

impl<'a> Message<'a> {
    pub(crate) fn header(&self) -> &Header {
        &self.header
    }

    /// The byte size on the wire of this message
    pub(crate) fn wire_size(&self) -> usize {
        self.header.wire_size() + self.body.wire_size() + self.suffix.wire_size()
    }

    /// Serializes the object into the PTP wire format.
    ///
    /// Returns the used buffer size that contains the message or an error.
    pub(crate) fn serialize(&self, buffer: &mut [u8]) -> Result<usize, super::WireFormatError> {
        let (header, rest) = buffer.split_at_mut(34);
        let (body, tlv) = rest.split_at_mut(self.body.wire_size());

        self.header
            .serialize_header(
                self.body.content_type(),
                self.body.wire_size() + self.suffix.wire_size(),
                header,
            )
            .unwrap();

        self.body.serialize(body).unwrap();

        self.suffix.serialize(tlv).unwrap();

        Ok(self.wire_size())
    }

    /// Deserializes a message from the PTP wire format.
    ///
    /// Returns the message or an error.
    pub(crate) fn deserialize(buffer: &'a [u8]) -> Result<Self, super::WireFormatError> {
        let header_data = Header::deserialize_header(buffer)?;

        if header_data.message_length < 34 {
            return Err(WireFormatError::Invalid);
        }

        // Ensure we have the entire message and ignore potential padding
        // Skip the header bytes and only keep the content
        let content_buffer = buffer
            .get(34..(header_data.message_length as usize))
            .ok_or(WireFormatError::BufferTooShort)?;

        let body = MessageBody::deserialize(
            header_data.message_type,
            &header_data.header,
            content_buffer,
        )?;

        let tlv_buffer = &content_buffer
            .get(body.wire_size()..)
            .ok_or(super::WireFormatError::BufferTooShort)?;
        let suffix = TlvSet::deserialize(tlv_buffer)?;

        Ok(Message {
            header: header_data.header,
            body,
            suffix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::InstanceConfig,
        crypto::{HmacSha256_128, SecurityPolicy},
    };

    struct TestSecurityProvider(SecurityPolicy);

    struct TestSecurityAssociation(SecurityPolicy);

    impl SecurityAssociation for TestSecurityAssociation {
        fn policy_data(&self) -> crate::crypto::SecurityPolicy {
            self.0
        }

        fn mac(&self, _key_id: u32) -> Option<&dyn crate::crypto::Mac> {
            Some(std::boxed::Box::leak(std::boxed::Box::new(
                HmacSha256_128::new([0; 32]),
            )))
        }

        fn register_sequence_id(
            &mut self,
            _key_id: u32,
            _sender: crate::crypto::SenderIdentificaton,
            _sequence_id: u16,
        ) -> bool {
            true
        }

        fn signing_mac(&self) -> (u32, &dyn crate::crypto::Mac) {
            (
                0,
                std::boxed::Box::leak(std::boxed::Box::new(HmacSha256_128::new([0; 32]))),
            )
        }
    }

    impl SecurityAssociationProvider for TestSecurityProvider {
        type Association<'a> = TestSecurityAssociation;

        fn lookup(&self, _spp: u8) -> Option<Self::Association<'_>> {
            Some(TestSecurityAssociation(self.0))
        }
    }

    #[test]
    fn test_signing_ignoring_correction() {
        let port_identity = PortIdentity::default();
        let default_ds = DefaultDS::new(InstanceConfig {
            clock_identity: Default::default(),
            priority_1: 128,
            priority_2: 128,
            domain_number: 0,
            sdo_id: Default::default(),
            slave_only: false,
        });

        let provider = TestSecurityProvider(SecurityPolicy {
            ignore_correction: true,
        });

        let mut test_message = Message::sync(&default_ds, port_identity, 1);
        let mut backing_buffer = [0; MAX_DATA_LEN];
        test_message
            .add_signature(0, &provider, &mut backing_buffer)
            .unwrap();

        let mut message_buffer = [0; MAX_DATA_LEN];
        let message_len = test_message.serialize(&mut message_buffer).unwrap();

        // Modify correction field
        message_buffer[9] = message_buffer[9].wrapping_add(9);

        let received_message = Message::deserialize(&message_buffer[..message_len]).unwrap();
        assert!(received_message.verify_signed(&provider));

        // Modify message
        message_buffer[message_len - 1] = message_buffer[message_len - 1].wrapping_add(5);

        let received_message = Message::deserialize(&message_buffer[..message_len]).unwrap();
        assert!(!received_message.verify_signed(&provider));
    }

    #[test]
    fn test_signing() {
        let port_identity = PortIdentity::default();
        let default_ds = DefaultDS::new(InstanceConfig {
            clock_identity: Default::default(),
            priority_1: 128,
            priority_2: 128,
            domain_number: 0,
            sdo_id: Default::default(),
            slave_only: false,
        });

        let provider = TestSecurityProvider(SecurityPolicy {
            ignore_correction: false,
        });

        let mut test_message = Message::sync(&default_ds, port_identity, 1);
        let mut backing_buffer = [0; MAX_DATA_LEN];
        test_message
            .add_signature(0, &provider, &mut backing_buffer)
            .unwrap();

        let mut message_buffer = [0; MAX_DATA_LEN];
        let message_len = test_message.serialize(&mut message_buffer).unwrap();

        let received_message = Message::deserialize(&message_buffer[..message_len]).unwrap();
        assert!(received_message.verify_signed(&provider));

        // Modify correction field
        message_buffer[9] = message_buffer[9].wrapping_add(9);

        let received_message = Message::deserialize(&message_buffer[..message_len]).unwrap();
        assert!(!received_message.verify_signed(&provider));
    }
}
