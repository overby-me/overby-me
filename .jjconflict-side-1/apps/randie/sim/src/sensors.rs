//! What the drone's sensors would have read, given where it is.
//!
//! Each sensor is a fan of rays cast against the room, and each writes what it
//! finds into the firmware's own sensor struct. From the firmware's side there
//! is no difference between this and a wire.

// Simulated sensor readings are floats by definition.
#![allow(clippy::cast_precision_loss)]

use core::f64::consts::FRAC_PI_2;

use randie_firmware::laser::{LASER_MAX_DISTANCE_CM, Laser};
use randie_firmware::sonar::Sonar;

use crate::block::{Block, BlockType};
use crate::ray::Ray;
use crate::vector::Vec2;

/// How far the laser reaches, in centimetres. Shorter than the range the
/// module reports at, so there is a band where it sees nothing and says so.
pub const LASER_LENGTH: f64 = 400.0;

/// How many rays the sonar's cone is sampled with.
pub const SONAR_RAYS: usize = 57;
/// How wide the sonar's cone is, in degrees.
pub const SONAR_SPAN_DEGREES: f64 = 15.0;
/// How far the sonar reaches, in centimetres.
pub const SONAR_LENGTH: f64 = 220.0;

/// The three-beam laser range finder: one beam left, one ahead, one right.
#[derive(Clone, Debug)]
pub struct SimLaser {
    /// The beams, in the order left, front, right.
    pub rays: [Ray; 3],
    /// How far they reach.
    pub length: f64,
}

impl SimLaser {
    /// Where the left-hand beam sits in [`SimLaser::rays`].
    pub const LEFT: usize = 0;
    /// Where the forward beam sits in [`SimLaser::rays`].
    pub const FRONT: usize = 1;
    /// Where the right-hand beam sits in [`SimLaser::rays`].
    pub const RIGHT: usize = 2;

    /// A laser at a drone's position and heading.
    #[must_use]
    pub fn new(origin: Vec2, angle: f64, length: f64) -> Self {
        Self {
            rays: [
                Ray::new(origin, length, angle),
                Ray::new(origin, length, angle),
                Ray::new(origin, length, angle),
            ],
            length,
        }
    }

    /// Casts the three beams and reports what they hit.
    ///
    /// Glass is transparent to the laser, so window blocks are skipped
    /// outright. Anything further off than the beam reaches reads as the
    /// module's maximum, which is what it sends when nothing came back.
    pub fn update(&mut self, pos: Vec2, angle: f64, blocks: &[Block], out: &mut Laser) {
        self.rays[Self::LEFT].update(pos, angle + FRAC_PI_2);
        self.rays[Self::FRONT].update(pos, angle);
        self.rays[Self::RIGHT].update(pos, angle - FRAC_PI_2);

        let mut readings = [LASER_MAX_DISTANCE_CM; 3];

        for block in blocks {
            if block.kind == BlockType::Window {
                continue;
            }

            for (reading, ray) in readings.iter_mut().zip(&self.rays) {
                if let Some(distance) = block.intersect(ray) {
                    *reading = (*reading).min(distance as u16);
                }
            }
        }

        for reading in &mut readings {
            if f64::from(*reading) > self.length {
                *reading = LASER_MAX_DISTANCE_CM;
            }
        }

        out.val_left = readings[Self::LEFT];
        out.val_front = readings[Self::FRONT];
        out.val_right = readings[Self::RIGHT];
    }
}

/// The forward sonar: a cone, sampled as a fan of rays.
#[derive(Clone, Debug)]
pub struct SimSonar {
    /// The rays the cone is sampled with, from one edge to the other.
    pub rays: Vec<Ray>,
    /// How wide the cone is, in radians.
    pub span: f64,
    /// How far it reaches, in centimetres.
    pub length: f64,
}

impl SimSonar {
    /// A sonar at a drone's position and heading.
    #[must_use]
    pub fn new(origin: Vec2, angle: f64, ray_count: usize, span: f64, length: f64) -> Self {
        Self {
            rays: (0..ray_count)
                .map(|_| Ray::new(origin, length, angle))
                .collect(),
            span,
            length,
        }
    }

    /// Casts the cone and reports the nearest thing in it.
    ///
    /// Glass counts: the sonar is the only sensor that sees a window, which is
    /// how the drone tells one from a doorway.
    ///
    /// A reading out of range clears the valid flag but leaves the last
    /// distance in place, as the C does. That matters, because the navigator
    /// reads the distance without checking the flag when it marks up its map.
    pub fn update(&mut self, pos: Vec2, angle: f64, blocks: &[Block], out: &mut Sonar) {
        let resolution = self.span / self.rays.len() as f64;
        let start = angle - (self.span / 2.0);

        for (index, ray) in self.rays.iter_mut().enumerate() {
            ray.update(pos, start + (index as f64 * resolution));
        }

        let nearest = blocks
            .iter()
            .flat_map(|block| self.rays.iter().filter_map(|ray| block.intersect(ray)))
            .fold(f64::INFINITY, f64::min);

        out.valid = nearest <= self.length;
        if out.valid {
            out.value = nearest as u16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use randie_firmware::io::{DPin, Tx};

    /// A wall a metre east of the origin, and nothing else.
    fn wall_to_the_east() -> Vec<Block> {
        vec![Block::new(Vec2::new(100.0, 0.0), BlockType::Wall)]
    }

    #[test]
    fn the_laser_reads_what_is_in_front_of_it() {
        let mut laser = SimLaser::new(Vec2::ZERO, 0.0, LASER_LENGTH);
        let mut out = Laser::new(Tx::Tx1);

        laser.update(Vec2::ZERO, 0.0, &wall_to_the_east(), &mut out);

        assert_eq!(out.val_front, 87);
        assert_eq!(out.val_left, LASER_MAX_DISTANCE_CM);
        assert_eq!(out.val_right, LASER_MAX_DISTANCE_CM);
    }

    #[test]
    fn turning_the_drone_turns_the_beams_with_it() {
        let mut laser = SimLaser::new(Vec2::ZERO, 0.0, LASER_LENGTH);
        let mut out = Laser::new(Tx::Tx1);

        // Facing north, so the wall to the east is off the right-hand beam.
        laser.update(Vec2::ZERO, FRAC_PI_2, &wall_to_the_east(), &mut out);

        assert_eq!(out.val_right, 87);
        assert_eq!(out.val_front, LASER_MAX_DISTANCE_CM);
    }

    #[test]
    fn the_laser_goes_through_glass() {
        let glass = vec![Block::new(Vec2::new(100.0, 0.0), BlockType::Window)];
        let mut laser = SimLaser::new(Vec2::ZERO, 0.0, LASER_LENGTH);
        let mut out = Laser::new(Tx::Tx1);

        laser.update(Vec2::ZERO, 0.0, &glass, &mut out);

        assert_eq!(out.val_front, LASER_MAX_DISTANCE_CM);
    }

    #[test]
    fn something_out_of_the_lasers_reach_reads_as_nothing() {
        let far = vec![Block::new(Vec2::new(1000.0, 0.0), BlockType::Wall)];
        let mut laser = SimLaser::new(Vec2::ZERO, 0.0, LASER_LENGTH);
        let mut out = Laser::new(Tx::Tx1);

        laser.update(Vec2::ZERO, 0.0, &far, &mut out);

        assert_eq!(out.val_front, LASER_MAX_DISTANCE_CM);
    }

    #[test]
    fn the_sonar_reads_what_is_in_front_of_it() {
        let mut sonar = SimSonar::new(
            Vec2::ZERO,
            0.0,
            SONAR_RAYS,
            SONAR_SPAN_DEGREES.to_radians(),
            SONAR_LENGTH,
        );
        let mut out = Sonar::new(DPin::P2, DPin::P3);

        sonar.update(Vec2::ZERO, 0.0, &wall_to_the_east(), &mut out);

        assert!(out.valid);
        assert_eq!(out.value, 87);
    }

    #[test]
    fn the_sonar_sees_glass() {
        let glass = vec![Block::new(Vec2::new(100.0, 0.0), BlockType::Window)];
        let mut sonar = SimSonar::new(
            Vec2::ZERO,
            0.0,
            SONAR_RAYS,
            SONAR_SPAN_DEGREES.to_radians(),
            SONAR_LENGTH,
        );
        let mut out = Sonar::new(DPin::P2, DPin::P3);

        sonar.update(Vec2::ZERO, 0.0, &glass, &mut out);

        assert!(out.valid);
        assert_eq!(out.value, 87);
    }

    #[test]
    fn an_out_of_range_sonar_reading_is_marked_invalid_but_left_in_place() {
        let far = vec![Block::new(Vec2::new(1000.0, 0.0), BlockType::Wall)];
        let mut sonar = SimSonar::new(
            Vec2::ZERO,
            0.0,
            SONAR_RAYS,
            SONAR_SPAN_DEGREES.to_radians(),
            SONAR_LENGTH,
        );
        let mut out = Sonar::new(DPin::P2, DPin::P3);
        out.value = 42;

        sonar.update(Vec2::ZERO, 0.0, &far, &mut out);

        assert!(!out.valid);
        assert_eq!(out.value, 42);
    }

    #[test]
    fn the_sonars_cone_catches_what_a_single_beam_would_miss() {
        // A block off to one side, inside the cone but not straight ahead.
        let off_axis = vec![Block::new(Vec2::new(200.0, 20.0), BlockType::Wall)];
        let mut sonar = SimSonar::new(
            Vec2::ZERO,
            0.0,
            SONAR_RAYS,
            SONAR_SPAN_DEGREES.to_radians(),
            SONAR_LENGTH,
        );
        let mut out = Sonar::new(DPin::P2, DPin::P3);

        sonar.update(Vec2::ZERO, 0.0, &off_axis, &mut out);

        assert!(out.valid);
    }
}
