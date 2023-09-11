//! examples/smallest.rs

#![no_main]
#![no_std]
#![deny(warnings)]
#![deny(unsafe_code)]
#![deny(missing_docs)]

use panic_probe as _; // panic handler
use defmt_rtt as _; // global logger
use rtic::app;

#[app(device = stm32f7xx_hal::pac)]
mod app {
    #[shared]
    struct Shared {}

    #[local]
    struct Local {}

    #[init]
    fn init(_: init::Context) -> (Shared, Local) {
        defmt::println!("hoi");
        (Shared {}, Local {})
    }
}