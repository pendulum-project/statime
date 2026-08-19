//! A compact Kalman clock servo for systems with a characterized oscillator.
//!
//! Unlike [`super::KalmanFilter`], this filter uses one estimator and a fixed,
//! configured oscillator-wander model. It still estimates network measurement
//! noise online. This removes the second estimator and its convergence state,
//! but makes the quality of `frequency_wander` part of the application tuning.
//!
//! The [noise-estimation design used by Statime][paper] needs its second,
//! temporarily open-loop estimator specifically because network noise obscures
//! oscillator wander at short intervals. Omitting that estimator is justified
//! when oscillator wander has instead been characterized or conservatively
//! bounded for the target and environment; it is not a generally equivalent
//! replacement for online wander estimation.
//!
//! [paper]: https://tweedegolf.nl/images/estimating-noise-for-clock-synchronizing-kalman-filters-copyright.pdf

use super::{kalman::InnerFilter, Filter, FilterEstimate, FilterUpdate};
use crate::{
    port::Measurement,
    time::{Duration, Time},
    Clock,
};

const ERROR_SAMPLES: usize = 32;

fn sqr(value: f64) -> f64 {
    value * value
}

/// Configuration for [`FixedWanderKalmanFilter`].
///
/// The defaults are a reference preset for an embedded ordinary clock with an
/// uncompensated crystal oscillator, hardware packet timestamps, and one-Hz
/// Sync. They deliberately favor conservative startup over trusting the first
/// few network observations. Applications with oscillator characterization
/// should override `frequency_wander`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedWanderKalmanConfig {
    /// Offset above which the clock is stepped instead of slewed.
    pub step_threshold: Duration,
    /// Time over which an estimated offset is removed by frequency steering.
    pub steer_time: Duration,
    /// Maximum phase-removal frequency correction, in ppm.
    pub max_steer: f64,
    /// Maximum total clock frequency correction, in ppm.
    pub max_frequency: f64,
    /// Initial one-sigma fractional-frequency uncertainty.
    ///
    /// The 100 ppm default covers the initial tolerance of common XOs without
    /// immediately saturating the default clock-actuator range.
    pub initial_frequency_uncertainty: f64,
    /// Initial one-sigma timestamp-measurement uncertainty.
    ///
    /// This conservative value is used until four closely paired Sync and
    /// DelayReq observations allow measurement noise to be estimated online.
    pub initial_measurement_uncertainty: Duration,
    /// Fractional-frequency random-walk variance per second.
    ///
    /// This is the `A` term in the process covariance. In the cited paper, an
    /// Intel I210 oscillator measured independently and estimated online under
    /// good network conditions both gave approximately `6.25e-18`. The default
    /// is the next, four-times-larger estimator bin as a conservative reference;
    /// it is not a substitute for target and environment characterization.
    pub frequency_wander: f64,
    /// Relative path-delay random-walk variance per second.
    ///
    /// The default grows an initially exact delay estimate to one-percent
    /// standard uncertainty after one hour without further observations.
    pub delay_wander: f64,
}

impl Default for FixedWanderKalmanConfig {
    fn default() -> Self {
        Self {
            step_threshold: Duration::from_seconds(1e-3),
            steer_time: Duration::from_seconds(2.0),
            max_steer: 200.0,
            max_frequency: 400.0,
            initial_frequency_uncertainty: 100e-6,
            initial_measurement_uncertainty: Duration::from_seconds(1e-3),
            frequency_wander: 2.5e-17,
            delay_wander: 1e-4 / 3600.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct MeasurementNoise {
    data: [f64; ERROR_SAMPLES],
    next: usize,
    len: usize,
    last_sync: Option<(Time, Duration)>,
    last_delay: Option<(Time, Duration)>,
    peer_delay: bool,
}

impl MeasurementNoise {
    const RANGE_SAMPLES: usize = 4;
    const VARIANCE_SAMPLES: usize = 8;

    fn observe(&mut self, measurement: Measurement, frequency: f64) {
        if let Some(sync) = measurement.raw_sync_offset {
            if let Some((time, delay)) = self.last_delay.take() {
                if (measurement.event_time - time).abs() < Duration::from_millis(200) {
                    self.push(
                        sync.seconds() - delay.seconds()
                            + (time - measurement.event_time).seconds() * frequency,
                    );
                } else {
                    self.last_sync = Some((measurement.event_time, sync));
                }
            } else {
                self.last_sync = Some((measurement.event_time, sync));
            }
        }

        if let Some(delay) = measurement.raw_delay_offset {
            if let Some((time, sync)) = self.last_sync.take() {
                if (measurement.event_time - time).abs() < Duration::from_millis(200) {
                    self.push(
                        sync.seconds() - delay.seconds()
                            + (measurement.event_time - time).seconds() * frequency,
                    );
                } else {
                    self.last_delay = Some((measurement.event_time, delay));
                }
            } else {
                self.last_delay = Some((measurement.event_time, delay));
            }
        }

        if let Some(delay) = measurement.peer_delay {
            self.last_sync = None;
            self.last_delay = None;
            self.peer_delay = true;
            self.push(delay.seconds());
        }
    }

    fn push(&mut self, value: f64) {
        self.data[self.next] = value;
        self.next = (self.next + 1) % self.data.len();
        self.len = (self.len + 1).min(self.data.len());
    }

    fn variance(&self, config: &FixedWanderKalmanConfig) -> f64 {
        if self.len < Self::RANGE_SAMPLES {
            sqr(config.initial_measurement_uncertainty.seconds())
        } else if self.len < Self::VARIANCE_SAMPLES {
            let values = &self.data[..self.len];
            let (min, max) = values
                .iter()
                .copied()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
                    (min.min(value), max.max(value))
                });
            sqr(max - min)
        } else {
            let values = &self.data[..self.len];
            let mean = values.iter().sum::<f64>() / self.len as f64;
            // Sync-minus-DelayReq contains two independent one-way errors, so
            // its sample variance is twice either observation's variance.
            values.iter().map(|value| sqr(value - mean)).sum::<f64>()
                / (2.0 * (self.len - 1) as f64)
        }
    }
}

/// Three-state Kalman clock servo with fixed oscillator-wander covariance.
///
/// The filter estimates local-minus-master phase, residual fractional
/// frequency, and mean path delay. It uses one estimator and specialized scalar
/// algebra, making it smaller than [`super::KalmanFilter`], which runs a second
/// estimator to learn oscillator wander.
///
/// Use this filter when the oscillator and its operating environment are known
/// well enough to configure [`FixedWanderKalmanConfig::frequency_wander`], or
/// when deterministic memory, code size, and startup behavior matter more than
/// adapting across unknown hardware. A poor wander value can make the filter
/// either sluggish and overconfident (too small) or noisy (too large); prefer
/// [`super::KalmanFilter`] for general-purpose systems without that knowledge.
pub struct FixedWanderKalmanFilter {
    config: FixedWanderKalmanConfig,
    estimate: Option<InnerFilter>,
    noise: MeasurementNoise,
    frequency: Option<f64>,
}

impl Filter for FixedWanderKalmanFilter {
    type Config = FixedWanderKalmanConfig;

    fn new(config: Self::Config) -> Self {
        Self {
            config,
            estimate: None,
            noise: MeasurementNoise::default(),
            frequency: None,
        }
    }

    fn measurement<C: Clock>(&mut self, measurement: Measurement, clock: &mut C) -> FilterUpdate {
        if let Some(estimate) = self.estimate.as_ref() {
            if measurement.event_time < estimate.time() {
                return FilterUpdate::default();
            }
        }

        self.noise.observe(
            measurement,
            self.estimate.as_ref().map_or(0.0, InnerFilter::frequency),
        );
        let variance =
            self.noise.variance(&self.config) * if self.noise.peer_delay { 2.0 } else { 1.0 };

        if measurement.raw_sync_offset.is_some() || measurement.raw_delay_offset.is_some() {
            self.ensure_frequency(clock);
        }

        let estimate = self.estimate.get_or_insert_with(|| {
            InnerFilter::new(
                0.0,
                measurement.event_time,
                self.config.step_threshold,
                self.config.initial_frequency_uncertainty,
            )
        });
        estimate.progress_filtertime(
            measurement.event_time,
            self.config.frequency_wander,
            self.config.delay_wander,
        );
        if let Some(value) = measurement.raw_sync_offset {
            let value = value.seconds();
            if (value - estimate.offset()).abs() > self.config.step_threshold.seconds() {
                *estimate = InnerFilter::new(
                    value,
                    estimate.time(),
                    self.config.step_threshold,
                    self.config.initial_frequency_uncertainty,
                );
            } else {
                estimate.absorb_sync_offset(value, variance);
            }
        }
        if let Some(value) = measurement.raw_delay_offset {
            let value = value.seconds();
            if (value - estimate.offset()).abs() > self.config.step_threshold.seconds() {
                *estimate = InnerFilter::new(
                    value,
                    estimate.time(),
                    self.config.step_threshold,
                    self.config.initial_frequency_uncertainty,
                );
            } else {
                estimate.absorb_delay_offset(value, variance);
            }
        }
        if let Some(value) = measurement.peer_delay {
            estimate.absorb_peer_delay(value.seconds(), variance);
        }

        self.steer(clock)
    }

    fn update<C: Clock>(&mut self, clock: &mut C) -> FilterUpdate {
        self.change_frequency(0.0, clock);
        FilterUpdate {
            next_update: None,
            mean_delay: self.mean_delay(),
        }
    }

    fn demobilize<C: Clock>(mut self, clock: &mut C) {
        self.change_frequency(0.0, clock);
    }

    fn current_estimates(&self) -> FilterEstimate {
        FilterEstimate {
            offset_from_master: Duration::from_seconds(
                self.estimate.as_ref().map_or(0.0, InnerFilter::offset),
            ),
            mean_delay: self.mean_delay().unwrap_or(Duration::ZERO),
        }
    }
}

impl FixedWanderKalmanFilter {
    fn ensure_frequency<C: Clock>(&mut self, clock: &mut C) {
        if self.frequency.is_none() && clock.set_frequency(0.0).is_ok() {
            self.frequency = Some(0.0);
        }
    }

    fn change_frequency<C: Clock>(&mut self, target: f64, clock: &mut C) {
        let (Some(current), Some(estimate)) = (self.frequency, self.estimate.as_mut()) else {
            return;
        };
        let requested = target - estimate.frequency() * 1e6;
        let next =
            (current + requested).clamp(-self.config.max_frequency, self.config.max_frequency);
        let applied = next - current;
        if let Ok(time) = clock.set_frequency(next) {
            self.frequency = Some(next);
            estimate.absorb_frequency_steer(
                applied,
                time,
                self.config.frequency_wander,
                self.config.delay_wander,
            );
        }
    }

    fn steer<C: Clock>(&mut self, clock: &mut C) -> FilterUpdate {
        let Some(estimate) = self.estimate.as_ref() else {
            return FilterUpdate::default();
        };
        let offset = estimate.offset();
        if offset.abs() < self.config.step_threshold.seconds() {
            let target = (-offset * 1e6 / self.config.steer_time.seconds())
                .clamp(-self.config.max_steer, self.config.max_steer);
            self.change_frequency(target, clock);
            FilterUpdate {
                next_update: Some(core::time::Duration::from_secs_f64(
                    self.config.steer_time.seconds(),
                )),
                mean_delay: self.mean_delay(),
            }
        } else {
            if clock.step_clock(Duration::from_seconds(-offset)).is_ok() {
                if let Some(estimate) = self.estimate.as_mut() {
                    estimate.absorb_offset_steer(-offset);
                }
            }
            FilterUpdate {
                next_update: None,
                mean_delay: self.mean_delay(),
            }
        }
    }

    fn mean_delay(&self) -> Option<Duration> {
        self.estimate
            .as_ref()
            .map(|estimate| Duration::from_seconds(estimate.delay()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TimePropertiesDS;

    #[derive(Debug)]
    struct TestError;

    struct TestClock {
        time: Time,
        frequency: f64,
        last_step: Option<Duration>,
        fail_frequency: bool,
    }

    impl Clock for TestClock {
        type Error = TestError;

        fn now(&self) -> Time {
            self.time
        }

        fn step_clock(&mut self, offset: Duration) -> Result<Time, Self::Error> {
            self.last_step = Some(offset);
            self.time += offset;
            Ok(self.time)
        }

        fn set_frequency(&mut self, ppm: f64) -> Result<Time, Self::Error> {
            if self.fail_frequency {
                return Err(TestError);
            }
            self.frequency = ppm;
            Ok(self.time)
        }

        fn set_properties(&mut self, _: &TimePropertiesDS) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    fn test_clock(time: Time) -> TestClock {
        TestClock {
            time,
            frequency: 0.0,
            last_step: None,
            fail_frequency: false,
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-12 * expected.abs().max(1.0));
    }

    #[test]
    fn measurement_noise_uses_startup_range_then_ring_variance() {
        let config = FixedWanderKalmanConfig {
            initial_measurement_uncertainty: Duration::from_seconds(0.5),
            ..Default::default()
        };
        let mut noise = MeasurementNoise::default();

        for value in 0..3 {
            noise.push(value as f64);
        }
        assert_close(noise.variance(&config), 0.25);

        noise.push(3.0);
        assert_close(noise.variance(&config), 9.0);

        for value in 4..8 {
            noise.push(value as f64);
        }
        assert_close(noise.variance(&config), 3.0);

        for value in 8..33 {
            noise.push(value as f64);
        }
        // The ring now contains 1..=32. Their sample variance is 88, and
        // Sync-minus-DelayReq variance is twice the one-way variance.
        assert_close(noise.variance(&config), 44.0);
    }

    #[test]
    fn measurement_noise_pairs_sync_and_delay_with_frequency_correction() {
        let mut noise = MeasurementNoise::default();
        noise.observe(
            Measurement {
                event_time: Time::from_nanos(1_000_000_000),
                raw_sync_offset: Some(Duration::from_nanos(10)),
                ..Measurement::default()
            },
            1e-6,
        );
        noise.observe(
            Measurement {
                event_time: Time::from_nanos(1_100_000_000),
                raw_delay_offset: Some(Duration::from_nanos(2)),
                ..Measurement::default()
            },
            1e-6,
        );

        assert_eq!(noise.len, 1);
        assert_close(noise.data[0], 108e-9);
    }

    #[test]
    fn positive_local_phase_error_commands_negative_frequency() {
        let time = Time::from_nanos(1_000_000_000);
        let mut clock = test_clock(time);
        let mut filter = FixedWanderKalmanFilter::new(FixedWanderKalmanConfig::default());
        filter.measurement(
            Measurement {
                event_time: time,
                raw_sync_offset: Some(Duration::from_nanos(100)),
                ..Measurement::default()
            },
            &mut clock,
        );
        assert!(clock.frequency < 0.0);
    }

    #[test]
    fn frequency_command_respects_actuator_limit() {
        let time = Time::from_nanos(1_000_000_000);
        let mut clock = test_clock(time);
        let mut filter = FixedWanderKalmanFilter::new(FixedWanderKalmanConfig {
            max_frequency: 5.0,
            ..Default::default()
        });

        filter.measurement(
            Measurement {
                event_time: time,
                raw_sync_offset: Some(Duration::from_micros(100)),
                ..Measurement::default()
            },
            &mut clock,
        );

        assert_eq!(clock.frequency, -5.0);
    }

    #[test]
    fn large_phase_error_steps_the_clock() {
        let time = Time::from_nanos(1_000_000_000);
        let mut clock = test_clock(time);
        let mut filter = FixedWanderKalmanFilter::new(FixedWanderKalmanConfig::default());

        filter.measurement(
            Measurement {
                event_time: time,
                raw_sync_offset: Some(Duration::from_millis(2)),
                ..Measurement::default()
            },
            &mut clock,
        );

        assert!(
            (clock.last_step.unwrap() - Duration::from_millis(-2)).abs() < Duration::from_nanos(1)
        );
        assert!(filter.current_estimates().offset_from_master.abs() < Duration::from_nanos(1));
    }

    #[test]
    fn failed_frequency_initialization_is_not_assumed_applied() {
        let time = Time::from_nanos(1_000_000_000);
        let mut clock = test_clock(time);
        clock.fail_frequency = true;
        let mut filter = FixedWanderKalmanFilter::new(FixedWanderKalmanConfig::default());

        filter.measurement(
            Measurement {
                event_time: time,
                raw_sync_offset: Some(Duration::from_nanos(100)),
                ..Measurement::default()
            },
            &mut clock,
        );

        assert_eq!(filter.frequency, None);
        assert_eq!(clock.frequency, 0.0);
    }
}
