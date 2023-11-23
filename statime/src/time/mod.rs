//! Types that describe points in time ([`Time`]), and durations between two
//! instants ([`Duration`], [`Interval`])
//!
//! These are used throughout `statime` instead of types from [`std::time`] as
//! they fit closer with the on the wire representation of time in PTP.

mod duration;
mod instant;
mod interval;

pub use duration::Duration;
pub use instant::Time;
pub use interval::Interval;
