use crate::{
    bmc::{
        bmca::{Bmca, RecommendedState},
        dataset_comparison::DefaultDS,
    },
    clock::{TimeProperties, Watch},
    datastructures::{
        common::{ClockIdentity, ClockQuality, PortIdentity, TimeSource, Timestamp},
        messages::{
            AnnounceMessage, DelayReqMessage, DelayRespMessage, FollowUpMessage, Message,
            MessageBuilder, SyncMessage,
        },
    },
    network::{NetworkPacket, NetworkPort, NetworkRuntime},
    time::{Duration, Instant},
};

/// Object for keeping track of the current id
#[derive(Debug, Clone, Default)]
struct IdSequencer {
    cur_id: u16,
}

impl IdSequencer {
    /// Get the next id
    pub fn get(&mut self) -> u16 {
        let result = self.cur_id;
        self.cur_id = self.cur_id.wrapping_add(1);
        result
    }
}

/// Object containing all non-state specific data
pub struct PortData<NR: NetworkRuntime> {
    _runtime: NR,
    tc_port: NR::PortType,
    nc_port: NR::PortType,
    delay_req_ids: IdSequencer,
    identity: PortIdentity,
    sdo: u16,
    domain: u8,
    clock_quality: ClockQuality,
    time_properties: TimeProperties,
    port_config: PortConfig,
    bmca: Bmca,
    announce_seq_id: u16,
    sync_seq_id: u16,
    follow_up_seq_id: u16,
    delay_resp_seq_id: u16,
}

impl<NR: NetworkRuntime> PortData<NR> {
    pub fn new(
        _runtime: NR,
        tc_port: NR::PortType,
        nc_port: NR::PortType,
        identity: PortIdentity,
        sdo: u16,
        domain: u8,
        port_config: PortConfig,
        clock_quality: ClockQuality,
    ) -> Self {
        let bmca = Bmca::new(
            Duration::from_log_interval(port_config.log_announce_interval).into(),
            identity,
        );

        Self {
            _runtime,
            tc_port,
            nc_port,
            delay_req_ids: IdSequencer::default(),
            identity,
            sdo,
            domain,
            clock_quality,
            time_properties: TimeProperties::ArbitraryTime {
                time_traceable: false,
                frequency_traceable: false,
            },
            port_config,
            bmca,
            announce_seq_id: 0,
            sync_seq_id: 0,
            follow_up_seq_id: 0,
            delay_resp_seq_id: 0,
        }
    }
}

pub struct PortConfig {
    pub log_announce_interval: i8,
    pub log_sync_interval: i8,
    pub announce_receipt_timeout: i8,
    pub priority_1: u8,
    pub priority_2: u8,
}

pub struct Port<NR: NetworkRuntime, W: Watch> {
    portdata: PortData<NR>,
    announce_timeout_watch: W,
    announce_watch: W,
    sync_watch: W,
    state: State,
}

#[derive(Debug, Eq, PartialEq)]
pub struct Measurement {
    pub event_time: Instant,
    pub master_offset: Duration,
}

#[derive(Debug, Default, Copy, Clone)]
pub struct StateSlave {
    remote_master: PortIdentity,
    mean_delay: Option<Duration>,
    sync_id: Option<u16>,
    delay_id: Option<u16>,
    delay_send_id: Option<usize>,
    sync_correction: Option<Duration>,
    sync_send_time: Option<Instant>,
    sync_recv_time: Option<Instant>,
    delay_send_time: Option<Instant>,
    delay_recv_time: Option<Instant>,
    next_delay_measurement: Option<Instant>,
    pending_followup: Option<FollowUpMessage>,
}

impl StateSlave {
    fn handle_sync<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
        message: SyncMessage,
        timestamp: Instant,
    ) -> Option<()> {
        self.sync_id = Some(message.header().sequence_id());
        self.sync_recv_time = Some(timestamp);
        self.delay_send_time = None;
        self.delay_recv_time = None;

        if message.header().two_step_flag() {
            self.sync_correction = Some(Duration::from(message.header().correction_field()));
            self.sync_send_time = None;
        } else {
            self.sync_correction = None;
            self.sync_send_time = Some(
                Instant::from(message.origin_timestamp())
                    + Duration::from(message.header().correction_field()),
            );
        }

        if self.mean_delay.is_none() || self.next_delay_measurement.unwrap_or_default() < timestamp
        {
            let delay_id = port.delay_req_ids.get();
            let delay_req = MessageBuilder::new()
                .source_port_identity(port.identity)
                .sequence_id(delay_id)
                .log_message_interval(0x7F)
                .delay_req_message(Timestamp::default());
            let delay_req_encode = delay_req.serialize_vec().unwrap();
            self.delay_send_id = Some(
                port.tc_port
                    .send(&delay_req_encode)
                    .expect("Program error: missing timestamp id"),
            );
            self.delay_id = Some(delay_id);
            self.mean_delay = None;
        } else {
            self.delay_id = None;
        }

        if let Some(follow_up) = self.pending_followup {
            self.handle_followup(follow_up);
        }

        Some(())
    }

    fn handle_followup(&mut self, message: FollowUpMessage) -> Option<()> {
        // Ignore messages not belonging to currently processing sync
        if self.sync_id != Some(message.header().sequence_id()) {
            self.pending_followup = Some(message); // Store it for a potentially coming sync
            return None;
        }

        // Remove any previous pending messages, they are no longer current
        self.pending_followup = None;

        // Absorb into state
        self.sync_send_time = Some(
            Instant::from(message.precise_origin_timestamp())
                + Duration::from(message.header().correction_field())
                + self.sync_correction?,
        );
        self.sync_correction = None;

        Some(())
    }

    fn handle_delayresp(&mut self, message: DelayRespMessage) -> Option<()> {
        // Ignore messages not belonging to currently processing sync
        if self.delay_id? != message.header().sequence_id() {
            return None;
        }

        // Absorb into state
        self.delay_recv_time = Some(
            Instant::from(message.receive_timestamp())
                - Duration::from(message.header().correction_field()),
        );

        // Calculate when we should next measure delay
        //  note that sync_recv_time should always be set here, but if it isn't,
        //  taking the default (0) is safe for recovery.
        self.next_delay_measurement = Some(
            self.sync_recv_time.unwrap_or_default()
                + Duration::from_log_interval(message.header().log_message_interval())
                - Duration::from_fixed_nanos(0.1f64),
        );

        self.finish_delay_measurement();

        Some(())
    }

    /// Handle all messages in the SLAVE state
    fn handle_message<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
        message: Message,
        timestamp: Option<Instant>,
    ) -> Option<()> {
        if message.header().source_port_identity() != self.remote_master {
            return None;
        }

        match message {
            Message::Sync(message) => self.handle_sync(port, message, timestamp?),
            Message::FollowUp(message) => self.handle_followup(message),
            Message::DelayResp(message) => self.handle_delayresp(message),
            _ => None,
        }
    }

    fn handle_send_timestamp(&mut self, id: usize, timestamp: Instant) -> Option<()> {
        if self.delay_send_id? == id {
            self.delay_send_time = Some(timestamp);
            self.delay_send_id = None;
            self.finish_delay_measurement();
            Some(())
        } else {
            None
        }
    }

    fn finish_delay_measurement(&mut self) -> Option<()> {
        self.mean_delay = Some(
            (self.sync_recv_time? - self.sync_send_time?
                + (self.delay_recv_time? - self.delay_send_time?))
                / 2,
        );

        self.delay_send_time = None;
        self.delay_recv_time = None;
        self.delay_id = None;

        Some(())
    }

    fn extract_measurement(&mut self) -> Option<Measurement> {
        let result = Measurement {
            master_offset: self.sync_recv_time? - self.sync_send_time? - self.mean_delay?,
            event_time: self.sync_recv_time?,
        };

        self.sync_recv_time = None;
        self.sync_send_time = None;
        self.sync_id = None;

        Some(result)
    }
}

#[derive(Debug, Default, Copy, Clone)]
pub struct StateMaster {
    priority_1: u8,
    priority_2: u8,
    clock_identity: ClockIdentity,
    clock_quality: ClockQuality,
}

impl StateMaster {
    fn handle_message<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
        message: Message,
        timestamp: Option<Instant>,
    ) -> Option<()> {
        // Always ignore messages from own port
        if message.header().source_port_identity() == port.identity {
            return None;
        }

        match message {
            Message::DelayReq(message) => self.handle_delayreq(message, port, timestamp?),
            _ => None,
        }
    }

    /// Create an announce message
    pub fn send_announce_message<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
    ) -> Option<()> {
        let announce_message = MessageBuilder::new()
            .sequence_id(port.announce_seq_id)
            .source_port_identity(port.identity)
            .announce_message(
                Timestamp::default(),             //origin_timestamp: Timestamp,
                0,                                // TODO implement current_utc_offset: u16,
                self.priority_1,                  //grandmaster_priority_1: u8,
                self.clock_quality,               //grandmaster_clock_quality: ClockQuality,
                self.priority_2,                  //grandmaster_priority_2: u8,
                self.clock_identity,              //grandmaster_identity: ClockIdentity,
                0,                                // TODO implement steps_removed: u16,
                TimeSource::from_primitive(0xa0), // TODO implement time_source: TimeSource,
            );

        port.announce_seq_id = port.announce_seq_id.wrapping_add(1);
        let announce_message_encode = announce_message.serialize_vec().unwrap();
        port.nc_port.send(&announce_message_encode);

        Some(())
    }

    /// Create a sync message
    pub fn send_sync_message<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
        current_time: Instant,
    ) -> Option<()> {
        let sync_message = MessageBuilder::new()
            .sequence_id(port.sync_seq_id)
            .source_port_identity(port.identity)
            .sync_message(Timestamp::from(current_time));

        port.sync_seq_id = port.sync_seq_id.wrapping_add(1);
        let sync_message_encode = sync_message.serialize_vec().unwrap();
        port.tc_port.send(&sync_message_encode);

        Some(())
    }

    /// Create a follow up message
    pub fn send_follow_up_message<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
        current_time: Instant,
    ) -> Option<()> {
        let follow_up_message = MessageBuilder::new()
            .sequence_id(port.sync_seq_id)
            .source_port_identity(port.identity)
            .follow_up_message(Timestamp::from(current_time));

        port.follow_up_seq_id = port.follow_up_seq_id.wrapping_add(1);
        let follow_up_message_encode = follow_up_message.serialize_vec().unwrap();
        port.nc_port.send(&follow_up_message_encode);

        Some(())
    }

    /// Handle delay req by sending a delay resp
    fn handle_delayreq<NR: NetworkRuntime>(
        &mut self,
        message: DelayReqMessage,
        port: &mut PortData<NR>,
        timestamp: Instant,
    ) -> Option<()> {
        // Send delay response
        let delay_resp_message = MessageBuilder::new()
            .sequence_id(port.delay_resp_seq_id)
            .source_port_identity(port.identity)
            .delay_resp_message(
                Timestamp::from(timestamp),
                message.header().source_port_identity(),
            );

        port.delay_resp_seq_id = port.delay_resp_seq_id.wrapping_add(1);
        let delay_resp_encode = delay_resp_message.serialize_vec().unwrap();
        port.nc_port.send(&delay_resp_encode);

        Some(())
    }
}

#[derive(Debug, Copy, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum State {
    Listening,
    Slave(StateSlave),
    Master(StateMaster),
}

impl State {
    fn handle_message<NR: NetworkRuntime>(
        &mut self,
        port: &mut PortData<NR>,
        message: Message,
        timestamp: Option<Instant>,
    ) -> Option<()> {
        match self {
            State::Slave(state) => state.handle_message(port, message, timestamp),
            State::Master(state) => state.handle_message(port, message, timestamp),
            _ => None,
        }
    }

    fn handle_send_timestamp(&mut self, id: usize, timestamp: Instant) -> Option<()> {
        match self {
            State::Slave(state) => state.handle_send_timestamp(id, timestamp),
            _ => None,
        }
    }

    fn handle_recommended_state<W: Watch, NR: NetworkRuntime>(
        &mut self,
        recommended_state: &RecommendedState,
        announce_timeout_watch: &mut W,
        port: &mut PortData<NR>,
        announce_watch: &mut W,
        sync_watch: &mut W,
    ) {
        let announce_receipt_timeout = port.port_config.announce_receipt_timeout;
        let log_announce_interval = port.port_config.log_announce_interval;
        let log_sync_interval = port.port_config.log_sync_interval;

        match recommended_state {
            // TODO set things like steps_removed once they are added
            // TODO make sure states are complete
            RecommendedState::S1(announce_message) => match self {
                State::Listening => {
                    *self = State::Slave(StateSlave {
                        remote_master: announce_message.header().source_port_identity(),
                        ..Default::default()
                    });

                    // Restart announce receipt timeout timer
                    announce_timeout_watch.set_alarm(Duration::from_log_interval(
                        announce_receipt_timeout * log_announce_interval,
                    ));

                    log::info!(
                        "New state for port: Listening -> Slave. Remote master: {:?}",
                        announce_message
                            .header()
                            .source_port_identity()
                            .clock_identity
                    );
                }

                State::Slave(slave_state) => {
                    slave_state.remote_master = announce_message.header().source_port_identity();
                }

                // Transition MASTER to SLAVE
                State::Master(_master_state) => {
                    *self = State::Slave(StateSlave {
                        remote_master: announce_message.header().source_port_identity(),
                        ..Default::default()
                    });

                    // Stop MASTER watches
                    announce_watch.clear();
                    sync_watch.clear();

                    // Restart announce receipt timeout timer
                    announce_timeout_watch.set_alarm(Duration::from_log_interval(
                        announce_receipt_timeout * log_announce_interval,
                    ));

                    log::info!("New state for port: Master -> Slave");
                }
            },

            // Recommended state is master
            RecommendedState::M2(default_ds) => match self {
                // Stay master
                State::Master(_master_state) => {
                    *self = State::Master(StateMaster {
                        priority_1: default_ds.priority_1,
                        priority_2: default_ds.priority_2,
                        clock_identity: default_ds.clock_identity,
                        clock_quality: default_ds.clock_quality,
                    });
                }

                // Otherwise become master
                _ => {
                    // Stop the announce timeout alarm
                    announce_timeout_watch.clear();

                    *self = State::Master(StateMaster {
                        priority_1: default_ds.priority_1,
                        priority_2: default_ds.priority_2,
                        clock_identity: default_ds.clock_identity,
                        clock_quality: default_ds.clock_quality,
                    });

                    // Reset sequences in portdata?
                    port.announce_seq_id = 0;
                    port.sync_seq_id = 0;
                    port.follow_up_seq_id = 0;
                    port.delay_resp_seq_id = 0;

                    log::info!("New state for port: Master");

                    // Start sending announce messages
                    announce_watch.set_alarm(Duration::from_log_interval(log_announce_interval));

                    // Start sending sync messages
                    sync_watch.set_alarm(Duration::from_log_interval(log_sync_interval));
                }
            },

            // All other cases
            _ => match self {
                State::Listening => {
                    // Ignore
                }

                _ => {
                    *self = State::Listening;

                    // Stop MASTER watches
                    announce_watch.clear();
                    sync_watch.clear();

                    // Restart announce receipt timeout timer
                    announce_timeout_watch.set_alarm(Duration::from_log_interval(
                        announce_receipt_timeout * log_announce_interval,
                    ));

                    log::info!("New state for port: Listening");
                }
            },
        }
    }
}

impl<NR: NetworkRuntime, W: Watch> Port<NR, W> {
    pub fn new(
        identity: PortIdentity,
        sdo: u16,
        domain: u8,
        port_config: PortConfig,
        mut runtime: NR,
        interface: NR::InterfaceDescriptor,
        clock_quality: ClockQuality,
        announce_timeout_watch: W,
        announce_watch: W,
        sync_watch: W,
    ) -> Self {
        // Ptp needs two ports, 1 time critical one and 1 general port
        let tc_port = runtime
            .open(interface.clone(), true)
            .expect("Could not create time critical port");
        let nc_port = runtime
            .open(interface, false)
            .expect("Could not create non time critical port");

        Port {
            portdata: PortData::new(
                runtime,
                tc_port,
                nc_port,
                identity,
                sdo,
                domain,
                port_config,
                clock_quality,
            ),
            state: State::Listening,
            announce_timeout_watch: announce_timeout_watch,
            announce_watch,
            sync_watch,
        }
    }

    pub fn handle_alarm(&mut self, id: W::WatchId, current_time: Instant) {
        // When the announce timout expires, it means there
        // have been no announce messages in a while, so we
        // force a switch to the master state
        if id == self.announce_timeout_watch.id() {
            log::info!("Announce interval timeout");

            self.state = State::Master(StateMaster {
                priority_1: self.portdata.port_config.priority_1,
                priority_2: self.portdata.port_config.priority_2,
                clock_identity: self.portdata.identity.clock_identity,
                clock_quality: self.portdata.clock_quality,
            });

            // Reset sequences in portdata
            self.portdata.announce_seq_id = 0;
            self.portdata.sync_seq_id = 0;
            self.portdata.follow_up_seq_id = 0;
            self.portdata.delay_resp_seq_id = 0;

            log::info!("New state for port: Master");

            // Start sending announce messages
            self.announce_watch.set_alarm(Duration::from_log_interval(
                self.portdata.port_config.log_announce_interval,
            ));

            // Start sending sync messages
            self.sync_watch.set_alarm(Duration::from_log_interval(
                self.portdata.port_config.log_sync_interval,
            ));
        }

        // When the announce watch expires, send an announce message and restart
        if id == self.announce_watch.id() {
            self.send_announce_message();
            self.announce_watch.set_alarm(Duration::from_log_interval(
                self.portdata.port_config.log_announce_interval,
            ));
        }

        // When the sync watch expires, send a sync message and restart
        if id == self.sync_watch.id() {
            self.send_sync_message(current_time);

            // TODO: Is the follow up a config?
            self.send_follow_up_message(current_time);

            self.sync_watch.set_alarm(Duration::from_log_interval(
                self.portdata.port_config.log_sync_interval,
            ));
        }
    }

    /// Send an announce message
    pub fn send_announce_message(&mut self) {
        match self.state {
            State::Master(mut state) => &state.send_announce_message(&mut self.portdata),
            _ => &None,
        };
    }

    /// Send a sync message
    pub fn send_sync_message(&mut self, current_time: Instant) {
        match self.state {
            State::Master(mut state) => &state.send_sync_message(&mut self.portdata, current_time),
            _ => &None,
        };
    }

    /// Send a follow up message
    pub fn send_follow_up_message(&mut self, current_time: Instant) {
        match self.state {
            State::Master(mut state) => {
                &state.send_follow_up_message(&mut self.portdata, current_time)
            }
            _ => &None,
        };
    }

    pub fn handle_network(&mut self, packet: NetworkPacket, current_time: Instant) {
        self.process_message(packet, current_time);
    }

    pub fn handle_send_timestamp(&mut self, id: usize, timestamp: Instant) {
        self.state.handle_send_timestamp(id, timestamp);
    }

    /// Process messages, but only if they are from the same domain
    fn process_message(&mut self, packet: NetworkPacket, current_time: Instant) -> Option<()> {
        let message = Message::deserialize(&packet.data).ok()?;
        if message.header().sdo_id() != self.portdata.sdo
            || message.header().domain_number() != self.portdata.domain
        {
            return None;
        }

        self.state
            .handle_message(&mut self.portdata, message, packet.timestamp);

        #[allow(clippy::single_match)]
        match message {
            Message::Announce(announce) => {
                self.portdata
                    .bmca
                    .register_announce_message(&announce, current_time.into());

                // When an announce message is received, restart announce receipt timeout timer
                self.announce_timeout_watch
                    .set_alarm(Duration::from_log_interval(
                        self.portdata.port_config.announce_receipt_timeout
                            * self.portdata.port_config.log_announce_interval,
                    ));
            }
            _ => {}
        };

        None
    }

    pub fn extract_measurement(&mut self) -> Option<(Measurement, TimeProperties)> {
        match &mut self.state {
            State::Slave(state) => state
                .extract_measurement()
                .map(|measurement| (measurement, self.portdata.time_properties)),
            _ => None,
        }
    }

    pub fn take_best_port_announce_message(
        &mut self,
        current_time: Instant,
    ) -> Option<(AnnounceMessage, Timestamp, PortIdentity)> {
        self.portdata
            .bmca
            .take_best_port_announce_message(current_time.into())
    }

    pub fn perform_state_decision(
        &mut self,
        best_global_announce_message: Option<(&AnnounceMessage, &PortIdentity)>,
        best_port_announce_message: Option<(&AnnounceMessage, &PortIdentity)>,
    ) {
        let own_data = DefaultDS {
            priority_1: self.portdata.port_config.priority_1,
            clock_identity: self.portdata.identity.clock_identity,
            clock_quality: self.portdata.clock_quality,
            priority_2: self.portdata.port_config.priority_2,
        };

        let recommended_state = Bmca::calculate_recommended_state(
            &own_data,
            best_global_announce_message,
            best_port_announce_message,
            &self.state,
        );

        if let Some(recommended_state) = recommended_state {
            self.state.handle_recommended_state(
                &recommended_state,
                &mut self.announce_timeout_watch,
                &mut self.portdata,
                &mut self.announce_watch,
                &mut self.sync_watch,
            );
            #[allow(clippy::single_match)]
            match &recommended_state {
                RecommendedState::S1(announce_message) => {
                    self.portdata.time_properties = announce_message.time_properties();
                }
                _ => {}
            }
        }
    }

    pub fn get_log_announce_interval(&self) -> Duration {
        Duration::from_log_interval(self.portdata.port_config.log_announce_interval)
    }
}

#[cfg(test)]
mod tests {
    use super::{IdSequencer, PortData, StateSlave};
    use crate::{
        bmc::bmca::Bmca,
        datastructures::{
            common::{ClockQuality, PortIdentity, TimeInterval, Timestamp},
            messages::MessageBuilder,
        },
        network::{test::TestRuntime, NetworkRuntime},
        port::{Measurement, PortConfig},
        time::{Duration, Instant},
    };
    use fixed::traits::ToFixed;

    #[test]
    fn test_measurement_flow() {
        let mut network_runtime = TestRuntime::default();

        let master_id = PortIdentity::default();
        let mut test_id = PortIdentity::default();
        test_id.clock_identity.0[0] += 1;

        let mut test_state = StateSlave {
            remote_master: master_id,
            ..Default::default()
        };

        let mut test_port_data = PortData {
            _runtime: network_runtime.clone(),
            tc_port: network_runtime.open("".to_owned(), true).unwrap(),
            nc_port: network_runtime.open("".to_owned(), false).unwrap(),
            delay_req_ids: IdSequencer::default(),
            identity: test_id,
            sdo: 0,
            domain: 0,
            port_config: PortConfig {
                log_announce_interval: 1,
                log_sync_interval: 1,
                announce_receipt_timeout: 5,
                priority_1: 0,
                priority_2: 0,
            },
            bmca: Bmca::new(TimeInterval(2_000_000_000u64.to_fixed()), test_id),
            clock_quality: ClockQuality::default(),
            time_properties: crate::clock::TimeProperties::ArbitraryTime {
                time_traceable: false,
                frequency_traceable: false,
            },
            announce_seq_id: 0,
            delay_resp_seq_id: 0,
            follow_up_seq_id: 0,
            sync_seq_id: 0,
        };

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .correction_field(TimeInterval((1 as i16).to_fixed()))
                .sync_message(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
            Some(Instant::from_nanos(5)),
        );

        assert_eq!(test_state.extract_measurement(), None);

        let delay_req = network_runtime.get_sent().unwrap();
        test_state.handle_send_timestamp(delay_req.index, Instant::from_nanos(7));

        assert_eq!(test_state.extract_measurement(), None);

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .correction_field(TimeInterval((2 as i16).to_fixed()))
                .delay_resp_message(
                    Timestamp {
                        seconds: 0,
                        nanos: 11,
                    },
                    test_id,
                ),
            None,
        );

        assert_eq!(
            test_state.extract_measurement(),
            Some(Measurement {
                master_offset: Duration::from_nanos(1),
                event_time: Instant::from_nanos(5),
            })
        );
    }

    #[test]
    fn test_measurement_flow_timestamps_out_of_order() {
        let mut network_runtime = TestRuntime::default();

        let master_id = PortIdentity::default();
        let mut test_id = PortIdentity::default();
        test_id.clock_identity.0[0] += 1;

        let mut test_state = StateSlave {
            remote_master: master_id,
            ..Default::default()
        };

        let mut test_port_data = PortData {
            _runtime: network_runtime.clone(),
            tc_port: network_runtime.open("".to_owned(), true).unwrap(),
            nc_port: network_runtime.open("".to_owned(), false).unwrap(),
            delay_req_ids: IdSequencer::default(),
            identity: test_id,
            sdo: 0,
            domain: 0,
            port_config: PortConfig {
                log_announce_interval: 1,
                log_sync_interval: 1,
                announce_receipt_timeout: 5,
                priority_1: 0,
                priority_2: 0,
            },
            bmca: Bmca::new(TimeInterval(2_000_000_000u64.to_fixed()), test_id),
            clock_quality: ClockQuality::default(),
            time_properties: crate::clock::TimeProperties::ArbitraryTime {
                time_traceable: false,
                frequency_traceable: false,
            },
            announce_seq_id: 0,
            delay_resp_seq_id: 0,
            follow_up_seq_id: 0,
            sync_seq_id: 0,
        };

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .correction_field(TimeInterval((1 as i16).to_fixed()))
                .sync_message(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
            Some(Instant::from_nanos(5)),
        );

        assert_eq!(test_state.extract_measurement(), None);

        let delay_req = network_runtime.get_sent().unwrap();

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .correction_field(TimeInterval((2 as i16).to_fixed()))
                .delay_resp_message(
                    Timestamp {
                        seconds: 0,
                        nanos: 11,
                    },
                    test_id,
                ),
            None,
        );

        assert_eq!(test_state.extract_measurement(), None);

        test_state.handle_send_timestamp(delay_req.index, Instant::from_nanos(7));

        assert_eq!(
            test_state.extract_measurement(),
            Some(Measurement {
                master_offset: Duration::from_nanos(1),
                event_time: Instant::from_nanos(5),
            })
        );
    }

    #[test]
    fn test_measurement_flow_followup() {
        let mut network_runtime = TestRuntime::default();

        let master_id = PortIdentity::default();
        let mut test_id = PortIdentity::default();
        test_id.clock_identity.0[0] += 1;

        let mut test_state = StateSlave {
            remote_master: master_id,
            ..Default::default()
        };

        let mut test_port_data = PortData {
            _runtime: network_runtime.clone(),
            tc_port: network_runtime.open("".to_owned(), true).unwrap(),
            nc_port: network_runtime.open("".to_owned(), false).unwrap(),
            delay_req_ids: IdSequencer::default(),
            identity: test_id,
            sdo: 0,
            domain: 0,
            port_config: PortConfig {
                log_announce_interval: 1,
                log_sync_interval: 1,
                announce_receipt_timeout: 5,
                priority_1: 0,
                priority_2: 0,
            },
            bmca: Bmca::new(TimeInterval(2_000_000_000u64.to_fixed()), test_id),
            clock_quality: ClockQuality::default(),
            time_properties: crate::clock::TimeProperties::ArbitraryTime {
                time_traceable: false,
                frequency_traceable: false,
            },
            announce_seq_id: 0,
            delay_resp_seq_id: 0,
            follow_up_seq_id: 0,
            sync_seq_id: 0,
        };

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .two_step_flag(true)
                .correction_field(TimeInterval((1 as i16).to_fixed()))
                .sync_message(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
            Some(Instant::from_nanos(5)),
        );

        assert_eq!(test_state.extract_measurement(), None);

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .two_step_flag(true)
                .correction_field(TimeInterval((1 as i16).to_fixed()))
                .follow_up_message(Timestamp {
                    seconds: 0,
                    nanos: 1,
                }),
            None,
        );

        assert_eq!(test_state.extract_measurement(), None);

        let delay_req = network_runtime.get_sent().unwrap();
        test_state.handle_send_timestamp(delay_req.index, Instant::from_nanos(7));

        assert_eq!(test_state.extract_measurement(), None);

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .correction_field(TimeInterval((2 as i16).to_fixed()))
                .delay_resp_message(
                    Timestamp {
                        seconds: 0,
                        nanos: 11,
                    },
                    test_id,
                ),
            None,
        );

        assert_eq!(
            test_state.extract_measurement(),
            Some(Measurement {
                master_offset: Duration::from_nanos(0),
                event_time: Instant::from_nanos(5),
            })
        );
    }

    #[test]
    fn test_measurement_flow_followup_out_of_order() {
        let mut network_runtime = TestRuntime::default();

        let master_id = PortIdentity::default();
        let mut test_id = PortIdentity::default();
        test_id.clock_identity.0[0] += 1;

        let mut test_state = StateSlave {
            remote_master: master_id,
            ..Default::default()
        };

        let mut test_port_data = PortData {
            _runtime: network_runtime.clone(),
            tc_port: network_runtime.open("".to_owned(), true).unwrap(),
            nc_port: network_runtime.open("".to_owned(), false).unwrap(),
            delay_req_ids: IdSequencer::default(),
            identity: test_id,
            sdo: 0,
            domain: 0,
            port_config: PortConfig {
                log_announce_interval: 1,
                log_sync_interval: 1,
                announce_receipt_timeout: 5,
                priority_1: 0,
                priority_2: 0,
            },
            bmca: Bmca::new(TimeInterval(2_000_000_000u64.to_fixed()), test_id),
            clock_quality: ClockQuality::default(),
            time_properties: crate::clock::TimeProperties::ArbitraryTime {
                time_traceable: false,
                frequency_traceable: false,
            },
            announce_seq_id: 0,
            delay_resp_seq_id: 0,
            follow_up_seq_id: 0,
            sync_seq_id: 0,
        };

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .two_step_flag(true)
                .correction_field(TimeInterval((1 as i16).to_fixed()))
                .follow_up_message(Timestamp {
                    seconds: 0,
                    nanos: 1,
                }),
            None,
        );

        assert_eq!(test_state.extract_measurement(), None);

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .two_step_flag(true)
                .correction_field(TimeInterval((1 as i16).to_fixed()))
                .sync_message(Timestamp {
                    seconds: 0,
                    nanos: 0,
                }),
            Some(Instant::from_nanos(5)),
        );

        assert_eq!(test_state.extract_measurement(), None);

        let delay_req = network_runtime.get_sent().unwrap();
        test_state.handle_send_timestamp(delay_req.index, Instant::from_nanos(7));

        assert_eq!(test_state.extract_measurement(), None);

        test_state.handle_message(
            &mut test_port_data,
            MessageBuilder::new()
                .sdo_id(0)
                .unwrap()
                .domain_number(0)
                .correction_field(TimeInterval((2 as i16).to_fixed()))
                .delay_resp_message(
                    Timestamp {
                        seconds: 0,
                        nanos: 11,
                    },
                    test_id,
                ),
            None,
        );

        assert_eq!(
            test_state.extract_measurement(),
            Some(Measurement {
                master_offset: Duration::from_nanos(0),
                event_time: Instant::from_nanos(5),
            })
        );
    }
}
