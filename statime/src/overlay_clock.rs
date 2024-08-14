//! Virtual (overlay) clock and utilities related to it

use crate::{time::Time, time::Duration, Clock};

/// Clock overlay values
#[derive(Debug, Clone)]
pub struct ClockOverlay {
    /// underlying clock's timestamp of the last synchronization
    pub last_sync: Time,
    /// value to add to the OS clock's timestamp to obtain the virtual clock's timestamp
    pub shift: Duration,
    /// frequency correction factor, positive numbers accelerate the virtual clock relative to OS clock, negative make is slower
    pub freq_scale: f64,
}

/// Trait used for exporting the changes of the overlay from overlay clock to a third-party
pub trait ClockOverlayExporter {
    /// Called when overlay is updated.
    /// It doesn't make sense to export clock from more than 1 ClockOverlay, hence this method is mutable
    fn export(&mut self, overlay: &ClockOverlay);
}

/// Dummy exporter, does not forward clock overlay changes anywhere
#[derive(Debug)]
pub struct DoNotExport {
}
impl ClockOverlayExporter for DoNotExport {
    fn export(&mut self, _: &ClockOverlay) {
        // no-op
    }
}

#[cfg(feature = "std")]
/// Calls specified callback when overlay changes
pub struct CallbackExporter (std::boxed::Box<dyn FnMut(&ClockOverlay) + Send>);

#[cfg(feature = "std")]
impl ClockOverlayExporter for CallbackExporter {
    fn export(&mut self, overlay: &ClockOverlay) {
        (self.0)(overlay);
    }
}

#[cfg(feature = "std")]
impl core::fmt::Debug for CallbackExporter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CallbackExporter")
    }
}

#[cfg(feature = "std")]
impl<F: FnMut(&ClockOverlay) + Send + 'static> From<F> for CallbackExporter {
    fn from(cb: F) -> Self {
        Self(std::boxed::Box::new(cb))
    }
}

/* #[cfg(feature = "std")]
impl From<std::boxed::Box<dyn FnMut(&ClockOverlay)>> for CallbackExporter {
    fn from(cb: std::boxed::Box<dyn FnMut(&ClockOverlay)>) -> Self {
        Self(cb)
    }
} */

impl<T: ClockOverlayExporter> ClockOverlayExporter for Option<T> {
    fn export(&mut self, overlay: &ClockOverlay) {
        if let Some(v) = self {
            v.export(overlay);
        }
    }
}


/// An overlay over other, read-only clock, frequency-locked to it.
/// In other words, a virtual clock which can be tuned in software without affecting
/// the underlying system or hardware clock.
#[derive(Debug)]
pub struct OverlayClock<C: Clock, E: ClockOverlayExporter + core::fmt::Debug> {
    roclock: C,
    overlay: ClockOverlay,
    exporter: E,
}

impl<C: Clock, E: ClockOverlayExporter + core::fmt::Debug> OverlayClock<C, E> {
    /// Creates new OverlayClock based on given clock. Specify `DoNotExport` as `exporter` if you don't need to export the overlay.
    pub fn new(underlying_clock: C, exporter: E) -> Self {
        let now = underlying_clock.now();
        Self {
            roclock: underlying_clock,
            overlay: ClockOverlay {
                last_sync: now,
                shift: Duration::from_fixed_nanos(0),
                freq_scale: 0.0,
            },
            exporter
        }
    }

    /// Converts (shifts and scales) `Time` in underlying clock's timescale to overlay clock timescale
    pub fn time_from_underlying(&self, roclock_time: Time) -> Time {
        let elapsed = roclock_time - self.overlay.last_sync;
        let corr = elapsed * self.overlay.freq_scale;

        roclock_time + self.overlay.shift + corr
        // equals self.last_sync + self.shift + elapsed + corr
    }

    /// Returns reference to underlying clock
    pub fn underlying(&self) -> &C {
        &self.roclock
    }

    fn export(&mut self) {
        self.exporter.export(&self.overlay);
    }
}

impl<C: Clock, E: ClockOverlayExporter + core::fmt::Debug> Clock for OverlayClock<C, E> {
    type Error = C::Error;
    fn now(&self) -> Time {
        self.time_from_underlying(self.roclock.now())
    }
    fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
        // save current shift:
        let now_roclock = self.roclock.now();
        let now_local = self.time_from_underlying(now_roclock);

        self.overlay = ClockOverlay {
            last_sync: now_roclock,
            shift: now_local - now_roclock,
            freq_scale: ppm / 1_000_000f64
        };
        debug_assert_eq!(self.time_from_underlying(self.overlay.last_sync), now_local);

        self.export();
        Ok(now_local)
    }
    fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
        self.overlay.last_sync = self.roclock.now();
        self.overlay.shift += offset;

        self.export();
        Ok(self.time_from_underlying(self.overlay.last_sync))
    }
    fn set_properties(&mut self, _time_properties_ds: &crate::config::TimePropertiesDS) -> Result<(), Self::Error> {
        // we can ignore the properies - they are just metadata
        Ok(())
    }
}
