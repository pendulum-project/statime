//! Serializable implementations of datastructures to be used for observability
/// A concrete implementation of the PTP Current dataset (IEEE1588-2019 section
/// 8.2.2)
pub mod current;
/// A concrete implementation of the PTP Default dataset (IEEE1588-2019 section
/// 8.2.1)
pub mod default;
/// A concrete implementation of the PTP Parent dataset (IEEE1588-2019 section
/// 8.2.3)
pub mod parent;
/// A concrete implementation of the PTP Port dataset (IEEE1588-2019 section
/// 8.2.15)
pub mod port;

pub use crate::datastructures::datasets::PathTraceDS;
