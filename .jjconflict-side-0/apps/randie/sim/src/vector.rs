//! Two-dimensional vectors, in room coordinates.
//!
//! Everything in the simulated room is measured in centimetres from where the
//! drone was switched on, with x to the east and y to the north. That is not
//! where the firmware thinks it is: the drone's own map has its origin in a
//! corner and its position on it comes from dead reckoning. Keeping the two
//! apart is the point of the simulator.

use core::ops::{Add, AddAssign, Mul, Sub};

/// A point or a direction, in centimetres.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    /// The origin.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// A vector from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// A vector of the given length at the given angle, anticlockwise from
    /// east.
    #[must_use]
    pub fn from_polar(length: f64, angle: f64) -> Self {
        Self {
            x: angle.cos() * length,
            y: angle.sin() * length,
        }
    }

    /// How long it is.
    #[must_use]
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// The dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// The cross product's magnitude: the signed area of the parallelogram the
    /// two vectors span.
    #[must_use]
    pub fn determinant(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    /// Each component's reciprocal.
    #[must_use]
    pub fn invert(self) -> Self {
        Self {
            x: 1.0 / self.x,
            y: 1.0 / self.y,
        }
    }
}

impl Add for Vec2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f64> for Vec2 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl AddAssign for Vec2 {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::FRAC_PI_2;

    #[test]
    fn a_polar_vector_points_where_it_says() {
        let east = Vec2::from_polar(10.0, 0.0);
        assert!((east.x - 10.0).abs() < 1e-9);
        assert!(east.y.abs() < 1e-9);

        let north = Vec2::from_polar(10.0, FRAC_PI_2);
        assert!(north.x.abs() < 1e-9);
        assert!((north.y - 10.0).abs() < 1e-9);
    }

    #[test]
    fn length_is_the_hypotenuse() {
        assert!((Vec2::new(3.0, 4.0).length() - 5.0).abs() < 1e-9);
        assert_eq!(Vec2::ZERO.length(), 0.0);
    }

    #[test]
    fn arithmetic_is_componentwise() {
        let a = Vec2::new(1.0, 2.0);
        let b = Vec2::new(10.0, 20.0);

        assert_eq!(a + b, Vec2::new(11.0, 22.0));
        assert_eq!(b - a, Vec2::new(9.0, 18.0));
        assert_eq!(a * 3.0, Vec2::new(3.0, 6.0));
    }

    #[test]
    fn perpendicular_vectors_have_no_dot_and_all_determinant() {
        let east = Vec2::new(1.0, 0.0);
        let north = Vec2::new(0.0, 1.0);

        assert_eq!(east.dot(north), 0.0);
        assert_eq!(east.determinant(north), 1.0);
    }
}
