//! Re-export or add abstracted versions of `ETHERNET_MAC`, `ETHERNET_DMA`
//! and `ETHERNET_PTP` that introduce a delay for some registers on F4 parts.

pub use stm32f7::stm32f7x9::{ETHERNET_DMA, ETHERNET_MAC, ETHERNET_PTP};
