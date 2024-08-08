use arrayvec::ArrayVec;

use super::{state::PortState, ForwardedTLVProvider, Port, PortActionIterator, Running};
use crate::{
    datastructures::{
        common::{PortIdentity, Tlv, TlvSetBuilder, TlvType},
        messages::{DelayReqMessage, Header, Message, MAX_DATA_LEN},
    },
    filters::Filter,
    port::{actions::TimestampContextInner, PortAction, TimestampContext},
    ptp_instance::PtpInstanceStateMutex,
    time::Time,
};

impl<'a, A, C, F: Filter, R, S: PtpInstanceStateMutex> Port<'a, Running, A, R, C, F, S> {
    pub(super) fn send_sync(&mut self) -> PortActionIterator {
        if matches!(self.port_state, PortState::Master) {
            log::trace!("sending sync message");

            let seq_id = self.sync_seq_ids.generate();
            let packet_length = match self
                .instance_state
                .with_ref(|state| Message::sync(&state.default_ds, self.port_identity, seq_id))
                .serialize(&mut self.packet_buffer)
            {
                Ok(message) => message,
                Err(error) => {
                    log::error!("Statime bug: Could not serialize sync: {:?}", error);
                    return actions![];
                }
            };

            actions![
                PortAction::ResetSyncTimer {
                    duration: self.config.sync_interval.as_core_duration(),
                },
                PortAction::SendEvent {
                    context: TimestampContext {
                        inner: TimestampContextInner::Sync { id: seq_id },
                    },
                    data: &self.packet_buffer[..packet_length],
                    link_local: false,
                }
            ]
        } else {
            actions![]
        }
    }

    pub(super) fn handle_sync_timestamp(&mut self, id: u16, timestamp: Time) -> PortActionIterator {
        if matches!(self.port_state, PortState::Master) {
            let packet_length = match self
                .instance_state
                .with_ref(|state| {
                    Message::follow_up(&state.default_ds, self.port_identity, id, timestamp)
                })
                .serialize(&mut self.packet_buffer)
            {
                Ok(length) => length,
                Err(error) => {
                    log::error!(
                        "Statime bug: Could not serialize sync follow up {:?}",
                        error
                    );
                    return actions![];
                }
            };

            actions![PortAction::SendGeneral {
                data: &self.packet_buffer[..packet_length],
                link_local: false,
            }]
        } else {
            actions![]
        }
    }

    pub(super) fn send_announce(
        &mut self,
        tlv_provider: &mut impl ForwardedTLVProvider,
    ) -> PortActionIterator {
        if matches!(self.port_state, PortState::Master) {
            log::trace!("sending announce message");

            let mut tlv_buffer = [0; MAX_DATA_LEN];
            let mut tlv_builder = TlvSetBuilder::new(&mut tlv_buffer);

            let mut message = self.instance_state.with_ref(|state| {
                Message::announce(state, self.port_identity, self.announce_seq_ids.generate())
            });
            let mut tlv_margin = MAX_DATA_LEN - message.wire_size();

            let path_trace_enabled = self.instance_state.with_ref(|state| {
                let default_ds = &state.default_ds;
                let path_trace_ds = &state.path_trace_ds;
                if path_trace_ds.enable {
                    'path_trace: {
                        let mut path = path_trace_ds.list.clone();
                        if path.try_push(default_ds.clock_identity).is_err() {
                            break 'path_trace;
                        }

                        let value: ArrayVec<u8, MAX_DATA_LEN> =
                            path.into_iter().flat_map(|ci| ci.0).collect();
                        let tlv = Tlv {
                            tlv_type: TlvType::PathTrace,
                            value: value.as_slice().into(),
                        };

                        let tlv_size = tlv.wire_size();
                        if tlv_margin > tlv_size {
                            tlv_margin -= tlv_size;
                            // Will not fail as previous checks ensure sufficient space in buffer.
                            tlv_builder.add(tlv).unwrap();
                        }
                    }
                }

                path_trace_ds.enable
            });

            while let Some(tlv) = tlv_provider.next_if_smaller(tlv_margin) {
                assert!(tlv.size() < tlv_margin);
                let parent_port_identity = self
                    .instance_state
                    .with_ref(|s| s.parent_ds.parent_port_identity);
                if parent_port_identity != tlv.sender_identity {
                    // Ignore, shouldn't be forwarded
                    continue;
                }

                // Don't forward PATH_TRACE TLVs, we processed them and added our own
                if path_trace_enabled && tlv.tlv.tlv_type == TlvType::PathTrace {
                    continue;
                }

                tlv_margin -= tlv.size();
                // Will not fail as previous checks ensure sufficient space in buffer.
                tlv_builder.add(tlv.tlv).unwrap();
            }

            message.suffix = tlv_builder.build();

            let packet_length = match message.serialize(&mut self.packet_buffer) {
                Ok(length) => length,
                Err(error) => {
                    log::error!(
                        "Statime bug: Could not serialize announce message {:?}",
                        error
                    );
                    return actions![];
                }
            };

            actions![
                PortAction::ResetAnnounceTimer {
                    duration: self.config.announce_interval.as_core_duration(),
                },
                PortAction::SendGeneral {
                    data: &self.packet_buffer[..packet_length],
                    link_local: false,
                }
            ]
        } else {
            actions![]
        }
    }

    pub(super) fn handle_delay_req(
        &mut self,
        header: Header,
        message: DelayReqMessage,
        timestamp: Time,
    ) -> PortActionIterator {
        if matches!(self.port_state, PortState::Master) {
            log::debug!("Received DelayReq");
            let delay_resp_message = Message::delay_resp(
                header,
                message,
                self.port_identity,
                self.config.min_delay_req_interval(),
                timestamp,
            );

            let packet_length = match delay_resp_message.serialize(&mut self.packet_buffer) {
                Ok(length) => length,
                Err(error) => {
                    log::error!("Could not serialize delay response: {:?}", error);
                    return actions![];
                }
            };

            actions![PortAction::SendGeneral {
                data: &self.packet_buffer[..packet_length],
                link_local: false,
            }]
        } else {
            actions![]
        }
    }

    pub(super) fn handle_pdelay_req(
        &mut self,
        header: Header,
        timestamp: Time,
    ) -> PortActionIterator {
        log::debug!("Received PDelayReq");
        let pdelay_resp_message = self.instance_state.with_ref(|state| {
            Message::pdelay_resp(&state.default_ds, self.port_identity, header, timestamp)
        });

        let packet_length = match pdelay_resp_message.serialize(&mut self.packet_buffer) {
            Ok(length) => length,
            Err(error) => {
                log::error!("Could not serialize pdelay response: {:?}", error);
                return actions![];
            }
        };

        actions![PortAction::SendEvent {
            data: &self.packet_buffer[..packet_length],
            context: TimestampContext {
                inner: TimestampContextInner::PDelayResp {
                    id: header.sequence_id,
                    requestor_identity: header.source_port_identity
                }
            },
            link_local: true,
        }]
    }

    pub(super) fn handle_pdelay_response_timestamp(
        &mut self,
        id: u16,
        requestor_identity: PortIdentity,
        timestamp: Time,
    ) -> PortActionIterator {
        let pdelay_resp_follow_up_messgae = self.instance_state.with_ref(|state| {
            Message::pdelay_resp_follow_up(
                &state.default_ds,
                self.port_identity,
                requestor_identity,
                id,
                timestamp,
            )
        });

        let packet_length = match pdelay_resp_follow_up_messgae.serialize(&mut self.packet_buffer) {
            Ok(length) => length,
            Err(error) => {
                log::error!("Could not serialize pdelay_response_followup: {:?}", error);
                return actions![];
            }
        };

        actions![PortAction::SendGeneral {
            data: &self.packet_buffer[..packet_length],
            link_local: true,
        }]
    }
}

#[cfg(test)]
mod tests {
    use fixed::types::{I48F16, U96F32};

    use super::*;
    use crate::{
        config::DelayMechanism,
        datastructures::{
            common::{PortIdentity, TimeInterval},
            datasets::PathTraceDS,
            messages::{Header, MessageBody},
        },
        port::{
            tests::{setup_test_port, setup_test_state},
            NoForwardedTLVs,
        },
        time::Interval,
    };

    #[test]
    fn test_delay_response() {
        let state = setup_test_state();

        let mut port = setup_test_port(&state);

        port.set_forced_port_state(PortState::Master);

        port.config.delay_mechanism = DelayMechanism::E2E {
            interval: Interval::from_log_2(2),
        };

        let mut action = port.handle_delay_req(
            Header {
                sequence_id: 5123,
                source_port_identity: PortIdentity {
                    port_number: 83,
                    ..Default::default()
                },
                correction_field: TimeInterval(I48F16::from_bits(400)),
                ..Default::default()
            },
            DelayReqMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_fixed_nanos(U96F32::from_bits((200000 << 32) + (500 << 16))),
        );

        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = action.next()
        else {
            panic!("Unexpected resulting action");
        };
        assert!(action.next().is_none());
        drop(action);

        let msg = Message::deserialize(data).unwrap();
        let msg_header = msg.header;

        let msg = match msg.body {
            MessageBody::DelayResp(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_eq!(
            msg.requesting_port_identity,
            PortIdentity {
                port_number: 83,
                ..Default::default()
            }
        );
        assert_eq!(msg_header.sequence_id, 5123);
        assert_eq!(msg.receive_timestamp, Time::from_micros(200).into());
        assert_eq!(msg_header.log_message_interval, 2);
        assert_eq!(
            msg_header.correction_field,
            TimeInterval(I48F16::from_bits(900))
        );

        port.config.delay_mechanism = DelayMechanism::E2E {
            interval: Interval::from_log_2(5),
        };

        let mut action = port.handle_delay_req(
            Header {
                sequence_id: 879,
                source_port_identity: PortIdentity {
                    port_number: 12,
                    ..Default::default()
                },
                correction_field: TimeInterval(I48F16::from_bits(200)),
                ..Default::default()
            },
            DelayReqMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_fixed_nanos(U96F32::from_bits((220000 << 32) + (300 << 16))),
        );

        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = action.next()
        else {
            panic!("Unexpected resulting action");
        };
        assert!(action.next().is_none());

        let msg = Message::deserialize(data).unwrap();
        let msg_header = msg.header;

        let msg = match msg.body {
            MessageBody::DelayResp(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_eq!(
            msg.requesting_port_identity,
            PortIdentity {
                port_number: 12,
                ..Default::default()
            }
        );
        assert_eq!(msg_header.sequence_id, 879);
        assert_eq!(msg.receive_timestamp, Time::from_micros(220).into());
        assert_eq!(msg_header.log_message_interval, 5);
        assert_eq!(
            msg_header.correction_field,
            TimeInterval(I48F16::from_bits(500))
        );
    }

    #[test]
    fn test_announce() {
        let state = setup_test_state();

        let mut state_ref = state.borrow_mut();
        state_ref.default_ds.priority_1 = 15;
        state_ref.default_ds.priority_2 = 128;
        state_ref.parent_ds.grandmaster_priority_1 = 15;
        state_ref.parent_ds.grandmaster_priority_2 = 128;

        drop(state_ref);

        let mut port = setup_test_port(&state);

        port.set_forced_port_state(PortState::Master);

        let mut actions = port.send_announce(&mut NoForwardedTLVs);

        assert!(matches!(
            actions.next(),
            Some(PortAction::ResetAnnounceTimer { .. })
        ));
        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let msg = Message::deserialize(data).unwrap();
        let msg_header = msg.header;

        let msg_body = match msg.body {
            MessageBody::Announce(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_eq!(msg_body.grandmaster_priority_1, 15);
        assert_eq!(msg.suffix, Default::default());

        let mut actions = port.send_announce(&mut NoForwardedTLVs);

        assert!(matches!(
            actions.next(),
            Some(PortAction::ResetAnnounceTimer { .. })
        ));
        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());

        let msg2 = Message::deserialize(data).unwrap();
        let msg2_header = msg2.header;

        let msg2_body = match msg2.body {
            MessageBody::Announce(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_eq!(msg2_body.grandmaster_priority_1, 15);
        assert_eq!(msg2.suffix, Default::default());
        assert_ne!(msg2_header.sequence_id, msg_header.sequence_id);
    }

    #[test]
    fn test_announce_path_trace() {
        let state = setup_test_state();

        let mut state_ref = state.borrow_mut();
        state_ref.default_ds.priority_1 = 15;
        state_ref.default_ds.priority_2 = 128;
        state_ref.parent_ds.grandmaster_priority_1 = 15;
        state_ref.parent_ds.grandmaster_priority_2 = 128;
        state_ref.path_trace_ds = PathTraceDS::new(true);

        drop(state_ref);

        let mut port = setup_test_port(&state);

        port.set_forced_port_state(PortState::Master);

        let mut actions = port.send_announce(&mut NoForwardedTLVs);

        assert!(matches!(
            actions.next(),
            Some(PortAction::ResetAnnounceTimer { .. })
        ));
        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let msg = Message::deserialize(data).unwrap();

        let msg_body = match msg.body {
            MessageBody::Announce(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_eq!(msg_body.grandmaster_priority_1, 15);

        let mut tlvs = msg.suffix.tlv();
        let Some(Tlv {
            tlv_type: TlvType::PathTrace,
            value,
        }) = tlvs.next()
        else {
            panic!("Unexpected or missing TLV")
        };
        assert_eq!(value.as_ref(), [0; 8].as_ref());
        assert!(tlvs.next().is_none());
    }

    #[test]
    fn test_sync() {
        let state = setup_test_state();

        let mut state_ref = state.borrow_mut();
        state_ref.default_ds.priority_1 = 15;
        state_ref.default_ds.priority_2 = 128;
        state_ref.parent_ds.grandmaster_priority_1 = 15;
        state_ref.parent_ds.grandmaster_priority_2 = 128;

        drop(state_ref);

        let mut port = setup_test_port(&state);

        port.set_forced_port_state(PortState::Master);
        let mut actions = port.send_sync();

        assert!(matches!(
            actions.next(),
            Some(PortAction::ResetSyncTimer { .. })
        ));
        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let sync = Message::deserialize(data).unwrap();
        let sync_header = sync.header;

        let _sync = match sync.body {
            MessageBody::Sync(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        let id = match context.inner {
            TimestampContextInner::Sync { id } => id,
            _ => panic!("Wrong type of context"),
        };

        let mut actions = port.handle_sync_timestamp(
            id,
            Time::from_fixed_nanos(U96F32::from_bits((601300 << 32) + (230 << 16))),
        );

        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let follow = Message::deserialize(data).unwrap();
        let follow_header = follow.header;

        let follow = match follow.body {
            MessageBody::FollowUp(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_eq!(sync_header.sequence_id, follow_header.sequence_id);
        assert_eq!(
            sync_header.correction_field,
            TimeInterval(I48F16::from_bits(0))
        );
        assert_eq!(
            follow.precise_origin_timestamp,
            Time::from_fixed_nanos(601300).into()
        );
        assert_eq!(
            follow_header.correction_field,
            TimeInterval(I48F16::from_bits(230))
        );

        let mut actions = port.send_sync();

        assert!(matches!(
            actions.next(),
            Some(PortAction::ResetSyncTimer { .. })
        ));
        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let sync2 = Message::deserialize(data).unwrap();
        let sync2_header = sync2.header;

        let _sync2 = match sync2.body {
            MessageBody::Sync(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        let id = match context.inner {
            TimestampContextInner::Sync { id } => id,
            _ => panic!("wrong type of context"),
        };

        let mut actions = port.handle_sync_timestamp(
            id,
            Time::from_fixed_nanos(U96F32::from_bits((1000601300 << 32) + (543 << 16))),
        );

        let Some(PortAction::SendGeneral {
            data,
            link_local: false,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());

        let follow2 = Message::deserialize(data).unwrap();
        let follow2_header = follow2.header;

        let follow2 = match follow2.body {
            MessageBody::FollowUp(msg) => msg,
            _ => panic!("Unexpected message type"),
        };

        assert_ne!(sync_header.sequence_id, sync2_header.sequence_id);
        assert_eq!(sync2_header.sequence_id, follow2_header.sequence_id);
        assert_eq!(
            sync2_header.correction_field,
            TimeInterval(I48F16::from_bits(0))
        );
        assert_eq!(
            follow2.precise_origin_timestamp,
            Time::from_fixed_nanos(1000601300).into()
        );
        assert_eq!(
            follow2_header.correction_field,
            TimeInterval(I48F16::from_bits(543))
        );
    }

    #[test]
    fn test_peer_delay() {
        let state = setup_test_state();

        let mut port = setup_test_port(&state);

        let mut actions = port.handle_pdelay_req(Header::default(), Time::from_micros(500));

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };

        let response = Message::deserialize(data).unwrap();
        let MessageBody::PDelayResp(response_body) = response.body else {
            panic!("Unexpected message sent by port");
        };
        assert_eq!(
            response_body.request_receive_timestamp,
            Time::from_micros(500).into()
        );
        drop(response);
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_send_timestamp(context, Time::from_micros(550));

        let Some(PortAction::SendGeneral {
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };

        let response = Message::deserialize(data).unwrap();
        let MessageBody::PDelayRespFollowUp(response_body) = response.body else {
            panic!("Unexpected message sent by port");
        };
        assert_eq!(
            response_body.response_origin_timestamp,
            Time::from_micros(550).into()
        );
        drop(response);
        assert!(actions.next().is_none());
        drop(actions);
    }
}
