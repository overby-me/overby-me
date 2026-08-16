//! A Kalman filter over two sensors at once.
//!
//! The laser and the sonar both measure the distance to whatever is in front of
//! the drone, and they fail in different ways: the laser goes through glass,
//! the sonar's cone catches things off to the side. Fusing them gives one
//! distance with a better error than either.
//!
//! Two lines here differ from the C, both in the same direction. Its predict
//! step computed the new uncertainty from the *state* (`p = a·a·x`) rather than
//! from the previous uncertainty, and its covariance update subtracted the gain
//! from a raw `1` -- the integer 1, which in 16.16 is 0.000015 -- rather than
//! from one. Neither is a modelling choice: `kalman.c`, the single-sensor
//! filter alongside it, writes both expressions correctly, so the data-fusion
//! copy is a botched transcription of its own sibling. They are corrected here,
//! because a covariance that tracks the state and then changes sign is not a
//! Kalman filter at all.

use crate::fix16::Fix16;
use crate::log::Sender;
use crate::matrix::Matrix;

/// How many sensors the filter fuses.
pub const DATAFUSION_UNITS: usize = 2;
/// Where the laser's reading sits in the observation vector.
pub const ZLASER: usize = 0;
/// Where the sonar's reading sits in the observation vector.
pub const ZSONAR: usize = 1;

/// How many rounds [`KalmanDatafusion::calibrate`] will run before giving up.
/// The C had no such bound; see [`crate::kalman`].
const CALIBRATION_LIMIT: u32 = 1000;

/// The fused filter's state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KalmanDatafusion {
    /// Which component the filter is attached to. Only used for logging.
    pub source_component: Sender,
    /// How much the next measurement is assumed to differ from the last.
    pub a: Fix16,
    /// How much the control input counts towards the next estimate.
    pub b: Fix16,
    /// How each sensor's reading scales against the state. One column, one row
    /// per sensor.
    pub c: Matrix,
    /// The sensors' variances, on the diagonal.
    pub r: Matrix,
    /// The control signal.
    pub u_k: Fix16,
    /// The current observations, one row per sensor.
    pub z_k: Matrix,
    /// The Kalman gain, one column per sensor.
    pub g_k: Matrix,
    /// The prediction error.
    pub p_k: Fix16,
    /// The state estimate: the fused reading.
    pub x_k: Fix16,
}

impl KalmanDatafusion {
    /// A filter over two sensors.
    ///
    /// `c` and `r` default to all-ones and the identity, which weighs both
    /// sensors the same and assumes unit variance on each.
    #[must_use]
    pub fn new(
        a: Fix16,
        b: Fix16,
        component: Sender,
        c: Option<Matrix>,
        r: Option<Matrix>,
    ) -> Self {
        let c = c.unwrap_or_else(|| {
            let mut c = Matrix::new(DATAFUSION_UNITS, 1);
            c.set(0, 0, Fix16::ONE);
            c.set(1, 0, Fix16::ONE);
            c
        });

        let r = r.unwrap_or_else(|| Matrix::identity(DATAFUSION_UNITS));

        let mut g_k = Matrix::new(1, DATAFUSION_UNITS);
        g_k.set(0, 0, Fix16::ONE);
        g_k.set(0, 1, Fix16::ONE);

        Self {
            source_component: component,
            a,
            b,
            c,
            r,
            u_k: Fix16::ONE,
            z_k: Matrix::new(DATAFUSION_UNITS, 1),
            g_k,
            // Start uncertain, so the first readings move the estimate freely.
            p_k: Fix16::from_int(10),
            x_k: Fix16::ZERO,
        }
    }

    /// Folds one reading from each sensor into the estimate.
    pub fn filter(&mut self, z_laser: Fix16, z_sonar: Fix16) {
        self.z_k.set(ZLASER, 0, z_laser);
        self.z_k.set(ZSONAR, 0, z_sonar);

        self.predict();
        self.update();
    }

    /// Runs the filter on the first pair of readings until the estimate agrees
    /// with their average, to within the average of the sensors' variances.
    ///
    /// Returns whether it converged inside [`CALIBRATION_LIMIT`] rounds.
    pub fn calibrate(&mut self, z_0_laser: Fix16, z_0_sonar: Fix16) -> bool {
        let average = (z_0_laser + z_0_sonar) / Fix16::from_int(2);

        let mut variance = Fix16::ZERO;
        for row in 0..DATAFUSION_UNITS {
            for column in 0..DATAFUSION_UNITS {
                variance += self.r.get(row, column);
            }
        }
        let variance = variance / (Fix16::from_int(2) * Fix16::from_int(DATAFUSION_UNITS as i32));

        for _ in 0..CALIBRATION_LIMIT {
            self.filter(z_0_laser, z_0_sonar);

            if (average - self.x_k).abs() <= variance {
                return true;
            }
        }

        false
    }

    /// Steps the estimate and its uncertainty forward by one round.
    fn predict(&mut self) {
        self.x_k = self.a * self.x_k + self.b * self.u_k;
        self.p_k = self.a * self.a * self.p_k;
    }

    fn update(&mut self) {
        self.calculate_gain();
        self.calculate_state();
        self.calculate_error();
    }

    /// G = p·Cᵀ · (C·p·Cᵀ + R)⁻¹
    fn calculate_gain(&mut self) {
        let transposed = self.c.transpose();
        let scaled_transpose = transposed.scale(self.p_k);
        let scaled = self.c.scale(self.p_k);

        // The shapes are fixed by the constructor, so these cannot fail.
        let Some(spread) = scaled.mul(&transposed) else {
            return;
        };
        let Some(with_noise) = spread.add(&self.r) else {
            return;
        };
        let Some(inverse) = with_noise.invert() else {
            return;
        };
        let Some(gain) = scaled_transpose.mul(&inverse) else {
            return;
        };

        self.g_k = gain;
    }

    /// x = x + G·(z − C·x)
    fn calculate_state(&mut self) {
        let predicted = self.c.scale(self.x_k);

        let Some(residual) = self.z_k.sub(&predicted) else {
            return;
        };
        // A 1xN gain against an Nx1 residual: one number.
        let Some(correction) = self.g_k.mul(&residual) else {
            return;
        };

        self.x_k += correction.get(0, 0);
    }

    /// p = (1 − G·C)·p
    fn calculate_error(&mut self) {
        let Some(product) = self.g_k.mul(&self.c) else {
            return;
        };

        self.p_k = (Fix16::ONE - product.get(0, 0)) * self.p_k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> KalmanDatafusion {
        KalmanDatafusion::new(Fix16::ONE, Fix16::ZERO, Sender::Sonar, None, None)
    }

    #[test]
    fn two_sensors_that_agree_are_believed() {
        let mut filter = filter();
        for _ in 0..50 {
            filter.filter(Fix16::from_int(100), Fix16::from_int(100));
        }

        assert!((filter.x_k.to_f64() - 100.0).abs() < 1.0);
    }

    #[test]
    fn two_sensors_that_disagree_are_split_between() {
        let mut filter = filter();
        for _ in 0..50 {
            filter.filter(Fix16::from_int(80), Fix16::from_int(120));
        }

        assert!((filter.x_k.to_f64() - 100.0).abs() < 5.0);
    }

    #[test]
    fn uncertainty_falls_rather_than_changing_sign() {
        let mut filter = filter();
        let start = filter.p_k;
        for _ in 0..10 {
            filter.filter(Fix16::from_int(50), Fix16::from_int(50));
        }

        assert!(filter.p_k > Fix16::ZERO, "uncertainty went negative");
        assert!(filter.p_k < start);
    }

    #[test]
    fn the_noisier_sensor_counts_for_less() {
        // Twice the variance on the sonar, so the fused reading sits nearer
        // the laser's.
        let mut variances = Matrix::identity(DATAFUSION_UNITS);
        variances.set(ZSONAR, ZSONAR, Fix16::from_int(20));

        let mut filter = KalmanDatafusion::new(
            Fix16::ONE,
            Fix16::ZERO,
            Sender::Sonar,
            None,
            Some(variances),
        );

        for _ in 0..50 {
            filter.filter(Fix16::from_int(80), Fix16::from_int(120));
        }

        assert!(filter.x_k.to_f64() < 100.0, "{:?}", filter.x_k);
    }

    #[test]
    fn calibration_lands_near_the_first_pair() {
        let mut filter = filter();
        assert!(filter.calibrate(Fix16::from_int(60), Fix16::from_int(64)));
        assert!((filter.x_k.to_f64() - 62.0).abs() <= 1.0);
    }
}
