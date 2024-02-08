use core::fmt::{Display, Formatter};

use super::sequence_id::SequenceIdGenerator;
use crate::{
    datastructures::common::PortIdentity,
    filters::Filter,
    time::{Duration, Time},
};

#[derive(Debug, Default)]
#[allow(private_interfaces)]
pub(crate) enum PortState<F> {
    #[default]
    Listening,
    Master(MasterState),
    Passive,
    Slave(SlaveState<F>),
}

impl<F> Display for PortState<F> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PortState::Listening => write!(f, "Listening"),
            PortState::Master(_) => write!(f, "Master"),
            PortState::Passive => write!(f, "Passive"),
            PortState::Slave(_) => write!(f, "Slave"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SlaveState<F> {
    pub(super) remote_master: PortIdentity,

    pub(super) sync_state: SyncState,
    pub(super) delay_state: DelayState,

    pub(super) mean_delay: Option<Duration>,
    pub(super) last_raw_sync_offset: Option<Duration>,

    pub(super) delay_req_ids: SequenceIdGenerator,

    pub(super) filter: F,
}

impl<F> SlaveState<F> {
    pub(crate) fn remote_master(&self) -> PortIdentity {
        self.remote_master
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SyncState {
    Empty,
    Measuring {
        id: u16,
        send_time: Option<Time>,
        recv_time: Option<Time>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DelayState {
    Empty,
    Measuring {
        id: u16,
        send_time: Option<Time>,
        recv_time: Option<Time>,
    },
}

impl<F: Filter> SlaveState<F> {
    pub(super) fn new(remote_master: PortIdentity, filter_config: F::Config) -> Self {
        SlaveState {
            remote_master,
            sync_state: SyncState::Empty,
            delay_state: DelayState::Empty,
            mean_delay: None,
            last_raw_sync_offset: None,
            delay_req_ids: SequenceIdGenerator::new(),
            filter: F::new(filter_config),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(super) struct MasterState {
    pub(super) announce_seq_ids: SequenceIdGenerator,
    pub(super) sync_seq_ids: SequenceIdGenerator,
}

impl MasterState {
    pub(super) fn new() -> Self {
        MasterState {
            announce_seq_ids: SequenceIdGenerator::new(),
            sync_seq_ids: SequenceIdGenerator::new(),
        }
    }
}
