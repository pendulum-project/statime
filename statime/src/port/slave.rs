use rand::Rng;

use super::{
    state::{DelayState, PortState, SlaveState},
    Measurement, Port, PortActionIterator, Running,
};
use crate::{
    config::DelayMechanism,
    datastructures::messages::{DelayRespMessage, FollowUpMessage, Header, Message, SyncMessage},
    filters::Filter,
    port::{actions::TimestampContextInner, state::SyncState, PortAction, TimestampContext},
    time::{Duration, Time},
    Clock,
};

impl<'a, A, C: Clock, F: Filter, R> Port<Running<'a>, A, R, C, F> {
    pub(super) fn handle_time_measurement<'b>(&mut self) -> PortActionIterator<'b> {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                if let Some(measurement) = state.extract_measurement(self.config.delay_asymmetry) {
                    // If the received message allowed the (slave) state to calculate its offset
                    // from the master, update the local clock
                    let filter_updates = state.filter.measurement(measurement, &mut self.clock);
                    if let Some(mean_delay) = filter_updates.mean_delay {
                        state.mean_delay = Some(mean_delay);
                    }
                    PortActionIterator::from_filter(filter_updates)
                } else {
                    actions![]
                }
            }
            _ => actions![],
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

    pub(super) fn handle_sync(
        &mut self,
        header: Header,
        message: SyncMessage,
        recv_time: Time,
    ) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                log::debug!("Received sync {:?}", header.sequence_id);

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
                if self.port_identity != message.requesting_port_identity {
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
}

impl<'a, A, C: Clock, F: Filter, R: Rng> Port<Running<'a>, A, R, C, F> {
    pub(super) fn send_delay_request(&mut self) -> PortActionIterator {
        match self.port_state {
            PortState::Slave(ref mut state) => {
                log::debug!("Starting new delay measurement");

                let delay_id = self.delay_seq_ids.generate();
                let delay_req = Message::delay_req(
                    &self.lifecycle.state.default_ds,
                    self.port_identity,
                    delay_id,
                );

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
                let log_min_delay_req_interval = match self.config.delay_mechanism {
                    // the interval corresponds to the PortDS logMinDelayReqInterval
                    DelayMechanism::E2E { interval } => interval,
                };
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
                    }
                ]
            }
            _ => actions![],
        }
    }
}

impl<F> SlaveState<F> {
    fn extract_measurement(&mut self, delay_asymmetry: Duration) -> Option<Measurement> {
        let mut result = Measurement::default();

        if let SyncState::Measuring {
            send_time: Some(send_time),
            recv_time: Some(recv_time),
            ..
        } = self.sync_state
        {
            let raw_sync_offset = recv_time - send_time - delay_asymmetry;
            result.event_time = recv_time;
            result.raw_sync_offset = Some(raw_sync_offset);

            if let Some(mean_delay) = self.mean_delay {
                result.offset = Some(raw_sync_offset - mean_delay);
            }

            self.last_raw_sync_offset = Some(raw_sync_offset);
            self.sync_state = SyncState::Empty;
        } else if let DelayState::Measuring {
            send_time: Some(send_time),
            recv_time: Some(recv_time),
            ..
        } = self.delay_state
        {
            let raw_delay_offset = send_time - recv_time - delay_asymmetry;
            result.event_time = send_time;
            result.raw_delay_offset = Some(raw_delay_offset);

            if let Some(raw_sync_offset) = self.last_raw_sync_offset {
                result.delay = Some((raw_sync_offset - raw_delay_offset) / 2);
            }

            self.delay_state = DelayState::Empty;
        } else {
            // No measurement
            return None;
        }

        log::info!("Measurement: {:?}", result);

        Some(result)
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

        let mut state = SlaveState::<TestFilter>::new(Default::default(), ());
        state.mean_delay = Some(Duration::from_micros(100));

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
                    Some(Measurement {
                        event_time: Time::from_micros(49),
                        offset: Some(Duration::from_micros(-51)),
                        delay: None,
                        peer_delay: None,
                        raw_sync_offset: Some(Duration::from_micros(49)),
                        raw_delay_offset: None,
                    })
                );
            }
            _ => panic!("Unexpected port state"),
        }

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
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
            _ => panic!("Unexpected port state"),
        }
    }

    #[test]
    fn test_delay_asymmetry() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        port.config.delay_asymmetry = Duration::from_micros(100);

        let mut state = SlaveState::<TestFilter>::new(Default::default(), ());
        state.mean_delay = Some(Duration::from_micros(100));

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
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
            _ => panic!("Unexpected port state"),
        }
    }

    #[test]
    fn test_sync_with_delay() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::<TestFilter>::new(Default::default(), ());

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
                    Some(Measurement {
                        event_time: Time::from_micros(49),
                        offset: None,
                        delay: None,
                        peer_delay: None,
                        raw_sync_offset: Some(Duration::from_micros(49)),
                        raw_delay_offset: None,
                    })
                );
            }
            _ => panic!("Unexpected port state"),
        }

        let mut action = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = action.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent { context, data }) = action.next() else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        assert!(action.next().is_none());
        drop(action);
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

        let req = Message::deserialize(&data).unwrap();
        let req_header = req.header;

        let _req = match req.body {
            MessageBody::DelayReq(msg) => msg,
            _ => panic!("Incorrect message type"),
        };

        let timestamp_id = match context.inner {
            TimestampContextInner::DelayReq { id } => id,
            TimestampContextInner::Sync { .. } => panic!("Incorrect timestamp context"),
        };

        let mut action = port.handle_delay_timestamp(timestamp_id, Time::from_micros(100));
        assert!(action.next().is_none());
        drop(action);
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.mean_delay, Some(Duration::from_micros(100)));
                assert_eq!(
                    state.filter.last_measurement.take(),
                    Some(Measurement {
                        event_time: Time::from_micros(100),
                        offset: None,
                        delay: Some(Duration::from_micros(100)),
                        peer_delay: None,
                        raw_sync_offset: None,
                        raw_delay_offset: Some(Duration::from_micros(-151)),
                    })
                );

                state.mean_delay = None;
            }
            _ => panic!("Unexpected port state"),
        }

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

        let mut action = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = action.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent { context, data }) = action.next() else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();
        assert!(action.next().is_none());
        drop(action);
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

        let req = Message::deserialize(&data).unwrap();
        let req_header = req.header;

        let _req = match req.body {
            MessageBody::DelayReq(msg) => msg,
            _ => panic!("Incorrect message type"),
        };

        let timestamp_id = match context.inner {
            TimestampContextInner::DelayReq { id } => id,
            TimestampContextInner::Sync { .. } => panic!("Incorrect timestamp context"),
        };

        let mut action = port.handle_delay_timestamp(timestamp_id, Time::from_micros(1100));
        assert!(action.next().is_none());
        drop(action);
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
                    Some(Measurement {
                        event_time: Time::from_micros(1049),
                        offset: None,
                        delay: None,
                        peer_delay: None,
                        raw_sync_offset: Some(Duration::from_micros(47)),
                        raw_delay_offset: None,
                    })
                );
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.mean_delay, Some(Duration::from_micros(100)));
                assert_eq!(
                    state.filter.last_measurement.take(),
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
            _ => panic!("Unexpected port state"),
        }
    }

    #[test]
    fn test_follow_up_before_sync() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let mut state = SlaveState::<TestFilter>::new(Default::default(), ());
        state.mean_delay = Some(Duration::from_micros(100));

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
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
            _ => panic!("Unexpected port state"),
        }
    }

    #[test]
    fn test_old_followup_during() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let mut state = SlaveState::<TestFilter>::new(Default::default(), ());
        state.mean_delay = Some(Duration::from_micros(100));

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }
    }

    #[test]
    fn test_reset_after_missing_followup() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let mut state = SlaveState::<TestFilter>::new(Default::default(), ());
        state.mean_delay = Some(Duration::from_micros(100));

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
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
            _ => panic!("Unexpected port state"),
        }
    }

    #[test]
    fn test_ignore_unrelated_delayresp() {
        let state = setup_test_state();

        let mut port = setup_test_port_custom_filter::<TestFilter>(&state, ());

        let state = SlaveState::<TestFilter>::new(Default::default(), ());

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
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(
                    state.filter.last_measurement.take(),
                    Some(Measurement {
                        event_time: Time::from_micros(49),
                        offset: None,
                        delay: None,
                        peer_delay: None,
                        raw_sync_offset: Some(Duration::from_micros(49)),
                        raw_delay_offset: None,
                    })
                );
            }
            _ => panic!("Unexpected port state"),
        }

        let mut action = port.send_delay_request();

        let Some(PortAction::ResetDelayRequestTimer { .. }) = action.next() else {
            panic!("Unexpected action");
        };

        let Some(PortAction::SendEvent { context, data }) = action.next() else {
            panic!("Unexpected action");
        };
        let data = data.to_owned();

        let timestamp_id = match context.inner {
            TimestampContextInner::DelayReq { id } => id,
            TimestampContextInner::Sync { .. } => panic!("Incorrect timestamp context"),
        };

        drop(action);

        let mut action = port.handle_delay_timestamp(timestamp_id, Time::from_micros(100));

        assert!(action.next().is_none());
        drop(action);
        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.filter.last_measurement.take(), None);
            }
            _ => panic!("Unexpected port state"),
        }

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

        match port.port_state {
            PortState::Slave(ref mut state) => {
                assert_eq!(state.mean_delay, Some(Duration::from_micros(100)));
                assert_eq!(
                    state.filter.last_measurement.take(),
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
            _ => panic!("Unexpected port state"),
        }
    }
}
