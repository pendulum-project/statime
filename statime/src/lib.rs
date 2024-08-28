//! Statime is a library providing an implementation of PTP version 2.1
//! (IEEE1588-2019). It provides all the building blocks to setup PTP ordinary
//! and boundary clocks.
//!
//! `statime` is designed to be able to work with many different underlying
//! platforms, including embedded targets. This does mean that it cannot use the
//! standard library and platform specific libraries to interact with the system
//! clock and to access the network. That needs to be provided by the user of
//! the library.
//!
//! On modern linux kernels, the `statime-linux` crate provides ready to use
//! implementations of these interfaces. For other platforms the user will need
//! to implement these themselves.
//!
//! The `statime-stm32` crate gives an example of how to use `statime` on an
//! embedded target.
//!
//! # Implementing a PTP clock with `statime`
//! Implementing a clock device requires three parts. The [`PtpInstance`]
//! handles the logic for the Best Master Clock Algorithm (BMCA). A
//! [`Port`](`port::Port`) handles the logic for a single network interface of
//! the device. And a [`Clock`] per [`Port`](`port::Port`) that is a struct
//! provided by the user that can read and control a clock device that is
//! associated with a [`Port`](`port::Port`).
//!
//! ## Setup
//! The first step for a new implementation is to gather the configurations
//! needed, these are:
//! * A [`InstanceConfig`](`config::InstanceConfig`) that describes the device
//! * A [`TimePropertiesDS`](`config::TimePropertiesDS`) that describes the
//!   timescale that is used
//! * A [`PortConfig`](`config::PortConfig`) per [`Port`](`port::Port`) to
//!   configure its behavior
//!
//! The [`PtpInstance`] can then be created with [`PtpInstance::new`], providing
//! [`InstanceConfig`](`config::InstanceConfig`) and
//! [`TimePropertiesDS`](`config::TimePropertiesDS`). From that instance
//! [`Port`](`port::Port`)s can be created using [`PtpInstance::add_port`]
//! proviging it [`PortConfig`](`config::PortConfig`), its [`Clock`] and a
//! [`Filter`](`filters::Filter`).
//!
//! ## Running
//! The [`PtpInstance`] expects to execute the BMCA periodically. For this the
//! user must provide a slice containing all ports in the
//! [`InBmca`](`port::InBmca`) state.
//!
//! [`Port`](`port::Port`)s start out in the [`InBmca`](`port::InBmca`) state
//! and can be turned into [`Running`](`port::Running`) mode by calling
//! [`Port::end_bmca`](`port::Port::end_bmca`). And for running the BMCA
//! [`Port::start_bmca`](`port::Port::start_bmca`) turns it back into the
//! [`InBmca`](`port::InBmca`) state.
//!
//! While [`Running`](`port::Running`) a [`Port`](`port::Port`) expects the user
//! to keep track of a few different timers as well as two network sockets. The
//! [`Port`](`port::Port`) is informed about any events via one of the
//! [`Port::handle_*`](`port::Port::handle_send_timestamp`) methods. Actions the
//! [`Port`](`port::Port`) expects to be performed are returned in the form of
//! [`PortAction`](`port::PortAction`)s.
//!
//! # Testing a new implementation
//! A basic option for testing is to run `statime-linux` on your developer
//! machine and connecting your new implementation to a dedicated network port.
//! Now both the time synchronization can be observed e.g. by using a
//! pulse-per-second (PPS) pin. Additionally the protocol excahnge can be
//! observed with a tool like [Wireshark](https://www.wireshark.org/).
//!
//! # Cargo Features
//! This crate exposes two features `std` and `fuzz`. `std` enables a dependency
//! on the Rust standard library providing:
//! * [`std::error::Error`] implementations for error types
//! * Implementations of the [`config::AcceptableMasterList`] trait on types in
//!   [`std`]
//! * Usage of methods on [`f32`] and [`f64`] directly from [`std`] instead of
//!   [`libm`]
//!
//! The `fuzz` feature exposes internal types for fuzzing implementations in the
//! `statime::fuzz` module.

#![no_std]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(rustdoc::unescaped_backticks)]
#[cfg(feature = "std")]
extern crate std;

mod bmc;
mod clock;
pub mod config;
pub(crate) mod datastructures;
pub mod filters;
mod float_polyfill;
pub mod observability;
mod overlay_clock;
pub mod port;
mod ptp_instance;
#[cfg(feature = "std")]
mod shared_clock;
pub mod time;

pub use clock::Clock;
pub use overlay_clock::OverlayClock;
pub use ptp_instance::{PtpInstance, PtpInstanceState, PtpInstanceStateMutex};
#[cfg(feature = "std")]
pub use shared_clock::SharedClock;

/// Helper types used for fuzzing
///
/// Enabled by the `fuzz` `feature`
#[cfg(feature = "fuzz")]
pub mod fuzz {
    pub use crate::datastructures::messages::FuzzMessage;
}
