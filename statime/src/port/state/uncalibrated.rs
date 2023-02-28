use crate::datastructures::common::PortIdentity;
use crate::port::state::{PortState, SlaveState};

pub trait Calibrate: Sized {
    async fn calibrate(remote_master: PortIdentity) -> PortState<Self>;
}

#[derive(Debug)]
pub struct DefaultUncalibratedState;

impl Calibrate for DefaultUncalibratedState {
    async fn calibrate(remote_master: PortIdentity) -> PortState<Self> {
        // Do nothing and transition to slave directly
        PortState::Slave(SlaveState::new(remote_master))
    }
}
