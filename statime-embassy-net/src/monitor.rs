use core::sync::atomic::{AtomicU8, Ordering};

/// Quality of the PHC's relation to its selected PTP master.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ClockState {
    /// No successful servo update has established the clock relation yet.
    Unavailable,
    /// The servo is receiving measurements and disciplining the PHC.
    Tracking,
    /// Measurements stopped; the PHC retains its last applied rate.
    Holdover,
}

/// Lock-free observation of state that cannot be reconstructed from PTP time.
pub struct PtpMonitor {
    state: AtomicU8,
}

impl PtpMonitor {
    const STATE_MASK: u8 = 0x3;
    const PTP_TIMESCALE: u8 = 0x4;

    /// Create an unavailable monitor with no engine history.
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(ClockState::Unavailable as u8),
        }
    }

    /// Read the current TAI clock-relation state.
    pub fn state(&self) -> ClockState {
        let state = self.state.load(Ordering::Relaxed);
        if state & Self::PTP_TIMESCALE == 0 {
            ClockState::Unavailable
        } else {
            match state & Self::STATE_MASK {
                1 => ClockState::Tracking,
                2 => ClockState::Holdover,
                _ => ClockState::Unavailable,
            }
        }
    }

    pub(crate) fn tracking(&self) {
        let state = self.state.load(Ordering::Relaxed);
        if state & Self::PTP_TIMESCALE != 0 {
            self.state.store(
                Self::PTP_TIMESCALE | ClockState::Tracking as u8,
                Ordering::Relaxed,
            );
        }
    }

    pub(crate) fn holdover(&self) {
        let state = self.state.load(Ordering::Relaxed);
        if state & Self::STATE_MASK == ClockState::Tracking as u8 {
            self.state.store(
                state & !Self::STATE_MASK | ClockState::Holdover as u8,
                Ordering::Relaxed,
            );
        }
    }

    pub(crate) fn unavailable(&self) {
        let state = self.state.load(Ordering::Relaxed);
        self.state
            .store(state & !Self::STATE_MASK, Ordering::Relaxed);
    }

    pub(crate) fn set_ptp_timescale(&self, enabled: bool) {
        let state = self.state.load(Ordering::Relaxed);
        self.state.store(
            if enabled {
                state | Self::PTP_TIMESCALE
            } else {
                ClockState::Unavailable as u8
            },
            Ordering::Relaxed,
        );
    }
}

impl Default for PtpMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_requires_ptp_timescale_and_continuous_update() {
        let monitor = PtpMonitor::new();
        monitor.set_ptp_timescale(true);
        assert_eq!(monitor.state(), ClockState::Unavailable);
        monitor.tracking();
        assert_eq!(monitor.state(), ClockState::Tracking);
        monitor.holdover();
        assert_eq!(monitor.state(), ClockState::Holdover);
        monitor.unavailable();
        assert_eq!(monitor.state(), ClockState::Unavailable);
        monitor.tracking();
        assert_eq!(monitor.state(), ClockState::Tracking);
        monitor.set_ptp_timescale(false);
        assert_eq!(monitor.state(), ClockState::Unavailable);
    }
}
