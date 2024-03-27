use core::fmt::{Display, Formatter};

use crate::{
    datastructures::common::PortIdentity,
    time::{Duration, Time},
};

#[derive(Debug, Default)]
#[allow(private_interfaces)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum PortState {
    #[default]
    Faulty,
    Listening,
    Master,
    Passive,
    Slave(SlaveState),
}

impl Display for PortState {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            PortState::Listening => write!(f, "Listening"),
            PortState::Master => write!(f, "Master"),
            PortState::Passive => write!(f, "Passive"),
            PortState::Slave(_) => write!(f, "Slave"),
            PortState::Faulty => write!(f, "Faulty"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SlaveState {
    pub(super) remote_master: PortIdentity,

    pub(super) sync_state: SyncState,
    pub(super) delay_state: DelayState,

    pub(super) last_raw_sync_offset: Option<Duration>,
}

impl SlaveState {
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

impl SlaveState {
    pub(super) fn new(remote_master: PortIdentity) -> Self {
        SlaveState {
            remote_master,
            sync_state: SyncState::Empty,
            delay_state: DelayState::Empty,
            last_raw_sync_offset: None,
        }
    }
}
