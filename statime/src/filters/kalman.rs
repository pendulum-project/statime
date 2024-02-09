use super::matrix::{Matrix, Vector};
#[allow(unused_imports)]
use crate::float_polyfill::FloatPolyfill;
use crate::{
    filters::Filter,
    port::Measurement,
    time::{Duration, Time},
};

/// Configuration options for [KalmanFilter]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct KalmanConfiguration {
    /// Threshold above which errors in time are corrected by steps
    pub step_threshold: Duration,
    /// Band of measured time offsets in which the algorithm doesn't try to
    /// correct the offset, in standard deviations.
    pub deadzone: f64,
    /// Amount of time to take for correcting time offsets, in seconds.
    ///
    /// Lower values make the clock correct more quickly, but also less
    /// precisely.
    pub steer_time: Duration,

    /// Maximum frequency offset to introduce during steering (ppm)
    pub max_steer: f64,
    /// Maximum correction to frequency (both from steering and to correct for
    /// the properties of the clock) the algorithm can make. (ppm)
    pub max_freq_offset: f64,

    /// Initial uncertainty about the clocks frequency
    pub initial_frequency_uncertainty: f64,
    /// Initial estimate for the wander of the clock's frequency in s/s
    pub initial_wander: f64,
    /// Amount of uncertainty introduced into the delay over time.
    /// Larger values allow quicker adaptation to changing conditions,
    /// at the cost of less precise clock synchronization.
    ///
    /// This is automatically scaled with the size of the delay. (percentage per
    /// second)
    pub delay_wander: f64,

    /// Likelyhood below which we start pushing the wander selection process
    /// towards assuming a more precise clock.
    pub precision_low_probability: f64,
    /// Likelyhood above which we start pushing the wander selection process
    /// towards assuming a less precise clock.
    pub precision_high_probability: f64,
    /// Amount of resistance to changes in wander (max 127)
    pub precision_hysteresis: u8,

    /// Maximum time between sync and delay to consider the combination for
    /// automatic channel error estimation. Lower values improve the
    /// estimate as it gets polluted less by uncertainties in the clock
    /// frequency, but increase startup time.
    pub estimate_threshold: Duration,
    /// Amount of samples needed to start automatic measurement channel error
    /// estimation (max 32)
    pub difference_estimation_boundary: usize,
    /// Amount of samples needed to switch automatic measurement channel error
    /// estimation to using sample variance instead of largest-spread. (max
    /// 32)
    pub statistical_estimation_boundary: usize,
    /// Multiplication factor on uncertainty estimation when it comes from
    /// peer delay measurements (to compensate for there being multiple path
    /// segments).
    pub peer_delay_factor: f64,
}

impl Default for KalmanConfiguration {
    fn default() -> Self {
        Self {
            step_threshold: Duration::from_seconds(1e-3),
            deadzone: 0.0,
            steer_time: Duration::from_seconds(2.0),
            max_steer: 200.0,
            max_freq_offset: 400.0,
            initial_frequency_uncertainty: 100e-6,
            initial_wander: 1e-16,
            delay_wander: 1e-4 / 3600.0,
            precision_low_probability: 1.0 / 3.0,
            precision_high_probability: 2.0 / 3.0,
            precision_hysteresis: 16,
            estimate_threshold: Duration::from_millis(200),
            difference_estimation_boundary: 4,
            statistical_estimation_boundary: 8,
            peer_delay_factor: 2.0,
        }
    }
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

fn sqr(x: f64) -> f64 {
    x * x
}

#[derive(Debug, Default, Copy, Clone)]
struct MeasurementErrorEstimator {
    data: [f64; 32],
    next_idx: usize,
    fill: usize,
    last_sync: Option<(Time, Duration)>,
    last_delay: Option<(Time, Duration)>,
    peer_delay_detected: bool,
}

impl MeasurementErrorEstimator {
    fn mean(&self) -> f64 {
        self.data.iter().take(self.fill).sum::<f64>() / (self.data.len() as f64)
    }

    fn variance(&self) -> f64 {
        let mean = self.mean();
        self.data
            .iter()
            .take(self.fill)
            .map(|v| sqr(v - mean))
            .sum::<f64>()
            / ((self.data.len() - 1) as f64)
    }

    fn range_size(&self) -> f64 {
        self.data
            .iter()
            .take(self.fill)
            .max_by(|x, y| {
                if x > y {
                    core::cmp::Ordering::Greater
                } else {
                    core::cmp::Ordering::Less
                }
            })
            .unwrap()
            - self
                .data
                .iter()
                .take(self.fill)
                .min_by(|x, y| {
                    if x > y {
                        core::cmp::Ordering::Greater
                    } else {
                        core::cmp::Ordering::Less
                    }
                })
                .unwrap()
    }

    fn insert_entry(&mut self, entry: f64) {
        log::info!("New entry {}", entry * 1e9);
        self.data[self.next_idx] = entry;
        self.next_idx = (self.next_idx + 1) % self.data.len();
        self.fill = (self.fill + 1).min(self.data.len())
    }

    fn absorb_measurement(
        &mut self,
        m: Measurement,
        estimated_frequency: f64,
        config: &KalmanConfiguration,
    ) {
        if let Some(sync_offset) = m.raw_sync_offset {
            if let Some((time, delay_offset)) = self.last_delay.take() {
                if (m.event_time - time).abs() < config.estimate_threshold {
                    self.insert_entry(
                        sync_offset.seconds() - delay_offset.seconds()
                            + (time - m.event_time).seconds() * estimated_frequency,
                    );
                    log::info!(
                        "New uncertainty estimate: {}ns",
                        self.measurement_variance(config).sqrt() * 1e9,
                    );
                } else {
                    self.last_sync = Some((m.event_time, sync_offset));
                }
            } else {
                self.last_sync = Some((m.event_time, sync_offset));
            }
        }

        if let Some(delay_offset) = m.raw_delay_offset {
            if let Some((time, sync_offset)) = self.last_sync.take() {
                if (m.event_time - time).abs() < config.estimate_threshold {
                    self.insert_entry(
                        sync_offset.seconds() - delay_offset.seconds()
                            + (m.event_time - time).seconds() * estimated_frequency,
                    );
                    log::info!(
                        "New uncertainty estimate: {}ns",
                        self.measurement_variance(config).sqrt() * 1e9,
                    );
                } else {
                    self.last_delay = Some((m.event_time, delay_offset));
                }
            } else {
                self.last_delay = Some((m.event_time, delay_offset));
            }
        }

        if let Some(peer_delay) = m.peer_delay {
            self.last_delay = None;
            self.last_sync = None;
            self.peer_delay_detected = true;
            self.insert_entry(peer_delay.seconds());
        }
    }

    fn measurement_variance(&self, config: &KalmanConfiguration) -> f64 {
        if self.fill < config.difference_estimation_boundary {
            sqr(config.steer_time.seconds())
        } else if self.fill < config.statistical_estimation_boundary {
            sqr(self.range_size())
        } else {
            self.variance() / 2.0
        }
    }

    fn peer_delay(&self) -> bool {
        self.peer_delay_detected
    }
}

#[derive(Clone, Debug)]
struct InnerFilter {
    state: Vector<3>,
    uncertainty: Matrix<3, 3>,
    filter_time: Time,
}

impl InnerFilter {
    const MEASUREMENT_SYNC: Matrix<1, 3> = Matrix::new([[1.0, 0.0, 1.0]]);
    const MEASUREMENT_DELAY: Matrix<1, 3> = Matrix::new([[1.0, 0.0, -1.0]]);
    const MEASUREMENT_PEER_DELAY: Matrix<1, 3> = Matrix::new([[0.0, 0.0, 1.0]]);

    fn new(initial_offset: f64, time: Time, config: &KalmanConfiguration) -> Self {
        Self {
            state: Vector::new_vector([initial_offset, 0.0, 0.0]),
            uncertainty: Matrix::new([
                [sqr(config.step_threshold.seconds()), 0.0, 0.0],
                [0.0, sqr(config.initial_frequency_uncertainty), 0.0],
                [0.0, 0.0, sqr(config.step_threshold.seconds())],
            ]),
            filter_time: time,
        }
    }

    fn progress_filtertime(&mut self, time: Time, wander: f64, config: &KalmanConfiguration) {
        debug_assert!(time >= self.filter_time);
        if time < self.filter_time {
            return;
        }

        let delta_t = (time - self.filter_time).seconds();
        let update = Matrix::new([[1.0, delta_t, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        let process_noise = Matrix::new([
            [
                wander * delta_t * delta_t * delta_t / 3.,
                wander * delta_t * delta_t / 2.,
                0.,
            ],
            [wander * delta_t * delta_t / 2., wander * delta_t, 0.],
            [
                0.,
                0.,
                config.delay_wander * delta_t * sqr(self.state.ventry(2)),
            ],
        ]);

        self.state = update * self.state;
        self.uncertainty = update * self.uncertainty * update.transpose() + process_noise;
        self.filter_time = time;
    }

    fn absorb_sync_offset(&mut self, sync_offset: f64, variance: f64) {
        let measurement_vec = Vector::new_vector([sync_offset]);
        let measurement_noise = Matrix::new([[variance]]);
        self.absorb_measurement(measurement_vec, Self::MEASUREMENT_SYNC, measurement_noise);
    }

    fn absorb_delay_offset(&mut self, delay_offset: f64, variance: f64) {
        let measurement_vec = Vector::new_vector([delay_offset]);
        let measurement_noise = Matrix::new([[variance]]);
        self.absorb_measurement(measurement_vec, Self::MEASUREMENT_DELAY, measurement_noise);
    }

    fn absorb_peer_delay(&mut self, peer_delay: f64, variance: f64) {
        let measurement_vec = Vector::new_vector([peer_delay]);
        let measurement_noise = Matrix::new([[variance]]);
        self.absorb_measurement(
            measurement_vec,
            Self::MEASUREMENT_PEER_DELAY,
            measurement_noise,
        )
    }

    fn absorb_measurement(
        &mut self,
        measurement_vec: Vector<1>,
        measurement_transform: Matrix<1, 3>,
        measurement_noise: Matrix<1, 1>,
    ) {
        let (prediction, uncertainty) = self.predict(measurement_transform);

        let difference = measurement_vec - prediction;
        let difference_covariance = uncertainty + measurement_noise;
        let update_strength =
            self.uncertainty * measurement_transform.transpose() * difference_covariance.inverse();
        self.state = self.state + update_strength * difference;
        self.uncertainty = ((Matrix::unit() - update_strength * measurement_transform)
            * self.uncertainty)
            .symmetrize();
    }

    fn absorb_frequency_steer(
        &mut self,
        steer: f64,
        time: Time,
        wander: f64,
        config: &KalmanConfiguration,
    ) {
        self.progress_filtertime(time, wander, config);
        self.state = self.state + Vector::new_vector([0., steer * 1e-6, 0.]);
    }

    fn absorb_offset_steer(&mut self, steer: f64) {
        self.state = self.state + Vector::new_vector([steer, 0., 0.]);
        self.filter_time += Duration::from_seconds(steer);
    }

    fn predict<const N: usize>(
        &self,
        measurement_transform: Matrix<N, 3>,
    ) -> (Vector<N>, Matrix<N, N>) {
        let prediction = measurement_transform * self.state;
        let uncertainty =
            measurement_transform * self.uncertainty * measurement_transform.transpose();
        (prediction, uncertainty)
    }

    fn predict_sync_offset(&self) -> (f64, f64) {
        let (prediction, uncertainty) = self.predict(Self::MEASUREMENT_SYNC);
        (prediction.entry(0, 0), uncertainty.entry(0, 0))
    }

    fn predict_delay_offset(&self) -> (f64, f64) {
        let (prediction, uncertainty) = self.predict(Self::MEASUREMENT_DELAY);
        (prediction.entry(0, 0), uncertainty.entry(0, 0))
    }
}

#[derive(Default, Debug, Clone)]
struct BaseFilter(Option<InnerFilter>);

impl BaseFilter {
    fn new() -> Self {
        Self(None)
    }

    fn progress_filtertime(&mut self, time: Time, wander: f64, config: &KalmanConfiguration) {
        match &mut self.0 {
            Some(inner) => inner.progress_filtertime(time, wander, config),
            None => self.0 = Some(InnerFilter::new(0.0, time, config)),
        }
    }

    fn absorb_sync_offset(
        &mut self,
        sync_offset: f64,
        variance: f64,
        config: &KalmanConfiguration,
    ) {
        if let Some(inner) = &mut self.0 {
            if (sync_offset - inner.state.ventry(0)).abs() > config.step_threshold.seconds() {
                log::info!("Measurement too far from state, resetting");
                *inner = InnerFilter::new(sync_offset, inner.filter_time, config);
            } else {
                inner.absorb_sync_offset(sync_offset, variance)
            }
        }
    }

    fn absorb_delay_offset(
        &mut self,
        delay_offset: f64,
        variance: f64,
        config: &KalmanConfiguration,
    ) {
        if let Some(inner) = &mut self.0 {
            if (delay_offset - inner.state.ventry(0)).abs() > config.step_threshold.seconds() {
                log::info!("Measurement too far from state, resetting");
                *inner = InnerFilter::new(delay_offset, inner.filter_time, config);
            } else {
                inner.absorb_delay_offset(delay_offset, variance)
            }
        }
    }

    fn absorb_peer_delay(&mut self, peer_delay: f64, variance: f64) {
        if let Some(inner) = &mut self.0 {
            inner.absorb_peer_delay(peer_delay, variance)
        }
    }

    fn absorb_frequency_steer(
        &mut self,
        steer: f64,
        time: Time,
        wander: f64,
        config: &KalmanConfiguration,
    ) {
        match &mut self.0 {
            Some(inner) => inner.absorb_frequency_steer(steer, time, wander, config),
            None => self.0 = Some(InnerFilter::new(0.0, time, config)),
        }
    }

    fn absorb_offset_steer(&mut self, steer: f64) {
        if let Some(inner) = &mut self.0 {
            inner.absorb_offset_steer(steer)
        }
    }

    fn offset(&self) -> f64 {
        self.0
            .as_ref()
            .map(|inner| inner.state.ventry(0))
            .unwrap_or(0.0)
    }

    fn offset_uncertainty(&self, config: &KalmanConfiguration) -> f64 {
        self.0
            .as_ref()
            .map(|inner| inner.uncertainty.entry(0, 0).sqrt())
            .unwrap_or(config.step_threshold.seconds())
    }

    fn freq_offset(&self) -> f64 {
        self.0
            .as_ref()
            .map(|inner| inner.state.ventry(1))
            .unwrap_or(0.0)
    }

    fn freq_offset_uncertainty(&self, config: &KalmanConfiguration) -> f64 {
        self.0
            .as_ref()
            .map(|inner| inner.uncertainty.entry(1, 1).sqrt())
            .unwrap_or(config.initial_frequency_uncertainty)
    }

    fn mean_delay(&self) -> f64 {
        self.0
            .as_ref()
            .map(|inner| inner.state.ventry(2))
            .unwrap_or(0.0)
    }

    fn mean_delay_uncertainty(&self, config: &KalmanConfiguration) -> f64 {
        self.0
            .as_ref()
            .map(|inner| inner.uncertainty.entry(2, 2).sqrt())
            .unwrap_or(config.step_threshold.seconds())
    }

    fn predict_sync_offset(&self, config: &KalmanConfiguration) -> (f64, f64) {
        self.0
            .as_ref()
            .map(|inner| inner.predict_sync_offset())
            .unwrap_or((0.0, sqr(config.step_threshold.seconds())))
    }

    fn predict_delay_offset(&self, config: &KalmanConfiguration) -> (f64, f64) {
        self.0
            .as_ref()
            .map(|inner| inner.predict_delay_offset())
            .unwrap_or((0.0, sqr(config.step_threshold.seconds())))
    }

    fn after_filter_time(&self, time: Time) -> bool {
        match &self.0 {
            Some(inner) => time >= inner.filter_time,
            None => true,
        }
    }
}

fn clamp_adjustment(current: f64, error: f64, bound: f64) -> f64 {
    if current + error > bound {
        bound - current
    } else if current + error < -bound {
        -bound - current
    } else {
        error
    }
}

/// Kalman filter based way for controlling the clock
pub struct KalmanFilter {
    config: KalmanConfiguration,
    running_filter: BaseFilter,
    wander_filter: BaseFilter,
    wander_score: i8,
    wander: f64,
    wander_measurement_error: f64,
    measurement_error_estimator: MeasurementErrorEstimator,
    cur_frequency: Option<f64>,
}

impl Filter for KalmanFilter {
    type Config = KalmanConfiguration;

    fn new(config: Self::Config) -> Self {
        let measurement_error_estimator = MeasurementErrorEstimator::default();
        KalmanFilter {
            running_filter: BaseFilter::new(),
            wander_filter: BaseFilter::new(),
            wander_score: 0,
            wander: config.initial_wander,
            wander_measurement_error: measurement_error_estimator
                .measurement_variance(&config)
                .sqrt(),
            measurement_error_estimator,
            cur_frequency: None,
            config,
        }
    }

    fn measurement<C: crate::Clock>(
        &mut self,
        m: Measurement,
        clock: &mut C,
    ) -> super::FilterUpdate {
        if !self.running_filter.after_filter_time(m.event_time) {
            return super::FilterUpdate::default();
        }

        self.measurement_error_estimator.absorb_measurement(
            m,
            self.running_filter.freq_offset(),
            &self.config,
        );

        self.update_wander(m);

        self.running_filter
            .progress_filtertime(m.event_time, self.wander, &self.config);
        if let Some(sync_offset) = m.raw_sync_offset {
            // We can start controlling, so need a proper frequency.
            self.ensure_freq_init(clock);

            self.running_filter.absorb_sync_offset(
                sync_offset.seconds(),
                self.measurement_error_estimator
                    .measurement_variance(&self.config)
                    * (if self.measurement_error_estimator.peer_delay() {
                        self.config.peer_delay_factor
                    } else {
                        1.0
                    }),
                &self.config,
            );
        }
        if let Some(delay_offset) = m.raw_delay_offset {
            // We can start controlling, so need a proper frequency.
            self.ensure_freq_init(clock);

            self.running_filter.absorb_delay_offset(
                delay_offset.seconds(),
                self.measurement_error_estimator
                    .measurement_variance(&self.config)
                    * (if self.measurement_error_estimator.peer_delay() {
                        self.config.peer_delay_factor
                    } else {
                        1.0
                    }),
                &self.config,
            );
        }
        if let Some(peer_delay) = m.peer_delay {
            self.running_filter.absorb_peer_delay(
                peer_delay.seconds(),
                self.measurement_error_estimator
                    .measurement_variance(&self.config)
                    * (if self.measurement_error_estimator.peer_delay() {
                        self.config.peer_delay_factor
                    } else {
                        1.0
                    }),
            );
        }

        self.display_state();

        self.steer(clock)
    }

    fn update<C: crate::Clock>(&mut self, clock: &mut C) -> super::FilterUpdate {
        // Remote has gone away, set frequency as close as possible to our best estimate
        // of correct
        self.change_frequency(0.0, clock);
        super::FilterUpdate {
            next_update: None,
            mean_delay: Some(Duration::from_seconds(self.running_filter.mean_delay())),
        }
    }

    fn demobilize<C: crate::Clock>(mut self, clock: &mut C) {
        // Remote has gone away, set frequency as close as possible to our best estimate
        // of correct
        self.change_frequency(0.0, clock);
    }
}

impl KalmanFilter {
    fn change_frequency<C: crate::Clock>(&mut self, target: f64, clock: &mut C) {
        if let Some(cur_frequency) = self.cur_frequency {
            let error_ppm = clamp_adjustment(
                cur_frequency,
                self.running_filter.freq_offset() * 1e6 - target,
                self.config.max_freq_offset,
            );
            if let Ok(time) = clock.set_frequency(cur_frequency - error_ppm) {
                self.cur_frequency = Some(cur_frequency - error_ppm);
                self.running_filter.absorb_frequency_steer(
                    -error_ppm,
                    time,
                    self.wander,
                    &self.config,
                );
                self.wander_filter.absorb_frequency_steer(
                    -error_ppm,
                    time,
                    self.wander,
                    &self.config,
                );
                log::info!(
                    "Steered frequency by {}ppm (target: {}ppm)",
                    error_ppm,
                    target
                );
            } else {
                log::error!("Could not adjust clock frequency");
            }
        }
    }

    fn step<C: crate::Clock>(&mut self, clock: &mut C, offset: f64) {
        if clock.step_clock(Duration::from_seconds(-offset)).is_ok() {
            log::info!("Stepped clock by {}s", -offset);
            self.running_filter.absorb_offset_steer(-offset);
            self.wander_filter.absorb_offset_steer(-offset);
        }
    }

    fn display_state(&self) {
        log::info!(
            "Estimated offset {}ns+-{}ns, freq {}+-{}, delay {}+-{}",
            self.running_filter.offset() * 1e9,
            self.running_filter.offset_uncertainty(&self.config) * 1e9,
            self.running_filter.freq_offset() * 1e6,
            self.running_filter.freq_offset_uncertainty(&self.config) * 1e6,
            self.running_filter.mean_delay() * 1e9,
            self.running_filter.mean_delay_uncertainty(&self.config) * 1e9
        );
    }

    fn steer<C: crate::Clock>(&mut self, clock: &mut C) -> super::FilterUpdate {
        let error = self.running_filter.offset();
        if error.abs() < self.config.step_threshold.seconds() {
            let desired_adjust = error.signum()
                * (error.abs()
                    - self.running_filter.offset_uncertainty(&self.config) * self.config.deadzone)
                    .max(0.0);
            let target = (-desired_adjust * 1e6 / self.config.steer_time.seconds())
                .clamp(-self.config.max_steer, self.config.max_steer);
            self.change_frequency(target, clock);
            super::FilterUpdate {
                next_update: Some(core::time::Duration::from_secs_f64(
                    self.config.steer_time.seconds(),
                )),
                mean_delay: Some(Duration::from_seconds(self.running_filter.mean_delay())),
            }
        } else {
            self.step(clock, error);
            super::FilterUpdate {
                next_update: None,
                mean_delay: Some(Duration::from_seconds(self.running_filter.mean_delay())),
            }
        }
    }

    fn wander_score_update(&mut self, uncertainty: f64, prediction: f64, actual: f64) {
        log::info!("Wander uncertainty: {}ns", uncertainty.sqrt() * 1e9);
        if self.wander_measurement_error
            > 10.0
                * self
                    .measurement_error_estimator
                    .measurement_variance(&self.config)
                    .sqrt()
        {
            self.wander_filter = self.running_filter.clone();
            self.wander_measurement_error = self
                .measurement_error_estimator
                .measurement_variance(&self.config)
                .sqrt()
        } else if uncertainty.sqrt() > 10.0 * self.wander_measurement_error {
            log::info!(
                "Wander update predict: {}ns, actual: {}ns",
                prediction * 1e9,
                actual * 1e9
            );
            let p = 1.
                - chi_1(
                    sqr(actual - prediction) / (uncertainty + sqr(self.wander_measurement_error)),
                );
            log::info!("p: {}", p);
            if p < self.config.precision_low_probability {
                self.wander_score = self.wander_score.saturating_sub(1);
            } else if p > self.config.precision_high_probability {
                self.wander_score = self.wander_score.saturating_add(1);
            } else {
                self.wander_score = self.wander_score - self.wander_score.signum();
            }
            log::info!("Wander update");
            self.wander_filter = self.running_filter.clone();
            self.wander_measurement_error = self
                .measurement_error_estimator
                .measurement_variance(&self.config)
                .sqrt();
        }
    }

    fn update_wander(&mut self, m: Measurement) {
        self.wander_filter
            .progress_filtertime(m.event_time, self.wander, &self.config);
        if let Some(sync_offset) = m.raw_sync_offset {
            let (prediction, uncertainty) = self.wander_filter.predict_sync_offset(&self.config);
            self.wander_score_update(uncertainty, prediction, sync_offset.seconds());
        }
        if let Some(delay_offset) = m.raw_delay_offset {
            let (prediction, uncertainty) = self.wander_filter.predict_delay_offset(&self.config);
            self.wander_score_update(uncertainty, prediction, delay_offset.seconds());
        }
        log::info!("wander score: {}", self.wander_score);
        if self.wander_score < -(self.config.precision_hysteresis as i8) {
            self.wander /= 4.0;
            self.wander_score = 0;
            log::info!("Updated wander estimate: {:e}", self.wander);
        }
        if self.wander_score > (self.config.precision_hysteresis as i8) {
            self.wander *= 4.0;
            self.wander_score = 0;
            log::info!("Updated wander estimate: {:e}", self.wander);
        }
    }

    fn ensure_freq_init<C: crate::Clock>(&mut self, clock: &mut C) {
        // TODO: At some point we should probably look at a better
        // mechanism for this than just resetting the frequency.
        if self.cur_frequency.is_none() {
            if clock.set_frequency(0.0).is_ok() {
                self.cur_frequency = Some(0.0);
            } else {
                log::error!("Could not adjust clock frequency");
            }
        }
    }
}
