//! Implementation of [BasicFilter]

use super::Filter;
use crate::{port::Measurement, time::Duration};
use fixed::traits::LossyInto;

#[derive(Debug)]
struct PrevStepData {
    measurement: Measurement,
    correction: Duration,
}

/// A basic filter implementation that should work in most circumstances
#[derive(Debug)]
pub struct BasicFilter {
    last_step: Option<PrevStepData>,

    offset_confidence: Duration,
    freq_confidence: f64,

    gain: f64,
}

impl BasicFilter {
    pub fn new(gain: f64) -> Self {
        Self {
            last_step: None,
            offset_confidence: Duration::from_nanos(1_000_000_000),
            freq_confidence: 1e-4,
            gain,
        }
    }
}

impl Filter for BasicFilter {
    fn absorb(&mut self, measurement: Measurement) -> (Duration, f64) {
        // Reset on too-large difference
        if measurement.master_offset.abs() > Duration::from_nanos(1_000_000_000) {
            log::debug!("Offset too large, stepping {}", measurement.master_offset);
            self.offset_confidence = Duration::from_nanos(1_000_000_000);
            self.freq_confidence = 1e-4;
            return (-measurement.master_offset, 1.0);
        }

        // Determine offset
        let mut offset = measurement.master_offset;
        if offset.abs() > self.offset_confidence {
            offset = offset.clamp(-self.offset_confidence, self.offset_confidence);
            self.offset_confidence *= 2i32;
        } else {
            self.offset_confidence -= (self.offset_confidence - offset.abs()) * self.gain;
        }

        // And decide it's correction
        let correction = -offset * self.gain;

        let freq_corr = if let Some(last_step) = &self.last_step {
            // Calculate interval for us
            let interval_local: f64 =
                (measurement.event_time - last_step.measurement.event_time - last_step.correction)
                    .nanos()
                    .lossy_into();
            // and for the master
            let interval_master: f64 = ((measurement.event_time - measurement.master_offset)
                - (last_step.measurement.event_time - last_step.measurement.master_offset))
                .nanos()
                .lossy_into();

            // get relative frequency difference
            let mut freq_diff = interval_local / interval_master;
            if libm::fabs(freq_diff - 1.0) > self.freq_confidence {
                freq_diff = freq_diff.clamp(1.0 - self.freq_confidence, 1.0 + self.freq_confidence);
                self.freq_confidence *= 2.0;
            } else {
                self.freq_confidence -=
                    self.freq_confidence - libm::fabs(freq_diff - 1.0) * self.gain;
            }

            // and decide the correction
            1.0 + (freq_diff - 1.0) * self.gain * 0.1
        } else {
            // No data, so no correction
            1.0
        };

        log::info!(
            "Offset to master: {}, corrected with phase change {} and freq change {}",
            measurement.master_offset,
            correction,
            freq_corr
        );

        // Store data for next time
        self.last_step = Some(PrevStepData {
            measurement,
            correction,
        });

        (correction, freq_corr)
    }
}
