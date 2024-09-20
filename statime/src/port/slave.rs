use rand::Rng;

use super::{
    state::{DelayState, PortState},
    Measurement, PeerDelayState, Port, PortActionIterator, Running,
};
use crate::{
    config::DelayMechanism,
    datastructures::messages::{
        DelayRespMessage, FollowUpMessage, Header, Message, PDelayRespFollowUpMessage,
        PDelayRespMessage, SyncMessage,
    },
    filters::Filter,
    port::{actions::TimestampContextInner, state::SyncState, PortAction, TimestampContext},
    ptp_instance::PtpInstanceStateMutex,
    time::{Duration, Interval, Time},
    Clock,
};

impl<'a, A, C: Clock, F: Filter, R, S> Port<'a, Running, A, R, C, F, S> {
    pub(super) fn handle_time_measurement<'b>(&mut self) -> PortActionIterator<'b> {
        if let Some(measurement) = self.extract_measurement() {
            // If the received message allowed the (slave) state to calculate its offset
            // from the master, update the local clock
            let filter_updates = self.filter.measurement(measurement, &mut self.clock);
            if let Some(mean_delay) = filter_updates.mean_delay {
                self.mean_delay = Some(mean_delay);
            }
            PortActionIterator::from_filter(filter_updates)
        } else {
            actions![]
        }
    }

    pub(super) fn handle_delay_timestamp(
        &mut self,
        timestamp_id: u16,
        timestamp: Time,
    ) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => match state.delay_state {
                DelayState::Measuring {
                    id,
                    send_time: Some(_),
                    ..
                } if id == timestamp_id => {
                    log::error!("Double send timestamp for delay request");
                    actions![]
                }
                DelayState::Measuring {
                    id,
                    ref mut send_time,
                    ..
                } if id == timestamp_id => {
                    *send_time = Some(timestamp);
                    self.handle_time_measurement()
                }
                _ => {
                    log::warn!("Late timestamp for delay request ignored");
                    actions![]
                }
            },
            _ => actions![],
        }
    }

    pub(super) fn handle_pdelay_timestamp(
        &mut self,
        timestamp_id: u16,
        timestamp: Time,
    ) -> PortActionIterator {
        match self.peer_delay_state {
            PeerDelayState::Measuring {
                id,
                request_send_time: Some(_),
                ..
            } if id == timestamp_id => {
                log::error!("Double send timestamp for pdelay request");
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                ref mut request_send_time,
                ..
            } if id == timestamp_id => {
                *request_send_time = Some(timestamp);
                self.handle_time_measurement()
            }
            _ => {
                log::warn!("Late timestamp for pdelay request ignored");
                actions![]
            }
        }
    }

    pub(super) fn handle_sync(
        &mut self,
        header: Header,
        message: SyncMessage,
        recv_time: Time,
    ) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                log::debug!("Received sync {:?}", header.sequence_id);
                if state.remote_master != header.source_port_identity {
                    return actions![];
                }

                // substracting correction from recv time is equivalent to adding it to send
                // time
                let corrected_recv_time = recv_time - Duration::from(header.correction_field);

                if header.two_step_flag {
                    match state.sync_state {
                        SyncState::Measuring {
                            id,
                            recv_time: Some(_),
                            ..
                        } if id == header.sequence_id => {
                            log::warn!("Duplicate sync message");
                            // Ignore the sync message
                            actions![]
                        }
                        SyncState::Measuring {
                            id,
                            ref mut recv_time,
                            ..
                        } if id == header.sequence_id => {
                            *recv_time = Some(corrected_recv_time);
                            self.handle_time_measurement()
                        }
                        _ => {
                            state.sync_state = SyncState::Measuring {
                                id: header.sequence_id,
                                send_time: None,
                                recv_time: Some(corrected_recv_time),
                            };
                            actions![]
                        }
                    }
                } else {
                    match state.sync_state {
                        SyncState::Measuring { id, .. } if id == header.sequence_id => {
                            log::warn!("Duplicate sync message");
                            // Ignore the sync message
                            actions![]
                        }
                        _ => {
                            state.sync_state = SyncState::Measuring {
                                id: header.sequence_id,
                                send_time: Some(Time::from(message.origin_timestamp)),
                                recv_time: Some(corrected_recv_time),
                            };
                            self.handle_time_measurement()
                        }
                    }
                }
            }
            _ => actions![],
        }
    }

    pub(super) fn handle_follow_up(
        &mut self,
        header: Header,
        message: FollowUpMessage,
    ) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                log::debug!("Received FollowUp {:?}", header.sequence_id);
                if state.remote_master != header.source_port_identity {
                    return actions![];
                }

                let packet_send_time = Time::from(message.precise_origin_timestamp)
                    + Duration::from(header.correction_field);

                match state.sync_state {
                    SyncState::Measuring {
                        id,
                        send_time: Some(_),
                        ..
                    } if id == header.sequence_id => {
                        log::warn!("Duplicate FollowUp message");
                        // Ignore the followup
                        actions![]
                    }
                    SyncState::Measuring {
                        id,
                        ref mut send_time,
                        ..
                    } if id == header.sequence_id => {
                        *send_time = Some(packet_send_time);
                        self.handle_time_measurement()
                    }
                    _ => {
                        state.sync_state = SyncState::Measuring {
                            id: header.sequence_id,
                            send_time: Some(packet_send_time),
                            recv_time: None,
                        };
                        self.handle_time_measurement()
                    }
                }
            }
            _ => actions![],
        }
    }

    pub(super) fn handle_delay_resp(
        &mut self,
        header: Header,
        message: DelayRespMessage,
    ) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                log::debug!("Received DelayResp");
                if self.port_identity != message.requesting_port_identity
                    || state.remote_master != header.source_port_identity
                {
                    return actions![];
                }

                match state.delay_state {
                    DelayState::Measuring {
                        id,
                        recv_time: Some(_),
                        ..
                    } if id == header.sequence_id => {
                        log::warn!("Duplicate DelayResp message");
                        // Ignore the Delay response
                        actions![]
                    }
                    DelayState::Measuring {
                        id,
                        ref mut recv_time,
                        ..
                    } if id == header.sequence_id => {
                        *recv_time = Some(
                            Time::from(message.receive_timestamp)
                                - Duration::from(header.correction_field),
                        );
                        self.handle_time_measurement()
                    }
                    _ => {
                        log::warn!("Unexpected DelayResp message");
                        // Ignore the Delay response
                        actions![]
                    }
                }
            }
            _ => actions![],
        }
    }

    pub(super) fn handle_peer_delay_response(
        &mut self,
        header: Header,
        message: PDelayRespMessage,
        recv_time: Time,
    ) -> PortActionIterator {
        if self.port_identity != message.requesting_port_identity {
            return actions![];
        }

        match self.peer_delay_state {
            PeerDelayState::PostMeasurement {
                id,
                responder_identity,
            } if id == header.sequence_id && responder_identity != header.source_port_identity => {
                log::error!(
                    "Responses from multiple devices to peer delay request, disabling port!"
                );
                self.set_forced_port_state(PortState::Faulty);
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                responder_identity: Some(identity),
                ..
            } if id == header.sequence_id && identity != header.source_port_identity => {
                log::error!(
                    "Responses from multiple devices to peer delay request, disabling port!"
                );
                self.set_forced_port_state(PortState::Faulty);
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                response_recv_time: Some(_),
                ..
            } if id == header.sequence_id => {
                log::warn!("Duplicate PDelayResp message");
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                ref mut request_recv_time,
                ref mut response_recv_time,
                ref mut response_send_time,
                ref mut responder_identity,
                ..
            } if id == header.sequence_id => {
                *response_recv_time = Some(recv_time - Duration::from(header.correction_field));
                *request_recv_time = Some(message.request_receive_timestamp.into());
                *responder_identity = Some(header.source_port_identity);

                if !header.two_step_flag {
                    *response_send_time = Some(message.request_receive_timestamp.into());
                }
                self.handle_time_measurement()
            }
            _ => {
                log::warn!("Unexpected PDelayResp message");
                actions![]
            }
        }
    }

    pub(super) fn handle_peer_delay_response_follow_up(
        &mut self,
        header: Header,
        message: PDelayRespFollowUpMessage,
    ) -> PortActionIterator {
        if self.port_identity != message.requesting_port_identity {
            return actions![];
        }

        match self.peer_delay_state {
            PeerDelayState::PostMeasurement {
                id,
                responder_identity,
            } if id == header.sequence_id && responder_identity != header.source_port_identity => {
                log::error!(
                    "Responses from multiple devices to peer delay request, disabling port!"
                );
                self.set_forced_port_state(PortState::Faulty);
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                responder_identity: Some(identity),
                ..
            } if id == header.sequence_id && identity != header.source_port_identity => {
                log::error!(
                    "Responses from multiple devices to peer delay request, disabling port!"
                );
                self.set_forced_port_state(PortState::Faulty);
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                response_send_time: Some(_),
                ..
            } if id == header.sequence_id => {
                log::warn!("Duplicate PDelayRespFollowUp message");
                actions![]
            }
            PeerDelayState::Measuring {
                id,
                ref mut response_send_time,
                ref mut responder_identity,
                ..
            } if id == header.sequence_id => {
                *response_send_time = Some(
                    Time::from(message.response_origin_timestamp)
                        + Duration::from(header.correction_field),
                );
                *responder_identity = Some(header.source_port_identity);
                self.handle_time_measurement()
            }
            _ => {
                log::warn!("Unexpected PDelayRespFollowUp message");
                actions![]
            }
        }
    }

    fn extract_measurement(&mut self) -> Option<Measurement> {
        let mut result = Measurement::default();

        if let PeerDelayState::Measuring {
            request_send_time: Some(request_send_time),
            request_recv_time: Some(request_recv_time),
            response_send_time: Some(response_send_time),
            response_recv_time: Some(response_recv_time),
            responder_identity: Some(responder_identity),
            id,
        } = self.peer_delay_state
        {
            result.event_time = response_recv_time;
            result.peer_delay = Some(
                ((response_recv_time - request_send_time)
                    - (response_send_time - request_recv_time))
                    / 2.0,
            );
            self.peer_delay_state = PeerDelayState::PostMeasurement {
                id,
                responder_identity,
            };

            log::info!("Measurement: {:?}", result);

            if matches!(self.port_state, PortState::Faulty) {
                log::info!("Recovered port");
                self.set_forced_port_state(PortState::Listening);
            }

            return Some(result);
        }

        match self.port_state {
            PortState::Slave(ref mut state) => {
                if let SyncState::Measuring {
                    send_time: Some(send_time),
                    recv_time: Some(recv_time),
                    ..
                } = state.sync_state
                {
                    let raw_sync_offset = recv_time - send_time - self.config.delay_asymmetry;
                    result.event_time = recv_time;
                    result.raw_sync_offset = Some(raw_sync_offset);

                    if let Some(mean_delay) = self.mean_delay {
                        result.offset = Some(raw_sync_offset - mean_delay);
                    }

                    state.last_raw_sync_offset = Some(raw_sync_offset);
                    state.sync_state = SyncState::Empty;
                } else if let DelayState::Measuring {
                    send_time: Some(send_time),
                    recv_time: Some(recv_time),
                    ..
                } = state.delay_state
                {
                    let raw_delay_offset = send_time - recv_time - self.config.delay_asymmetry;
                    result.event_time = send_time;
                    result.raw_delay_offset = Some(raw_delay_offset);

                    if let Some(raw_sync_offset) = state.last_raw_sync_offset {
                        result.delay = Some((raw_sync_offset - raw_delay_offset) / 2);
                    }

                    state.delay_state = DelayState::Empty;
                } else {
                    // No measurement
                    return None;
                }

                log::info!("Measurement: {:?}", result);

                Some(result)
            }
            _ => None,
        }
    }
}

impl<'a, A, C: Clock, F: Filter, R: Rng, S: PtpInstanceStateMutex>
    Port<'a, Running, A, R, C, F, S>
{
    pub(super) fn send_delay_request(&mut self) -> PortActionIterator {
        match self.config.delay_mechanism {
            DelayMechanism::E2E { interval } => self.send_e2e_delay_request(interval),
            DelayMechanism::P2P { interval } => self.send_p2p_delay_request(interval),
        }
    }

    fn send_p2p_delay_request(
        &mut self,
        log_min_pdelay_req_interval: Interval,
    ) -> PortActionIterator {
        let pdelay_id = self.pdelay_seq_ids.generate();

        let pdelay_req = self.instance_state.with_ref(|state| {
            Message::pdelay_req(&state.default_ds, self.port_identity, pdelay_id)
        });
        let message_length = match pdelay_req.serialize(&mut self.packet_buffer) {
            Ok(length) => length,
            Err(error) => {
                log::error!("Could not serialize pdelay request: {:?}", error);
                return actions![];
            }
        };

        self.peer_delay_state = PeerDelayState::Measuring {
            id: pdelay_id,
            responder_identity: None,
            request_send_time: None,
            request_recv_time: None,
            response_send_time: None,
            response_recv_time: None,
        };

        let random = self.rng.sample::<f64, _>(rand::distributions::Open01);
        let factor = random * 2.0f64;
        let duration = log_min_pdelay_req_interval
            .as_core_duration()
            .mul_f64(factor);

        actions![
            PortAction::ResetDelayRequestTimer { duration },
            PortAction::SendEvent {
                context: TimestampContext {
                    inner: TimestampContextInner::PDelayReq { id: pdelay_id },
                },
                data: &self.packet_buffer[..message_length],
                link_local: true,
            }
        ]
    }

    fn send_e2e_delay_request(
        &mut self,
        log_min_delay_req_interval: Interval,
    ) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                log::debug!("Starting new delay measurement");

                let delay_id = self.delay_seq_ids.generate();
                let delay_req = self.instance_state.with_ref(|state| {
                    Message::delay_req(&state.default_ds, self.port_identity, delay_id)
                });

                let message_length = match delay_req.serialize(&mut self.packet_buffer) {
                    Ok(length) => length,
                    Err(error) => {
                        log::error!("Could not serialize delay request: {:?}", error);
                        return actions![];
                    }
                };

                state.delay_state = DelayState::Measuring {
                    id: delay_id,
                    send_time: None,
                    recv_time: None,
                };

                let random = self.rng.sample::<f64, _>(rand::distributions::Open01);
                let factor = random * 2.0f64;
                let duration = log_min_delay_req_interval
                    .as_core_duration()
                    .mul_f64(factor);

                actions![
                    PortAction::ResetDelayRequestTimer { duration },
                    PortAction::SendEvent {
                        context: TimestampContext {
                            inner: TimestampContextInner::DelayReq { id: delay_id },
                        },
                        data: &self.packet_buffer[..message_length],
                        link_local: false,
                    }
                ]
            }
            _ => actions![],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::ToOwned;

    use super::*;
    use crate::{
        datastructures::{
            common::{PortIdentity, TimeInterval},
            messages::MessageBody,
        },
        filters::FilterUpdate,
        port::{
            state::SlaveState,
            tests::{setup_test_port_custom_filter, setup_test_state},
            Measurement,
        },
    };

    struct TestFilter {
        last_measurement: Option<Measurement>,
    }

    impl Filter for TestFilter {
        type Config = ();

        fn new(_config: Self::Config) -> Self {
            Self {
                last_measurement: None,
            }
        }

        fn measurement<C: Clock>(&mut self, m: Measurement, _clock: &mut C) -> FilterUpdate {
            self.last_measurement = Some(m);
            if let Some(delay) = m.delay {
                FilterUpdate {
                    next_update: None,
                    mean_delay: Some(delay),
                }
            } else {
                Default::default()
            }
        }

        fn demobilize<C: Clock>(self, _clock: &mut C) {
            Default::default()
        }

        fn update<C: Clock>(&mut self, _clock: &mut C) -> FilterUpdate {
            Default::default()
        }
    }

    #[test]
    fn test_sync_without_delay_msg() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::new(Default::default());
        port.mean_delay = Some(Duration::from_micros(100));

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_sync(
            Header {
                two_step_flag: false,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(49),
                offset: Some(Duration::from_micros(-51)),
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(49)),
                raw_delay_offset: None,
            })
        );

        let mut action = port.handle_sync(
            Header {
                two_step_flag: true,
                sequence_id: 15,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(1050),
        );
        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_follow_up(
            Header {
                sequence_id: 15,
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            FollowUpMessage {
                precise_origin_timestamp: Time::from_micros(1000).into(),
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(1049),
                offset: Some(Duration::from_micros(-53)),
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(47)),
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_delay_asymmetry() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        port.config.delay_asymmetry = Duration::from_micros(100);

        let state = SlaveState::new(Default::default());
        port.mean_delay = Some(Duration::from_micros(100));

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_sync(
            Header {
                two_step_flag: false,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(49),
                offset: Some(Duration::from_micros(-151)),
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(-51)),
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_sync_with_delay() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::new(Default::default());

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_sync(
            Header {
                two_step_flag: false,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(49),
                offset: None,
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(49)),
                raw_delay_offset: None,
            })
        );

        let mut action = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = action.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: false,
        }) = action.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let req = Message::deserialize(&data).unwrap();
        let req_header = req.header;

        let _req = match req.body {
            MessageBody::DelayReq(msg) => msg,
            _ => panic!("Incorrect message type"),
        };

        let timestamp_id = match context.inner {
            TimestampContextInner::DelayReq { id } => id,
            _ => panic!("Incorrect timestamp context"),
        };

        let mut action = port.handle_delay_timestamp(timestamp_id, Time::from_micros(100));
        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_delay_resp(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req_header.sequence_id,
                ..Default::default()
            },
            DelayRespMessage {
                receive_timestamp: Time::from_micros(253).into(),
                requesting_port_identity: req_header.source_port_identity,
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.mean_delay, Some(Duration::from_micros(100)));
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(100),
                offset: None,
                delay: Some(Duration::from_micros(100)),
                peer_delay: None,
                raw_sync_offset: None,
                raw_delay_offset: Some(Duration::from_micros(-151)),
            })
        );

        port.mean_delay = None;

        let mut action = port.handle_sync(
            Header {
                two_step_flag: true,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(1050),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = action.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: false,
        }) = action.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let req = Message::deserialize(&data).unwrap();
        let req_header = req.header;

        let _req = match req.body {
            MessageBody::DelayReq(msg) => msg,
            _ => panic!("Incorrect message type"),
        };

        let timestamp_id = match context.inner {
            TimestampContextInner::DelayReq { id } => id,
            _ => panic!("Incorrect timestamp context"),
        };

        let mut action = port.handle_delay_timestamp(timestamp_id, Time::from_micros(1100));
        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_follow_up(
            Header {
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            FollowUpMessage {
                precise_origin_timestamp: Time::from_micros(1000).into(),
            },
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(1049),
                offset: None,
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(47)),
                raw_delay_offset: None,
            })
        );

        let mut action = port.handle_delay_resp(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req_header.sequence_id,
                ..Default::default()
            },
            DelayRespMessage {
                receive_timestamp: Time::from_micros(1255).into(),
                requesting_port_identity: req_header.source_port_identity,
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.mean_delay, Some(Duration::from_micros(100)));
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(1100),
                offset: None,
                delay: Some(Duration::from_micros(100)),
                peer_delay: None,
                raw_sync_offset: None,
                raw_delay_offset: Some(Duration::from_micros(-153)),
            })
        );
    }

    #[test]
    fn test_follow_up_before_sync() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::new(Default::default());
        port.mean_delay = Some(Duration::from_micros(100));

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_follow_up(
            Header {
                sequence_id: 15,
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            FollowUpMessage {
                precise_origin_timestamp: Time::from_micros(10).into(),
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_sync(
            Header {
                two_step_flag: true,
                sequence_id: 15,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(49),
                offset: Some(Duration::from_micros(-63)),
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(37)),
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_old_followup_during() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::new(Default::default());
        port.mean_delay = Some(Duration::from_micros(100));

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_sync(
            Header {
                two_step_flag: true,
                sequence_id: 15,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_follow_up(
            Header {
                sequence_id: 14,
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            FollowUpMessage {
                precise_origin_timestamp: Time::from_micros(10).into(),
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_follow_up(
            Header {
                sequence_id: 15,
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            FollowUpMessage {
                precise_origin_timestamp: Time::from_micros(10).into(),
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.filter.last_measurement.take(), None);
    }

    #[test]
    fn test_reset_after_missing_followup() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::new(Default::default());
        port.mean_delay = Some(Duration::from_micros(100));

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_sync(
            Header {
                two_step_flag: true,
                sequence_id: 14,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_sync(
            Header {
                two_step_flag: true,
                sequence_id: 15,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(1050),
        );

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_follow_up(
            Header {
                sequence_id: 15,
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            FollowUpMessage {
                precise_origin_timestamp: Time::from_micros(1000).into(),
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(1049),
                offset: Some(Duration::from_micros(-53)),
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(47)),
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_ignore_unrelated_delayresp() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::new(Default::default());

        port.set_forced_port_state(PortState::Slave(state));

        let mut action = port.handle_sync(
            Header {
                two_step_flag: false,
                correction_field: TimeInterval(1000.into()),
                ..Default::default()
            },
            SyncMessage {
                origin_timestamp: Time::from_micros(0).into(),
            },
            Time::from_micros(50),
        );

        // DelayReq is sent independently
        assert!(action.next().is_none());
        drop(action);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(49),
                offset: None,
                delay: None,
                peer_delay: None,
                raw_sync_offset: Some(Duration::from_micros(49)),
                raw_delay_offset: None,
            })
        );

        let mut action = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = action.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: false,
        }) = action.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();

        let timestamp_id = match context.inner {
            TimestampContextInner::DelayReq { id } => id,
            _ => panic!("Incorrect timestamp context"),
        };

        drop(action);

        let mut action = port.handle_delay_timestamp(timestamp_id, Time::from_micros(100));

        assert!(action.next().is_none());
        drop(action);
        assert_eq!(port.filter.last_measurement.take(), None);

        let req = Message::deserialize(&data).unwrap();
        let req_header = req.header;

        let _req = match req.body {
            MessageBody::DelayReq(msg) => msg,
            _ => panic!("Incorrect message type"),
        };

        let mut action = port.handle_delay_resp(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req_header.sequence_id,
                ..Default::default()
            },
            DelayRespMessage {
                receive_timestamp: Time::from_micros(353).into(),
                requesting_port_identity: PortIdentity {
                    port_number: 83,
                    ..Default::default()
                },
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_delay_resp(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req_header.sequence_id.wrapping_sub(1),
                ..Default::default()
            },
            DelayRespMessage {
                receive_timestamp: Time::from_micros(353).into(),
                requesting_port_identity: req_header.source_port_identity,
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.filter.last_measurement.take(), None);

        let mut action = port.handle_delay_resp(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req_header.sequence_id,
                ..Default::default()
            },
            DelayRespMessage {
                receive_timestamp: Time::from_micros(253).into(),
                requesting_port_identity: req_header.source_port_identity,
            },
        );

        assert!(action.next().is_none());
        drop(action);

        assert_eq!(port.mean_delay, Some(Duration::from_micros(100)));
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(100),
                offset: None,
                delay: Some(Duration::from_micros(100)),
                peer_delay: None,
                raw_sync_offset: None,
                raw_delay_offset: Some(Duration::from_micros(-151)),
            })
        );
    }

    #[test]
    fn test_peer_delay_1step() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());
        port.config.delay_mechanism = DelayMechanism::P2P {
            interval: Interval::from_log_2(1),
        };

        let state = SlaveState::new(Default::default());

        port.set_forced_port_state(PortState::Slave(state));

        let mut actions = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_send_timestamp(context, Time::from_micros(50));
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let req = Message::deserialize(&data).unwrap();
        assert!(matches!(req.body, MessageBody::PDelayReq(_)));

        let mut actions = port.handle_peer_delay_response(
            Header {
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            PDelayRespMessage {
                request_receive_timestamp: Time::from_micros(100).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
            Time::from_micros(152),
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(150),
                offset: None,
                delay: None,
                peer_delay: Some(Duration::from_micros(50)),
                raw_sync_offset: None,
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_peer_delay_2step() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());
        port.config.delay_mechanism = DelayMechanism::P2P {
            interval: Interval::from_log_2(1),
        };

        let state = SlaveState::new(Default::default());

        port.set_forced_port_state(PortState::Slave(state));

        let mut actions = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_send_timestamp(context, Time::from_micros(50));
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let req = Message::deserialize(&data).unwrap();
        assert!(matches!(req.body, MessageBody::PDelayReq(_)));

        let mut actions = port.handle_peer_delay_response(
            Header {
                two_step_flag: true,
                correction_field: TimeInterval(1000.into()),
                sequence_id: req.header.sequence_id,
                ..Default::default()
            },
            PDelayRespMessage {
                request_receive_timestamp: Time::from_micros(101).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
            Time::from_micros(154),
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_peer_delay_response_follow_up(
            Header {
                correction_field: TimeInterval(1000.into()),
                sequence_id: req.header.sequence_id,
                ..Default::default()
            },
            PDelayRespFollowUpMessage {
                response_origin_timestamp: Time::from_micros(103).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(153),
                offset: None,
                delay: None,
                peer_delay: Some(Duration::from_micros(50)),
                raw_sync_offset: None,
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_peer_delay_2step_followup_before_response() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());
        port.config.delay_mechanism = DelayMechanism::P2P {
            interval: Interval::from_log_2(1),
        };

        let state = SlaveState::new(Default::default());

        port.set_forced_port_state(PortState::Slave(state));

        let mut actions = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_send_timestamp(context, Time::from_micros(50));
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let req = Message::deserialize(&data).unwrap();
        assert!(matches!(req.body, MessageBody::PDelayReq(_)));

        let mut actions = port.handle_peer_delay_response_follow_up(
            Header {
                correction_field: TimeInterval(1000.into()),
                sequence_id: req.header.sequence_id,
                ..Default::default()
            },
            PDelayRespFollowUpMessage {
                response_origin_timestamp: Time::from_micros(103).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_peer_delay_response(
            Header {
                two_step_flag: true,
                correction_field: TimeInterval(1000.into()),
                sequence_id: req.header.sequence_id,
                ..Default::default()
            },
            PDelayRespMessage {
                request_receive_timestamp: Time::from_micros(101).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
            Time::from_micros(154),
        );
        assert!(actions.next().is_none());
        drop(actions);

        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(153),
                offset: None,
                delay: None,
                peer_delay: Some(Duration::from_micros(50)),
                raw_sync_offset: None,
                raw_delay_offset: None,
            })
        );
    }

    #[test]
    fn test_peer_delay_faulty() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());
        port.config.delay_mechanism = DelayMechanism::P2P {
            interval: Interval::from_log_2(1),
        };

        let state = SlaveState::new(Default::default());

        port.set_forced_port_state(PortState::Slave(state));

        let mut actions = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_send_timestamp(context, Time::from_micros(50));
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let req = Message::deserialize(&data).unwrap();
        assert!(matches!(req.body, MessageBody::PDelayReq(_)));

        let mut actions = port.handle_peer_delay_response(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req.header.sequence_id,
                ..Default::default()
            },
            PDelayRespMessage {
                request_receive_timestamp: Time::from_micros(100).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
            Time::from_micros(152),
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(150),
                offset: None,
                delay: None,
                peer_delay: Some(Duration::from_micros(50)),
                raw_sync_offset: None,
                raw_delay_offset: None,
            })
        );
        assert!(!matches!(port.port_state, PortState::Faulty));

        let mut actions = port.handle_peer_delay_response(
            Header {
                source_port_identity: PortIdentity {
                    clock_identity: Default::default(),
                    port_number: 5,
                },
                correction_field: TimeInterval(2000.into()),
                ..Default::default()
            },
            PDelayRespMessage {
                request_receive_timestamp: Time::from_micros(100).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
            Time::from_micros(152),
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());
        assert!(matches!(port.port_state, PortState::Faulty));

        let mut actions = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent {
            context,
            data,
            link_local: true,
        }) = actions.next()
        else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let mut actions = port.handle_send_timestamp(context, Time::from_micros(50));
        assert!(actions.next().is_none());
        drop(actions);
        assert!(port.filter.last_measurement.take().is_none());

        let req = Message::deserialize(&data).unwrap();
        assert!(matches!(req.body, MessageBody::PDelayReq(_)));

        let mut actions = port.handle_peer_delay_response(
            Header {
                correction_field: TimeInterval(2000.into()),
                sequence_id: req.header.sequence_id,
                ..Default::default()
            },
            PDelayRespMessage {
                request_receive_timestamp: Time::from_micros(100).into(),
                requesting_port_identity: req.header.source_port_identity,
            },
            Time::from_micros(152),
        );
        assert!(actions.next().is_none());
        drop(actions);
        assert_eq!(
            port.filter.last_measurement.take(),
            Some(Measurement {
                event_time: Time::from_micros(150),
                offset: None,
                delay: None,
                peer_delay: Some(Duration::from_micros(50)),
                raw_sync_offset: None,
                raw_delay_offset: None,
            })
        );
        assert!(!matches!(port.port_state, PortState::Faulty));
    }
}
