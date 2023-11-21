//! Statime is a library providing an implementation of PTP version 2.1
//! (IEEE1588-2019). It provides all the building blocks to setup PTP ordinary
//! and boundary clocks.
//!
//! Note: We are currently planning a major overhaul of the library. This will
//! also result in significant changes to the public API.
//!
//! # Device interfaces
//! `statime` is designed to be able to work with many different underlying
//! platforms, including embedded targets. This does mean that it cannot use the
//! standard library and platform specific libraries to interact with the system
//! clock and to access the network. That needs to be provided by the user of
//! the library.
//!
//! The `statime` crate defines a [`Clock`] interface that provide access to the
//! system clock. The [`PtpInstance`] and [`Port`](`port::Port`)
//! abstractions provide the needed glue to access the network.
//!
//! On modern linux kernels, the `statime-linux` crate provides ready to use
//! implementations of these interfaces. For other platforms the user will need
//! to implement these themselves.
//!
//!
//! --------
//!
//! # What is this?
//! * What it statime?
//! * What is PTP?
//! * Where can I get more information?
//!
//! # How do I use this?
//! * Configurations
//! * Setup Interface
//! * Setup Ports
//! * Running the Ports
//! * Running the BMCA
//!
//! # How can I verify/test my implementation?
//! * Run it against statime-linux?

#![no_std]

#[cfg(feature = "std")]
extern crate std;

mod bmc;
mod clock;
pub mod config;
pub(crate) mod datastructures;
pub mod filters;
pub mod port;
mod ptp_instance;
pub mod time;

pub use clock::Clock;
pub use ptp_instance::PtpInstance;

#[cfg(feature = "fuzz")]
pub mod fuzz {
    pub use crate::datastructures::messages::FuzzMessage;
}
