use crate::time::Duration;
use crate::time::Time;
use crate::Clock;
use std::sync::Arc;
use std::sync::Mutex;

/// A wrapper for stateful `statime::Clock` implementations to make them behave
/// like e.g. `statime_linux::LinuxClock` - clones share state with each other
#[derive(Debug)]
pub struct SharedClock<C>(pub Arc<Mutex<C>>)
where
    C: Clock;

impl<C: Clock> SharedClock<C> {
    /// Take given clock and make it a `SharedClock`
    pub fn new(clock: C) -> Self {
        Self(Arc::new(Mutex::new(clock)))
    }
}

impl<C: Clock> Clone for SharedClock<C> {
    /// Clone the shared reference to the clock (behaviour consistent with `statime_linux::LinuxClock`)
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<C: Clock> Clock for SharedClock<C> {
    type Error = C::Error;
    fn now(&self) -> Time {
        self.0.lock().unwrap().now()
    }
    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
        self.0.lock().unwrap().set_frequency(ppm)
    }
    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
        self.0.lock().unwrap().step_clock(offset)
    }
    fn set_properties(
        &mut self,
        time_properties_ds: &crate::config::TimePropertiesDS,
    ) -> Result<(), Self::Error> {
        self.0.lock().unwrap().set_properties(time_properties_ds)
    }
}
