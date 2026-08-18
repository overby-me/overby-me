//! Sensor beams.
//!
//! A ray is where a beam starts, which way it points, and how far it reaches.
//! Its direction vector is as long as the beam, so intersections come back as
//! a fraction of the beam's length and scale straight back to centimetres.

use crate::vector::Vec2;

/// One beam.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Ray {
    /// Where the beam leaves the sensor.
    pub origin: Vec2,
    /// Which way it points, scaled to how far it reaches.
    pub direction: Vec2,
    /// How far it reaches, in centimetres.
    pub length: f64,
    /// Which way it points, in radians anticlockwise from east.
    pub angle: f64,
}

impl Ray {
    /// A beam from `origin`, `length` centimetres long, pointing at `angle`.
    #[must_use]
    pub fn new(origin: Vec2, length: f64, angle: f64) -> Self {
        Self {
            origin,
            direction: Vec2::from_polar(length, angle),
            length,
            angle,
        }
    }

    /// Points the beam somewhere else.
    pub fn update(&mut self, origin: Vec2, angle: f64) {
        self.origin = origin;
        self.angle = angle;
        self.direction = Vec2::from_polar(self.length, angle);
    }

    /// Where the beam ends if nothing stops it.
    #[must_use]
    pub fn end(&self) -> Vec2 {
        self.origin + self.direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::FRAC_PI_2;

    #[test]
    fn a_ray_reaches_as_far_as_it_is_long() {
        let ray = Ray::new(Vec2::ZERO, 400.0, 0.0);
        assert!((ray.direction.length() - 400.0).abs() < 1e-9);
        assert!((ray.end().x - 400.0).abs() < 1e-9);
    }

    #[test]
    fn pointing_a_ray_moves_its_end_but_not_its_reach() {
        let mut ray = Ray::new(Vec2::ZERO, 400.0, 0.0);
        ray.update(Vec2::new(10.0, 10.0), FRAC_PI_2);

        assert_eq!(ray.origin, Vec2::new(10.0, 10.0));
        assert!((ray.direction.length() - 400.0).abs() < 1e-9);
        assert!((ray.end().y - 410.0).abs() < 1e-9);
    }
}
