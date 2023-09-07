use super::{
    matrix::{Matrix, Vector},
    FilterUpdate,
};
use crate::{Clock, Duration, Filter, Measurement, Time};

#[derive(Debug)]
pub struct KalmanFilter {
    inner: KalmanFilterState,
}

impl KalmanFilter {
    fn startup<C: Clock>(
        &mut self,
        config: KalmanFilterConfig,
        m: Measurement,
        delay_stats: AveragingBuffer,
        clock: &mut C,
    ) -> FilterUpdate {
        let mut offset = -m.master_offset;
        let uncertainty = delay_stats.variance();
        if offset.abs().seconds() > uncertainty.sqrt() {
            log::debug!("Startup step {:e}ns", offset.nanos_lossy());
            if let Err(error) = clock.step_clock(offset) {
                log::error!("Could not step clock: {:?}", error);
            }
            offset = Duration::ZERO
        }
        if let Err(error) = clock.set_frequency(0.0) {
            log::error!("Could not initialize frquency: {:?}", error);
        }
        self.inner = KalmanFilterState::Running(InnerKalmanFilter {
            state: Vector::new_vector([offset.seconds(), 0.0]),
            uncertainty: Matrix::new([
                [uncertainty, 0.0],
                [0.0, config.initial_frequency_uncertainty],
            ]),
            clock_wander: config.initial_wander,
            delay_stats,
            precision_score: 0,
            last_measurement: m,
            filter_time: m.event_time,
            desired_freq: 0.0,
            cur_freq: 0.0,
            config,
        });
        FilterUpdate::default()
    }
}

impl Filter for KalmanFilter {
    type Config = KalmanFilterConfig;

    fn new(config: Self::Config) -> Self {
        Self {
            inner: KalmanFilterState::Initial(config, None),
        }
    }

    fn measurement<C: Clock>(&mut self, m: Measurement, clock: &mut C) -> FilterUpdate {
        match self.inner {
            KalmanFilterState::Initial(config, None) => {
                debug_assert!(false, "Should not be possible");
                self.startup(config, m, AveragingBuffer::default(), clock)
            }
            KalmanFilterState::Initial(config, Some(cached_delay)) => {
                let mut delay_stats = AveragingBuffer::default();
                delay_stats.update(cached_delay.seconds());
                self.startup(config, m, delay_stats, clock)
            }
            KalmanFilterState::Running(ref mut state) => state.handle_measurement(m, clock),
        }
    }

    fn delay(&mut self, delay: Duration) -> Duration {
        log::info!("Delay: {}ns", delay.seconds()*1e9);
        match self.inner {
            KalmanFilterState::Initial(_, ref mut cached_delay) => {
                *cached_delay = Some(delay);
                delay
            }
            KalmanFilterState::Running(ref mut state) => state.handle_delay(delay),
        }
    }

    fn update<C: Clock>(&mut self, clock: &mut C) -> super::FilterUpdate {
        match self.inner {
            KalmanFilterState::Initial(_, _) => FilterUpdate::default(),
            KalmanFilterState::Running(ref mut state) => state.handle_update(clock),
        }
    }

    fn demobilize<C: crate::Clock>(self, clock: &mut C) {
        match self.inner {
            KalmanFilterState::Initial(_, _) => {}
            KalmanFilterState::Running(state) => state.demobilize(clock),
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum KalmanFilterState {
    Initial(KalmanFilterConfig, Option<Duration>),
    Running(InnerKalmanFilter),
}

#[derive(Debug)]
struct InnerKalmanFilter {
    state: Vector<2>,
    uncertainty: Matrix<2, 2>,
    clock_wander: f64,

    delay_stats: AveragingBuffer,

    precision_score: i32,

    last_measurement: Measurement,
    filter_time: Time,

    desired_freq: f64,
    cur_freq: f64,

    config: KalmanFilterConfig,
}

impl InnerKalmanFilter {
    /// Move the filter forward to reflect the situation at a new, later
    /// timestamp
    fn progress_filtertime(&mut self, time: Time) {
        debug_assert!(
            time > self.filter_time,
            "time {time:?} is before filter_time {:?}",
            self.filter_time
        );
        if time < self.filter_time {
            return;
        }

        // Time step paremeters
        let delta_t = (time - self.filter_time).seconds();
        let update = Matrix::new([[1.0, delta_t], [0.0, 1.0]]);
        let process_noise = Matrix::new([
            [
                self.clock_wander * delta_t * delta_t * delta_t / 3.,
                self.clock_wander * delta_t * delta_t / 2.,
            ],
            [
                self.clock_wander * delta_t * delta_t / 2.,
                self.clock_wander * delta_t,
            ],
        ]);

        // Kalman filter update
        self.state = update * self.state;
        self.uncertainty = update * self.uncertainty * update.transpose() + process_noise;
        self.filter_time = time;

        log::trace!("Filter progressed: {:?}", time);
    }

    /// Absorb knowledge from a measurement
    fn absorb_measurement(&mut self, measurement: Measurement) -> (f64, f64) {
        // Measurement parameters
        let delay_variance = self.delay_stats.variance();

        log::info!(
            "Measurement: {:.3}±{:.3}",
            -measurement.master_offset.seconds() * 1e9,
            delay_variance.sqrt() * 1e9
        );

        // Kalman filter update
        let measurement_vec = Vector::new_vector([-measurement.master_offset.seconds()]);
        let measurement_transform = Matrix::new([[1., 0.]]);
        let measurement_noise = Matrix::new([[delay_variance / 4.]]);
        let difference = measurement_vec - measurement_transform * self.state;
        let difference_covariance =
            measurement_transform * self.uncertainty * measurement_transform.transpose()
                + measurement_noise;
        let update_strength =
            self.uncertainty * measurement_transform.transpose() * difference_covariance.inverse();
        self.state = self.state + update_strength * difference;
        self.uncertainty = ((Matrix::unit() - update_strength * measurement_transform)
            * self.uncertainty)
            .symmetrize();

        // Statistics
        let p = chi_1(difference.inner(difference_covariance.inverse() * difference));
        // Calculate an indicator of how much of the measurement was incorporated
        // into the state. 1.0 - is needed here as this should become lower as
        // measurement noise's contribution to difference uncertainty increases.
        let weight = 1.0 - measurement_noise.determinant() / difference_covariance.determinant();

        self.last_measurement = measurement;

        log::trace!("Measurement absorbed, p: {}, weight: {}", p, weight);

        (p, weight)
    }

    // Our estimate for the clock stability might be completely wrong. The code here
    // correlates the estimation for errors to what we actually observe, so we can
    // update our estimate should it turn out to be significantly off.
    fn update_wander_estimate(&mut self, p: f64, weight: f64) {
        // Note that chi is exponentially distributed with mean 2
        // Also, we do not steer towards a smaller precision estimate when measurement
        // noise dominates.
        if 1. - p < self.config.precision_low_probability
            && weight > self.config.precision_minimum_weight
        {
            self.precision_score -= 1;
        } else if 1. - p > self.config.precision_high_probability {
            self.precision_score += 1;
        } else {
            self.precision_score -= self.precision_score.signum();
        }
        log::trace!(
            "Wander estimate update, precision_score: {}, p: {}",
            self.precision_score,
            p,
        );
        if self.precision_score <= -self.config.precision_hysteresis {
            self.clock_wander /= 4.0;
            self.precision_score = 0;
            log::debug!(
                "Decreased wander estimate, wander: {}",
                self.clock_wander.sqrt(),
            );
        } else if self.precision_score >= self.config.precision_hysteresis {
            self.clock_wander *= 4.0;
            self.precision_score = 0;
            log::debug!(
                "Increased wander estimate, wander: {}",
                self.clock_wander.sqrt(),
            );
        }
    }

    fn process_offset_steering(&mut self, steer: f64) {
        self.state = self.state - Vector::new_vector([steer, 0.0]);
        self.last_measurement.master_offset -= Duration::from_seconds(steer);
        self.last_measurement.event_time += Duration::from_seconds(steer);
        self.filter_time += Duration::from_seconds(steer);
    }

    fn process_frequency_steering(&mut self, time: Time, steer: f64) {
        self.progress_filtertime(time);
        self.state = self.state - Vector::new_vector([0.0, steer]);
        self.last_measurement.master_offset +=
            Duration::from_seconds(steer * (time - self.last_measurement.event_time).seconds());
    }

    /// Update based on a new measurement.
    fn handle_measurement<C: Clock>(
        &mut self,
        measurement: Measurement,
        clock: &mut C,
    ) -> FilterUpdate {
        // Always update the root_delay, root_dispersion, leap second status and
        // stratum, as they always represent the most accurate state.
        if measurement.event_time < self.filter_time {
            log::warn!(
                "Discarded measurement as old. This can be a sign of independent changes to the \
                 clock"
            );
            // Ignore the past
            return Default::default();
        }

        // Environment update
        self.progress_filtertime(measurement.event_time);

        let (p, weight) = self.absorb_measurement(measurement);
        self.update_wander_estimate(p, weight);

        log::debug!(
            "peer offset {:e}±{:e}ns, freq {}±{}ppm",
            self.state.ventry(0) * 1e9,
            self.uncertainty.entry(0, 0).sqrt() * 1e9,
            self.state.ventry(1) * 1e6,
            self.uncertainty.entry(1, 1).sqrt() * 1e6
        );

        let offset = self.state.ventry(0);
        let offset_uncertainty = self.uncertainty.entry(0, 0).sqrt();
        let freq_delta = self.state.ventry(1);
        let freq_uncertainty = self.uncertainty.entry(1, 1).sqrt();

        log::info!(
            "Offset: {:.3}±{:.3}ns Frequency: {:.3}±{:.3}ppm",
            offset * 1e9,
            offset_uncertainty * 1e9,
            freq_delta * 1e6,
            freq_uncertainty * 1e6
        );

        if freq_uncertainty > 1e-4 {
            // Don't steer until we have some sense of what our frequency is
            return FilterUpdate::default();
        }

        if self.desired_freq == 0.0 && offset.abs() > offset_uncertainty * self.config.steer_offset_threshold {
            self.steer_offset(
                offset - offset_uncertainty * self.config.steer_offset_leftover * offset.signum(),
                clock,
            )
        } else {
            if freq_delta.abs() > freq_uncertainty * self.config.steer_frequency_threshold {
                self.steer_frequency(
                    freq_delta - self.config.steer_frequency_leftover * freq_delta.signum(),
                    clock,
                )
            }
            FilterUpdate::default()
        }
    }

    fn handle_delay(&mut self, delay: Duration) -> Duration {
        log::debug!("Received delay {:?}", delay);
        self.delay_stats.update(delay.seconds());
        log::debug!("Variance: {:e}ns", self.delay_stats.variance().sqrt() * 1e9);
        Duration::from_seconds(self.delay_stats.mean())
    }

    fn handle_update<C: Clock>(&mut self, clock: &mut C) -> FilterUpdate {
        self.desired_freq = 0.0;
        self.steer_frequency(self.state.ventry(1), clock);
        log::debug!(
            "peer offset {:e}±{:e}ns, freq {}±{}ppm",
            self.state.ventry(0) * 1e9,
            self.uncertainty.entry(0, 0).sqrt() * 1e9,
            self.state.ventry(1) * 1e6,
            self.uncertainty.entry(1, 1).sqrt() * 1e6
        );
        FilterUpdate::default()
    }

    fn steer_offset<C: Clock>(&mut self, offset: f64, clock: &mut C) -> FilterUpdate {
        if offset > self.config.step_threshold {
            log::debug!("Stepping {:?}ns", offset * 1e9);
            self.desired_freq = 0.0;
            self.steer_frequency(self.state.ventry(1), clock);
            if let Err(error) = clock.step_clock(Duration::from_seconds(offset)) {
                log::error!("Could not step clock: {:?}", error);
            }
            self.process_offset_steering(offset);
            FilterUpdate::default()
        } else {
            log::debug!("Slewing {:?}ns", offset * 1e9);
            let freq = self
                .config
                .slew_maximum_frequency_offset
                .min(offset.abs() / self.config.slew_minimum_duration);
            let duration = core::time::Duration::from_secs_f64(offset.abs() / freq);
            self.desired_freq = -freq * offset.signum();
            self.steer_frequency(self.state.ventry(1) - self.desired_freq, clock);
            FilterUpdate {
                next_update: Some(duration),
            }
        }
    }

    fn steer_frequency<C: Clock>(&mut self, steer: f64, clock: &mut C) {
        self.cur_freq += steer;
        let time = match clock.set_frequency(self.cur_freq * 1e6) {
            Ok(v) => v,
            Err(error) => {
                log::error!("Could not adjust clock frequency: {:?}", error);
                return;
            }
        };
        self.process_frequency_steering(time, steer);
        log::info!(
            "Changed frequency, steered {}ppm, desired freq {}ppm, total adjustment: {}ppm",
            steer * 1e6,
            self.desired_freq * 1e6,
            self.cur_freq * 1e6,
        );
    }

    fn demobilize<C: Clock>(self, clock: &mut C) {
        // We don't need to do the bookkeeping as it won't be used.
        if let Err(error) = clock.set_frequency((self.cur_freq + self.state.ventry(1)) * 1e6) {
            log::error!("Could not change clock frequency: {:?}", error);
        }
    }
}

#[derive(Debug, Default, Copy, Clone)]
struct AveragingBuffer {
    data: [f64; 32],
    next_idx: usize,
    fill: usize,
}

/// Approximation of 1 - the chi-squared cdf with 1 degree of freedom
/// source: https://en.wikipedia.org/wiki/Error_function
fn chi_1(chi: f64) -> f64 {
    const P: f64 = 0.3275911;
    const A1: f64 = 0.254829592;
    const A2: f64 = -0.284496736;
    const A3: f64 = 1.421413741;
    const A4: f64 = -1.453152027;
    const A5: f64 = 1.061405429;

    let x = (chi / 2.).sqrt();
    let t = 1. / (1. + P * x);
    (A1 * t + A2 * t * t + A3 * t * t * t + A4 * t * t * t * t + A5 * t * t * t * t * t)
        * (-(x * x)).exp()
}

fn sqr<M: Copy + core::ops::Mul<M>>(v: M) -> M::Output {
    v * v
}

impl AveragingBuffer {
    fn mean(&self) -> f64 {
        self.data[..self.fill].iter().sum::<f64>() / (self.fill as f64)
    }

    fn variance(&self) -> f64 {
        if self.fill == self.data.len() {
            let mean = self.data.iter().sum::<f64>() / (self.data.len() as f64);
            self.data.iter().map(|v| sqr(v - mean)).sum::<f64>() / ((self.data.len() - 1) as f64)
        } else {
            // The 4 deals with the fact that the measurements are not entirely independent.
            4.0*sqr(*self
                .data
                .iter()
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Greater))
                .unwrap_or(&1.0))
        }
    }

    fn update(&mut self, rtt: f64) {
        self.data[self.next_idx] = rtt;
        self.next_idx = (self.next_idx + 1) % self.data.len();
        if self.fill < self.data.len() {
            self.fill += 1;
        }
    }
}

#[derive(Debug, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "kebab-case", deny_unknown_fields)
)]
pub struct KalmanFilterConfig {
    /// Probability bound below which we start moving towards decreasing
    /// our precision estimate. (probability, 0-1)
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_precision_low_probability")
    )]
    pub precision_low_probability: f64,
    /// Probability bound above which we start moving towards increasing
    /// our precision estimate. (probability, 0-1)
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_precision_high_probability")
    )]
    pub precision_high_probability: f64,
    /// Ammount of hysteresis in changeing the precision estimate. (count, 1+)
    #[cfg_attr(feature = "serde", serde(default = "default_precision_hysteresis"))]
    pub precision_hysteresis: i32,
    /// Lower bound on the ammount of effect our precision estimate
    /// has on the total noise estimate before we allow decreasing
    /// of the precision estimate. (weight, 0-1)
    #[cfg_attr(feature = "serde", serde(default = "default_precision_minimum_weight"))]
    pub precision_minimum_weight: f64,

    /// Threshold (in number of standard deviations) above which
    /// measurements with a significantly larger network delay
    /// are rejected. (standard deviations, 0+)
    #[cfg_attr(feature = "serde", serde(default = "default_delay_outlier_threshold"))]
    pub delay_outlier_threshold: f64,

    /// Initial estimate of the clock wander of the combination
    /// of our local clock and that of the peer. (s/s^2)
    #[cfg_attr(feature = "serde", serde(default = "default_initial_wander"))]
    pub initial_wander: f64,
    /// Initial uncertainty of the frequency difference between
    /// our clock and that of the peer. (s/s)
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_initial_frequency_uncertainty")
    )]
    pub initial_frequency_uncertainty: f64,

    /// Maximum peer uncertainty before we start disregarding it
    /// Note that this is combined uncertainty due to noise and
    /// possible assymetry error (see also weights below). (seconds)
    #[cfg_attr(feature = "serde", serde(default = "default_maximum_peer_uncertainty"))]
    pub maximum_peer_uncertainty: f64,
    /// Weight of statistical uncertainty when constructing
    /// overlap ranges. (standard deviations, 0+)
    #[cfg_attr(feature = "serde", serde(default = "default_range_statistical_weight"))]
    pub range_statistical_weight: f64,
    /// Weight of delay uncertainty when constructing overlap
    /// ranges. (weight, 0-1)
    #[cfg_attr(feature = "serde", serde(default = "default_range_delay_weight"))]
    pub range_delay_weight: f64,

    /// How far from 0 (in multiples of the uncertainty) should
    /// the offset be before we correct. (standard deviations, 0+)
    #[cfg_attr(feature = "serde", serde(default = "default_steer_offset_threshold"))]
    pub steer_offset_threshold: f64,
    /// How many standard deviations do we leave after offset
    /// correction? (standard deviations, 0+)
    #[cfg_attr(feature = "serde", serde(default = "default_steer_offset_leftover"))]
    pub steer_offset_leftover: f64,
    /// How far from 0 (in multiples of the uncertainty) should
    /// the frequency estimate be before we correct. (standard deviations, 0+)
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_steer_frequency_threshold")
    )]
    pub steer_frequency_threshold: f64,
    /// How many standard deviations do we leave after frequency
    /// correction? (standard deviations, 0+)
    #[cfg_attr(feature = "serde", serde(default = "default_steer_frequency_leftover"))]
    pub steer_frequency_leftover: f64,
    /// From what offset should we step the clock instead of
    /// trying to adjust gradually? (seconds, 0+)
    #[cfg_attr(feature = "serde", serde(default = "default_step_threshold"))]
    pub step_threshold: f64,
    /// What is the maximum frequency offset during a slew (s/s)
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_slew_maximum_frequency_offset")
    )]
    pub slew_maximum_frequency_offset: f64,
    /// What is the minimum duration of a slew (s)
    #[cfg_attr(feature = "serde", serde(default = "default_slew_minimum_duration"))]
    pub slew_minimum_duration: f64,

    /// Ignore a servers advertised dispersion when synchronizing.
    /// Can improve synchronization quality with servers reporting
    /// overly conservative root dispersion.
    #[cfg_attr(feature = "serde", serde(default))]
    pub ignore_server_dispersion: bool,
}

impl Default for KalmanFilterConfig {
    fn default() -> Self {
        Self {
            precision_low_probability: default_precision_low_probability(),
            precision_high_probability: default_precision_high_probability(),
            precision_hysteresis: default_precision_hysteresis(),
            precision_minimum_weight: default_precision_minimum_weight(),

            delay_outlier_threshold: default_delay_outlier_threshold(),

            initial_wander: default_initial_wander(),
            initial_frequency_uncertainty: default_initial_frequency_uncertainty(),

            maximum_peer_uncertainty: default_maximum_peer_uncertainty(),
            range_statistical_weight: default_range_statistical_weight(),
            range_delay_weight: default_range_delay_weight(),

            steer_offset_threshold: default_steer_offset_threshold(),
            steer_offset_leftover: default_steer_offset_leftover(),
            steer_frequency_threshold: default_steer_frequency_threshold(),
            steer_frequency_leftover: default_steer_frequency_leftover(),
            step_threshold: default_step_threshold(),
            slew_maximum_frequency_offset: default_slew_maximum_frequency_offset(),
            slew_minimum_duration: default_slew_minimum_duration(),

            ignore_server_dispersion: false,
        }
    }
}

fn default_precision_low_probability() -> f64 {
    1. / 3.
}

fn default_precision_high_probability() -> f64 {
    2. / 3.
}

fn default_precision_hysteresis() -> i32 {
    16
}

fn default_precision_minimum_weight() -> f64 {
    0.1
}

fn default_delay_outlier_threshold() -> f64 {
    5.
}

fn default_initial_wander() -> f64 {
    1e-8
}

fn default_initial_frequency_uncertainty() -> f64 {
    100e-6
}

fn default_maximum_peer_uncertainty() -> f64 {
    0.250
}

fn default_range_statistical_weight() -> f64 {
    2.
}

fn default_range_delay_weight() -> f64 {
    0.25
}

fn default_steer_offset_threshold() -> f64 {
    2.0
}

fn default_steer_offset_leftover() -> f64 {
    1.0
}

fn default_steer_frequency_threshold() -> f64 {
    0.0
}

fn default_steer_frequency_leftover() -> f64 {
    0.0
}

fn default_step_threshold() -> f64 {
    0.001
}

fn default_slew_maximum_frequency_offset() -> f64 {
    50e-6
}

fn default_slew_minimum_duration() -> f64 {
    0.5
}
