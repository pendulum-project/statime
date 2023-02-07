#![cfg_attr(all(not(test), not(feature = "std")), no_std)]

use core::fmt::Display;
use datastructures::common::PortIdentity;

pub mod bmc;
pub mod clock;
pub mod datastructures;
pub mod filters;
pub mod network;
pub mod port;
pub mod ptp_instance;
pub mod time;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    PortBecameInitializing {
        port_id: PortIdentity,
    },
    PortBecameFaulty {
        port_id: PortIdentity,
    },
    PortBecameDisabled {
        port_id: PortIdentity,
    },
    PortBecameListening {
        port_id: PortIdentity,
    },
    PortBecamePreMaster {
        port_id: PortIdentity,
    },
    PortBecameMaster {
        port_id: PortIdentity,
    },
    PortBecamePassive {
        port_id: PortIdentity,
    },
    PortBecameUncalibrated {
        port_id: PortIdentity,
    },
    PortBecameSlave {
        port_id: PortIdentity,
        master_port_id: PortIdentity,
    },
}

impl Display for Event {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Event::PortBecameInitializing { port_id } => {
                write!(f, "Port {port_id} became `Initializing`")
            }
            Event::PortBecameFaulty { port_id } => write!(f, "Port {port_id} became `Faulty`"),
            Event::PortBecameDisabled { port_id } => write!(f, "Port {port_id} became `Disabled`"),
            Event::PortBecameListening { port_id } => {
                write!(f, "Port {port_id} became `Listening`")
            }
            Event::PortBecamePreMaster { port_id } => {
                write!(f, "Port {port_id} became `PreMaster`")
            }
            Event::PortBecameMaster { port_id } => write!(f, "Port {port_id} became `Master`"),
            Event::PortBecamePassive { port_id } => write!(f, "Port {port_id} became `Passive`"),
            Event::PortBecameUncalibrated { port_id } => {
                write!(f, "Port {port_id} became `Uncalibrated`")
            }
            Event::PortBecameSlave {
                port_id,
                master_port_id,
            } => write!(f, "Port {port_id} became `Slave` to {master_port_id}"),
        }
    }
}
