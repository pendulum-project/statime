//! Definitions and implementations of the abstract clock types

use crate::{
    datastructures::datasets::TimePropertiesDS,
    time::{Duration, Time},
};

/// Clock manipulation and querying interface
///
/// The clock trait is the primary way the PTP stack interfaces with the
/// system's clock. It's implementation should be provided by the user of the
/// Statime crate, and should provide information on and ways to manipulate the
/// system's clock. An implementation of this trait for linux is provided in the
/// statime-linux crate.
///
/// Note that the clock implementation is responsible for handling leap seconds.
/// On most operating systems, this will be provided for by the OS, but on some
/// platforms this may require extra logic.
pub trait Clock {
    /// Type of the error the methods of this [`Clock`] may return
    type Error: core::fmt::Debug;

    /// Get the current time of the clock
    fn now(&self) -> Time;

    /// Change the current time of the clock by offset. Returns
    /// the time at which the change was applied.
    ///
    /// The applied correction should be as close as possible to
    /// the requested correction. The reported time of the change
    /// should be as close as possible to the time the change was
    /// applied
    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error>;

    /// Set the frequency of the clock, returning the time
    /// at which the change was applied. The value is in ppm
    /// difference from the clocks base frequency.
    ///
    /// The applied correction should be as close as possible to
    /// the requested correction. The reported time of the change
    /// should be as close as possible to the time the change was
    /// applied
    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error>;

    /// Adjust the timescale properties of the clock, including
    /// things like the leap indicator, to the extend supported by the
    /// system.
    fn set_properties(&mut self, time_properties_ds: &TimePropertiesDS) -> Result<(), Self::Error>;
}

#[cfg(feature = "std")]
impl<T: Clock + ?Sized> Clock for std::boxed::Box<T> {
    type Error = T::Error;
    fn now(&self) -> Time {
        self.as_ref().now()
    }
    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
        self.as_mut().step_clock(offset)
    }
    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
        self.as_mut().set_frequency(ppm)
    }
    fn set_properties(&mut self, time_properties_ds: &TimePropertiesDS) -> Result<(), Self::Error> {
        self.as_mut().set_properties(time_properties_ds)
    }
}
