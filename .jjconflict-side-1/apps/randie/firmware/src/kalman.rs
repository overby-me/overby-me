//! A one-dimensional Kalman filter, for smoothing a single sensor.

use crate::fix16::Fix16;
use crate::log::Sender;

/// How many rounds [`KalmanState::calibrate`] will run before giving up.
///
/// The C spun until the estimate came within the sensor's variance of the
/// first reading, with nothing to stop it if that never happened -- a gain of
/// zero, or a variance of zero, and the drone hangs on the ground. The loop is
/// bounded here. Converging takes a handful of rounds for any sane pair.
const CALIBRATION_LIMIT: u32 = 1000;

/// The filter's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KalmanState {
    /// Which component the filter is attached to. Only used for logging.
    pub source_component: Sender,
    /// How much the next measurement is assumed to differ from the last.
    pub a: Fix16,
    /// The sensor's variance: how far a reading tends to fall from the truth.
    pub r: Fix16,
    /// The current observation.
    pub z_k: Fix16,
    /// The Kalman gain.
    pub g_k: Fix16,
    /// The prediction error.
    pub p_k: Fix16,
    /// The state estimate: the filtered reading.
    pub x_k: Fix16,
    /// The control signal.
    pub u_k: Fix16,
}

impl KalmanState {
    /// A filter with the given system and sensor constants.
    #[must_use]
    pub fn new(a: Fix16, r: Fix16, component: Sender) -> Self {
        Self {
            source_component: component,
            a,
            r,
            z_k: Fix16::ZERO,
            // Start uncertain, so the first readings move the estimate freely.
            p_k: Fix16::from_int(10),
            g_k: Fix16::ZERO,
            x_k: Fix16::ZERO,
            u_k: Fix16::ZERO,
        }
    }

    /// Folds one reading into the estimate.
    pub fn run(&mut self, z_k: Fix16) {
        self.z_k = z_k;

        // Predict from the previous round: x̂ₖ = a · x̂ₖ₋₁, pₖ = a · pₖ₋₁ · a.
        self.x_k = self.a * self.x_k;
        self.p_k = self.a * self.a * self.p_k;

        // gₖ = pₖ / (pₖ + r)
        self.g_k = self.p_k / (self.p_k + self.r);

        // x̂ₖ = x̂ₖ + gₖ · (zₖ − x̂ₖ)
        self.x_k = self.x_k + self.g_k * (self.z_k - self.x_k);

        // pₖ = (1 − gₖ) · pₖ
        self.p_k = (Fix16::ONE - self.g_k) * self.p_k;
    }

    /// Runs the filter on the first reading until the estimate agrees with it,
    /// so that the drone does not spend its first seconds chasing a state that
    /// started at zero.
    ///
    /// Returns whether it converged inside [`CALIBRATION_LIMIT`] rounds.
    pub fn calibrate(&mut self, z_0: Fix16) -> bool {
        for _ in 0..CALIBRATION_LIMIT {
            self.run(z_0);

            if (self.x_k - z_0).abs() <= self.r {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> KalmanState {
        KalmanState::new(Fix16::ONE, Fix16::from_f64(0.1), Sender::Sonar)
    }

    #[test]
    fn a_run_of_equal_readings_converges_on_them() {
        let mut filter = filter();
        for _ in 0..50 {
            filter.run(Fix16::from_int(100));
        }

        assert!((filter.x_k.to_f64() - 100.0).abs() < 0.5);
    }

    #[test]
    fn the_estimate_settles_between_noisy_readings() {
        let mut filter = filter();
        for round in 0..100 {
            // 100 either side of 50, alternating.
            filter.run(Fix16::from_int(if round % 2 == 0 { 40 } else { 60 }));
        }

        assert!((filter.x_k.to_f64() - 50.0).abs() < 5.0);
    }

    #[test]
    fn uncertainty_falls_as_readings_arrive() {
        let mut filter = filter();
        let start = filter.p_k;
        for _ in 0..10 {
            filter.run(Fix16::from_int(20));
        }

        assert!(filter.p_k < start);
    }

    #[test]
    fn calibration_lands_on_the_first_reading() {
        let mut filter = filter();
        assert!(filter.calibrate(Fix16::from_int(70)));
        assert!((filter.x_k.to_f64() - 70.0).abs() <= 0.1);
    }

    #[test]
    fn calibration_gives_up_rather_than_hanging() {
        // A system constant of zero pins the estimate at zero however many
        // readings arrive, which is what hung the C.
        let mut filter = KalmanState::new(Fix16::ZERO, Fix16::ONE, Sender::Ir);
        assert!(!filter.calibrate(Fix16::from_int(70)));
    }
}
