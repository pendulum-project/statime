use crate::datastructures::common::{PortIdentity, Timestamp};
use crate::datastructures::messages::{
    DelayRespMessage, FollowUpMessage, Message, MessageBuilder, SyncMessage,
};
use crate::network::NetworkPort;
use crate::port::sequence_id::SequenceIdGenerator;
use crate::port::Measurement;
use crate::time::{Duration, Instant};
use thiserror::Error;

type Result<T, E = SlaveError> = core::result::Result<T, E>;

#[derive(Debug)]
pub struct SlaveState {
    remote_master: PortIdentity,

    sync_state: SyncState,
    delay_state: DelayState,

    delay_req_ids: SequenceIdGenerator,

    next_delay_measurement: Option<Instant>,
    pending_followup: Option<FollowUpMessage>,
}

impl SlaveState {
    pub fn remote_master(&self) -> PortIdentity {
        self.remote_master
    }
}

#[derive(Debug)]
enum SyncState {
    Initial,
    AfterSync {
        sync_id: u16,
        sync_recv_time: Instant,
        sync_correction: Duration,
    },
    AfterFollowUp {
        sync_recv_time: Instant,
        sync_send_time: Instant,
    },
}

#[derive(Debug)]
enum DelayState {
    Initial,
    AfterSync {
        delay_id: u16,
        delay_send_time: Instant,
    },
    AfterDelayResp {
        mean_delay: Duration,
    },
}

impl DelayState {
    pub fn finished(&self) -> bool {
        match self {
            DelayState::Initial | DelayState::AfterSync { .. } => false,
            DelayState::AfterDelayResp { .. } => true,
        }
    }
}

impl SlaveState {
    pub fn new(remote_master: PortIdentity) -> Self {
        SlaveState {
            remote_master,
            sync_state: SyncState::Initial,
            delay_state: DelayState::Initial,
            delay_req_ids: SequenceIdGenerator::new(),
            next_delay_measurement: None,
            pending_followup: None,
        }
    }

    pub(crate) async fn handle_message<P: NetworkPort>(
        &mut self,
        message: Message,
        current_time: Instant,
        network_port: &mut P,
        port_identity: PortIdentity,
    ) -> Result<()> {
        // Only listen to master
        if message.header().source_port_identity() == self.remote_master {
            match message {
                Message::Sync(message) => {
                    self.handle_sync(message, current_time, network_port, port_identity)
                        .await
                }
                Message::FollowUp(message) => self.handle_follow_up(message),
                Message::DelayResp(message) => self.handle_delay_resp(message),
                _ => Err(SlaveError::UnexpectedMessage),
            }
        } else {
            Ok(())
        }
    }

    async fn handle_sync<P: NetworkPort>(
        &mut self,
        message: SyncMessage,
        current_time: Instant,
        network_port: &mut P,
        port_identity: PortIdentity,
    ) -> Result<()> {
        self.sync_state = if message.header().two_step_flag() {
            SyncState::AfterSync {
                sync_id: message.header().sequence_id(),
                sync_recv_time: current_time,
                sync_correction: Duration::from(message.header().correction_field()),
            }
        } else {
            SyncState::AfterFollowUp {
                sync_recv_time: current_time,
                sync_send_time: Instant::from(message.origin_timestamp())
                    + Duration::from(message.header().correction_field()),
            }
        };

        if !self.delay_state.finished()
            || self.next_delay_measurement.unwrap_or_default() < current_time
        {
            let delay_id = self.delay_req_ids.generate();
            let delay_req = MessageBuilder::new()
                .source_port_identity(port_identity)
                .sequence_id(delay_id)
                .log_message_interval(0x7F)
                .delay_req_message(Timestamp::default());
            let delay_req_encode = delay_req.serialize_vec().unwrap();
            let delay_send_time = network_port
                .send_time_critical(&delay_req_encode)
                .await
                .expect("Program error: missing timestamp id");
            self.delay_state = DelayState::AfterSync {
                delay_id,
                delay_send_time,
            };
        }

        if let Some(follow_up) = self.pending_followup {
            self.handle_follow_up(follow_up)?;
        }

        Ok(())
    }

    fn handle_follow_up(&mut self, message: FollowUpMessage) -> Result<()> {
        match self.sync_state {
            SyncState::AfterSync {
                sync_id,
                sync_recv_time,
                sync_correction,
            } => {
                // Ignore messages not belonging to currently processing sync
                if sync_id == message.header().sequence_id() {
                    // Remove any previous pending messages, they are no longer current
                    self.pending_followup = None;

                    // Absorb into state
                    let sync_send_time = Instant::from(message.precise_origin_timestamp())
                        + Duration::from(message.header().correction_field())
                        + sync_correction;
                    self.sync_state = SyncState::AfterFollowUp {
                        sync_recv_time,
                        sync_send_time,
                    };

                    Ok(())
                } else {
                    // Store it for a potentially coming sync
                    self.pending_followup = Some(message);
                    Ok(())
                }
            }
            // Wrong state
            SyncState::Initial | SyncState::AfterFollowUp { .. } => {
                // Store it for a potentially coming sync
                self.pending_followup = Some(message);
                Ok(())
            }
        }
    }

    fn handle_delay_resp(&mut self, message: DelayRespMessage) -> Result<()> {
        match self.sync_state {
            SyncState::AfterFollowUp {
                sync_recv_time,
                sync_send_time,
            } => {
                match self.delay_state {
                    DelayState::AfterSync {
                        delay_id,
                        delay_send_time,
                    } => {
                        // Ignore messages not belonging to currently processing sync
                        if delay_id != message.header().sequence_id() {
                            return Ok(());
                        }

                        // Absorb into state
                        let delay_recv_time = Instant::from(message.receive_timestamp())
                            - Duration::from(message.header().correction_field());

                        // Calculate when we should next measure delay
                        //  note that sync_recv_time should always be set here, but if it isn't,
                        //  taking the default (0) is safe for recovery.
                        self.next_delay_measurement = Some(
                            sync_recv_time
                                + Duration::from_log_interval(
                                    message.header().log_message_interval(),
                                )
                                - Duration::from_fixed_nanos(0.1f64),
                        );

                        let mean_delay = (sync_recv_time - sync_send_time
                            + (delay_recv_time - delay_send_time))
                            / 2;

                        self.delay_state = DelayState::AfterDelayResp { mean_delay };

                        Ok(())
                    }
                    // Wrong state
                    DelayState::Initial | DelayState::AfterDelayResp { .. } => {
                        Err(SlaveError::OutOfSequence)
                    }
                }
            }
            // Wrong state
            SyncState::Initial | SyncState::AfterSync { .. } => Err(SlaveError::OutOfSequence),
        }
    }

    pub(crate) fn extract_measurement(&mut self) -> Option<Measurement> {
        match self.sync_state {
            SyncState::AfterFollowUp {
                sync_recv_time,
                sync_send_time,
                ..
            } => {
                match self.delay_state {
                    DelayState::AfterDelayResp { mean_delay } => {
                        let result = Measurement {
                            master_offset: sync_recv_time - sync_send_time - mean_delay,
                            event_time: sync_recv_time,
                        };

                        self.sync_state = SyncState::Initial;

                        Some(result)
                    }
                    // Wrong state
                    DelayState::Initial | DelayState::AfterSync { .. } => None,
                }
            }
            // Wrong state
            SyncState::Initial | SyncState::AfterSync { .. } => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum SlaveError {
    #[error("received a message that a port in the slave state can never process")]
    UnexpectedMessage,
    #[error("received a message that can usually be processed, but not right now")]
    OutOfSequence,
}
