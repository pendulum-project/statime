use rand::Rng;

use super::{InBmca, Port, PortActionIterator, Running};
use crate::{
    bmc::bmca::{BestAnnounceMessage, RecommendedState},
    config::{AcceptableMasterList, LeapIndicator, TimePropertiesDS, TimeSource},
    datastructures::{
        common::{ClockIdentity, TlvType},
        datasets::{InternalCurrentDS, InternalDefaultDS, InternalParentDS, PathTraceDS},
        messages::Message,
    },
    filters::Filter,
    port::{
        state::{PortState, SlaveState},
        PortAction,
    },
    ptp_instance::PtpInstanceStateMutex,
    time::Duration,
    Clock,
};

impl<'a, A: AcceptableMasterList, C: Clock, F: Filter, R: Rng, S: PtpInstanceStateMutex>
    Port<'a, Running, A, R, C, F, S>
{
    pub(super) fn handle_announce<'b>(
        &'b mut self,
        message: &Message<'b>,
        announce: crate::datastructures::messages::AnnounceMessage,
    ) -> PortActionIterator<'b> {
        // IEEE 1588-2019 9.5.3: Update according to table 33 (decision code S1)
        if matches!(self.port_state, PortState::Slave(_))
            && announce.header.source_port_identity
                == self
                    .instance_state
                    .with_ref(|s| s.parent_ds.parent_port_identity)
        {
            let clock_loop_detected = self.instance_state.with_mut(|state| {
                let current_ds = &mut state.current_ds;
                let parent_ds = &mut state.parent_ds;
                let time_properties_ds = &mut state.time_properties_ds;
                let path_trace_ds = &mut state.path_trace_ds;

                current_ds.steps_removed = announce.steps_removed + 1;

                parent_ds.parent_port_identity = announce.header.source_port_identity;
                parent_ds.grandmaster_identity = announce.grandmaster_identity;
                parent_ds.grandmaster_clock_quality = announce.grandmaster_clock_quality;
                parent_ds.grandmaster_priority_1 = announce.grandmaster_priority_1;
                parent_ds.grandmaster_priority_2 = announce.grandmaster_priority_2;

                *time_properties_ds = announce.time_properties();

                if path_trace_ds.enable {
                    if let Some(tlv) = message
                        .suffix
                        .tlv()
                        .find(|tlv| tlv.tlv_type == TlvType::PathTrace)
                    {
                        let clock_identity = state.default_ds.clock_identity;
                        if tlv.value.chunks_exact(8).any(|ci| ci == clock_identity.0) {
                            log::warn!("Clock loop detected");
                            return true;
                        }

                        // Cannot panic as `list` is large enough to contain up to a whole message
                        path_trace_ds.list = tlv
                            .value
                            .chunks_exact(8)
                            .map(|ci| ClockIdentity(<[u8; 8]>::try_from(ci).unwrap()))
                            .collect();
                    }
                }

                false
            });

            if clock_loop_detected {
                return actions![];
            }
        }

        if self
            .bmca
            .register_announce_message(&message.header, &announce)
        {
            // Doing the multiport-same network check after registering the announce message
            // ensures that the message is acceptable wrt the acceptable master list.
            // This ensures that an administrator can block this mechanism via the
            // acceptable master list, making this less of an attack vector.
            if self.port_identity.clock_identity
                == message.header.source_port_identity.clock_identity
                && self.port_identity.port_number > message.header.source_port_identity.port_number
            {
                self.multiport_disable = Some(Duration::ZERO);
                self.set_forced_port_state(PortState::Passive);
            }
            actions![PortAction::ResetAnnounceReceiptTimer {
                duration: self.config.announce_duration(&mut self.rng),
            }]
            .with_forward_tlvs(message.suffix.tlv(), message.header.source_port_identity)
        } else {
            actions![]
        }
    }
}

// BMCA related functionality of the port
impl<'a, A: AcceptableMasterList, C: Clock, F: Filter, R: Rng, S: PtpInstanceStateMutex>
    Port<'a, InBmca, A, R, C, F, S>
{
    pub(crate) fn calculate_best_local_announce_message(&mut self) {
        self.lifecycle.local_best = self.bmca.take_best_port_announce_message()
    }
}

impl<'a, A, C: Clock, F: Filter, R: Rng, S: PtpInstanceStateMutex> Port<'a, InBmca, A, R, C, F, S> {
    pub(crate) fn step_announce_age(&mut self, step: Duration) {
        if let Some(mut age) = self.multiport_disable.take() {
            age += step;
            if age < self.config.announce_interval.as_duration() {
                self.multiport_disable = Some(age)
            }
        }

        self.bmca.step_age(step);
    }

    pub(crate) fn best_local_announce_message_for_bmca(&self) -> Option<BestAnnounceMessage> {
        // Announce messages received on a masterOnly PTP Port shall not be considered
        // in the global operation of the best master clock algorithm or in the update
        // of data sets. We still need them during the calculation of the recommended
        // port state though to avoid getting multiple masters in the segment.
        if self.config.master_only || matches!(self.port_state, PortState::Faulty) {
            None
        } else {
            self.lifecycle.local_best
        }
    }

    pub(crate) fn best_local_announce_message_for_state(&self) -> Option<BestAnnounceMessage> {
        // Announce messages received on a masterOnly PTP Port shall not be considered
        // in the global operation of the best master clock algorithm or in the update
        // of data sets. We still need them during the calculation of the recommended
        // port state though to avoid getting multiple masters in the segment.
        self.lifecycle.local_best
    }

    pub(crate) fn set_recommended_state(
        &mut self,
        recommended_state: RecommendedState,
        path_trace_ds: &mut PathTraceDS,
        time_properties_ds: &mut TimePropertiesDS,
        current_ds: &mut InternalCurrentDS,
        parent_ds: &mut InternalParentDS,
        default_ds: &InternalDefaultDS,
    ) {
        self.set_recommended_port_state(&recommended_state, default_ds);

        match recommended_state {
            RecommendedState::M1(defaultds) | RecommendedState::M2(defaultds) => {
                // a slave-only PTP port should never end up in the master state
                debug_assert!(!default_ds.slave_only);

                current_ds.steps_removed = 0;
                current_ds.offset_from_master = Duration::ZERO;

                parent_ds.parent_port_identity.clock_identity = defaultds.clock_identity;
                parent_ds.parent_port_identity.port_number = 0;
                parent_ds.grandmaster_identity = defaultds.clock_identity;
                parent_ds.grandmaster_clock_quality = defaultds.clock_quality;
                parent_ds.grandmaster_priority_1 = defaultds.priority_1;
                parent_ds.grandmaster_priority_2 = defaultds.priority_2;

                time_properties_ds.leap_indicator = LeapIndicator::NoLeap;
                time_properties_ds.current_utc_offset = None;
                time_properties_ds.ptp_timescale = true;
                time_properties_ds.time_traceable = false;
                time_properties_ds.frequency_traceable = false;
                time_properties_ds.time_source = TimeSource::InternalOscillator;

                path_trace_ds.list.clear();
            }
            RecommendedState::M3(_) | RecommendedState::P1(_) | RecommendedState::P2(_) => {}
            RecommendedState::S1(announce_message) => {
                // a master-only PTP port should never end up in the slave state
                debug_assert!(!self.config.master_only);

                current_ds.steps_removed = announce_message.steps_removed + 1;

                parent_ds.parent_port_identity = announce_message.header.source_port_identity;
                parent_ds.grandmaster_identity = announce_message.grandmaster_identity;
                parent_ds.grandmaster_clock_quality = announce_message.grandmaster_clock_quality;
                parent_ds.grandmaster_priority_1 = announce_message.grandmaster_priority_1;
                parent_ds.grandmaster_priority_2 = announce_message.grandmaster_priority_2;

                *time_properties_ds = announce_message.time_properties();

                if let Err(error) = self.clock.set_properties(time_properties_ds) {
                    log::error!("Could not update clock: {:?}", error);
                }
            }
        }

        // TODO: Discuss if we should change the clock's own time properties, or keep
        // the master's time properties separately
        if let RecommendedState::S1(announce_message) = &recommended_state {
            // Update time properties
            *time_properties_ds = announce_message.time_properties();
        }
    }

    fn set_recommended_port_state(
        &mut self,
        recommended_state: &RecommendedState,
        default_ds: &InternalDefaultDS,
    ) {
        match recommended_state {
            // TODO set things like steps_removed once they are added
            // TODO make sure states are complete
            RecommendedState::S1(announce_message) => {
                // a master-only PTP port should never end up in the slave state
                debug_assert!(!self.config.master_only);

                let remote_master = announce_message.header.source_port_identity;

                let update_state = match &self.port_state {
                    PortState::Faulty => false,
                    PortState::Listening | PortState::Master | PortState::Passive => true,
                    PortState::Slave(old_state) => old_state.remote_master() != remote_master,
                };

                if update_state {
                    let state = PortState::Slave(SlaveState::new(remote_master));
                    self.set_forced_port_state(state);

                    let duration = self.config.announce_duration(&mut self.rng);
                    let reset_announce = PortAction::ResetAnnounceReceiptTimer { duration };
                    let reset_delay = PortAction::ResetDelayRequestTimer {
                        duration: core::time::Duration::ZERO,
                    };
                    self.lifecycle.pending_action = actions![reset_announce, reset_delay];
                }
            }
            RecommendedState::M1(_) | RecommendedState::M2(_) | RecommendedState::M3(_) => {
                if default_ds.slave_only {
                    match self.port_state {
                        PortState::Listening | PortState::Faulty => { /* do nothing */ }
                        PortState::Slave(_) | PortState::Passive => {
                            self.set_forced_port_state(PortState::Listening);

                            // consistent with Port<InBmca>::new()
                            let duration = self.config.announce_duration(&mut self.rng);
                            let reset_announce = PortAction::ResetAnnounceReceiptTimer { duration };
                            self.lifecycle.pending_action = actions![reset_announce];
                        }
                        PortState::Master => {
                            let msg = "slave-only PTP port should not be in master state";
                            debug_assert!(!default_ds.slave_only, "{msg}");
                            log::error!("{msg}");
                        }
                    }
                } else if self.multiport_disable.is_some() {
                    if !matches!(self.port_state, PortState::Passive) {
                        self.set_forced_port_state(PortState::Passive);
                    }
                } else {
                    match self.port_state {
                        PortState::Listening | PortState::Slave(_) | PortState::Passive => {
                            self.set_forced_port_state(PortState::Master);

                            // Immediately start sending announces and syncs
                            let duration = core::time::Duration::from_secs(0);
                            self.lifecycle.pending_action = actions![
                                PortAction::ResetAnnounceTimer { duration },
                                PortAction::ResetSyncTimer { duration }
                            ];
                        }
                        PortState::Master | PortState::Faulty => { /* do nothing */ }
                    }
                }
            }
            RecommendedState::P1(_) | RecommendedState::P2(_) => match self.port_state {
                PortState::Listening | PortState::Slave(_) | PortState::Master => {
                    self.set_forced_port_state(PortState::Passive)
                }
                PortState::Passive | PortState::Faulty => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{ClockIdentity, InstanceConfig, SdoId},
        datastructures::{
            common::{PortIdentity, Tlv, TlvSetBuilder},
            messages::{AnnounceMessage, Header, Message, MessageBody, PtpVersion, MAX_DATA_LEN},
        },
        port::tests::{setup_test_port, setup_test_port_custom_identity, setup_test_state},
        time::Time,
    };

    fn default_announce_message_header() -> Header {
        Header {
            sdo_id: Default::default(),
            version: PtpVersion::new(2, 1).unwrap(),
            domain_number: Default::default(),
            alternate_master_flag: false,
            two_step_flag: false,
            unicast_flag: false,
            ptp_profile_specific_1: false,
            ptp_profile_specific_2: false,
            leap61: false,
            leap59: false,
            current_utc_offset_valid: false,
            ptp_timescale: false,
            time_tracable: false,
            frequency_tracable: false,
            synchronization_uncertain: false,
            correction_field: Default::default(),
            source_port_identity: Default::default(),
            sequence_id: Default::default(),
            log_message_interval: Default::default(),
        }
    }

    fn default_announce_message() -> AnnounceMessage {
        AnnounceMessage {
            header: default_announce_message_header(),
            origin_timestamp: Default::default(),
            current_utc_offset: Default::default(),
            grandmaster_priority_1: Default::default(),
            grandmaster_clock_quality: Default::default(),
            grandmaster_priority_2: Default::default(),
            grandmaster_identity: Default::default(),
            steps_removed: Default::default(),
            time_source: Default::default(),
        }
    }

    #[test]
    fn test_multiport_disable() {
        let state = setup_test_state();
        let mut port = setup_test_port_custom_identity(
            &state,
            PortIdentity {
                clock_identity: Default::default(),
                port_number: 1,
            },
        );

        port.set_forced_port_state(PortState::Master);

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity = port.port_identity.clock_identity;
        announce.header.source_port_identity.port_number = 2;
        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: Default::default(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];
        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        assert!(port.multiport_disable.is_none());
        assert!(matches!(port.port_state, PortState::Master));

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity = port.port_identity.clock_identity;
        announce.header.source_port_identity.port_number = 0;
        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: Default::default(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];
        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        assert!(port.multiport_disable.is_some());
        assert!(matches!(port.port_state, PortState::Passive));

        let instanceconfig = InstanceConfig {
            clock_identity: ClockIdentity::from_mac_address([1, 2, 3, 4, 5, 6]),
            priority_1: 128,
            priority_2: 128,
            domain_number: 0,
            sdo_id: SdoId::default(),
            slave_only: false,
            path_trace: false,
        };
        let mut port = port.start_bmca();
        port.set_recommended_port_state(
            &RecommendedState::M1(InternalDefaultDS::new(instanceconfig)),
            &InternalDefaultDS::new(instanceconfig),
        );

        assert!(port.multiport_disable.is_some());
        assert!(matches!(port.port_state, PortState::Passive));

        port.step_announce_age(port.config.announce_interval.as_duration());
        port.step_announce_age(port.config.announce_interval.as_duration());

        assert!(port.multiport_disable.is_none());
        port.set_recommended_port_state(
            &RecommendedState::M1(InternalDefaultDS::new(instanceconfig)),
            &InternalDefaultDS::new(instanceconfig),
        );
        assert!(matches!(port.port_state, PortState::Master));
    }

    #[test]
    fn test_announce_receive() {
        let state = setup_test_state();

        let mut port = setup_test_port(&state);

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity.0 = [1, 2, 3, 4, 5, 6, 7, 8];
        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: Default::default(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];

        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_general_receive(packet);
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut port = port.start_bmca();
        port.calculate_best_local_announce_message();
        assert!(port.best_local_announce_message_for_bmca().is_some());
    }

    #[test]
    fn test_announce_receive_via_event() {
        let state = setup_test_state();

        let mut port = setup_test_port(&state);

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity.0 = [1, 2, 3, 4, 5, 6, 7, 8];
        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: Default::default(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];

        let mut actions = port.handle_event_receive(packet, Time::from_micros(1));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_event_receive(packet, Time::from_micros(2));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut actions = port.handle_event_receive(packet, Time::from_micros(3));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
        drop(actions);

        let mut port = port.start_bmca();
        port.calculate_best_local_announce_message();
        assert!(port.best_local_announce_message_for_bmca().is_some());
    }

    #[test]
    fn test_announce_path_trace_loop() {
        let state = setup_test_state();

        let mut state_ref = state.borrow_mut();
        state_ref.parent_ds.parent_port_identity.clock_identity.0 = [1, 2, 3, 4, 5, 6, 7, 8];
        state_ref.path_trace_ds = PathTraceDS::new(true);
        drop(state_ref);

        let mut port = setup_test_port(&state);
        port.set_forced_port_state(PortState::Slave(SlaveState::new(Default::default())));

        let mut announce = default_announce_message();
        announce.header.source_port_identity.clock_identity.0 = [1, 2, 3, 4, 5, 6, 7, 8];

        // Clock loop
        let mut suffix = [0; MAX_DATA_LEN];
        let mut tlv_builder = TlvSetBuilder::new(&mut suffix);
        tlv_builder
            .add(Tlv {
                tlv_type: TlvType::PathTrace,
                value: [0; 8].as_ref().into(),
            })
            .unwrap();

        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: tlv_builder.build(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];

        let mut actions = port.handle_event_receive(packet, Time::from_micros(1));
        assert!(actions.next().is_none());

        drop(actions);

        // No clock loop
        let mut suffix = [0; MAX_DATA_LEN];
        let mut tlv_builder = TlvSetBuilder::new(&mut suffix);
        tlv_builder
            .add(Tlv {
                tlv_type: TlvType::PathTrace,
                value: [0xff; 8].as_ref().into(),
            })
            .unwrap();

        let announce_message = Message {
            header: announce.header,
            body: MessageBody::Announce(announce),
            suffix: tlv_builder.build(),
        };
        let mut packet = [0; MAX_DATA_LEN];
        let packet_len = announce_message.serialize(&mut packet).unwrap();
        let packet = &packet[..packet_len];

        let mut actions = port.handle_event_receive(packet, Time::from_micros(2));
        let Some(PortAction::ResetAnnounceReceiptTimer { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        let Some(PortAction::ForwardTLV { .. }) = actions.next() else {
            panic!("Unexpected action");
        };
        assert!(actions.next().is_none());
    }
}
