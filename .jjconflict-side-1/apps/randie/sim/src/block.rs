//! The room, as a grid of blocks.
//!
//! Everything the drone can run into is a 25 cm square, which is also the
//! resolution of the map the firmware builds. A block is either solid or glass;
//! the laser goes through glass and the sonar does not, which is the whole
//! reason the drone carries both.

use crate::ray::Ray;
use crate::vector::Vec2;

/// What a block is made of.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BlockType {
    /// Nothing there.
    #[default]
    Air,
    /// Solid: both sensors see it.
    Wall,
    /// Glass: only the sonar sees it.
    Window,
}

/// One block of the room.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Block {
    /// The centre, in centimetres.
    pub pos: Vec2,
    /// What it is made of.
    pub kind: BlockType,
    /// The bottom-left corner.
    pub min: Vec2,
    /// The top-right corner.
    pub max: Vec2,
}

impl Block {
    /// How wide a block is, in centimetres.
    pub const SIZE: f64 = 25.0;

    /// A block centred on a point.
    #[must_use]
    pub fn new(pos: Vec2, kind: BlockType) -> Self {
        let half = Self::SIZE / 2.0;

        Self {
            pos,
            kind,
            min: Vec2::new(pos.x - half, pos.y - half),
            max: Vec2::new(pos.x + half, pos.y + half),
        }
    }

    /// Whether a point is inside the block. The bottom-left edges belong to
    /// the block and the top-right ones do not, so neighbouring blocks do not
    /// both claim the boundary.
    #[must_use]
    pub fn contains(&self, point: Vec2) -> bool {
        point.x >= self.min.x
            && point.x < self.max.x
            && point.y >= self.min.y
            && point.y < self.max.y
    }

    /// How far along a ray this block is, in centimetres, or `None` if the ray
    /// misses it or stops short.
    ///
    /// The slab method: clip the ray against the pair of vertical edges and
    /// then the pair of horizontal ones, and see whether anything is left. A
    /// ray exactly parallel to an axis divides by zero, which gives an infinity
    /// for a miss and a not-a-number when the origin sits exactly on the slab.
    /// Rust's `f64::min` and `f64::max` return the other operand rather than
    /// propagating a not-a-number, which is what makes that case come out
    /// right; C++'s `std::min` propagates or not depending on argument order,
    /// so the original was at the mercy of which corner it tested first.
    #[must_use]
    pub fn intersect(&self, ray: &Ray) -> Option<f64> {
        let tx1 = (self.min.x - ray.origin.x) / ray.direction.x;
        let tx2 = (self.max.x - ray.origin.x) / ray.direction.x;
        let ty1 = (self.min.y - ray.origin.y) / ray.direction.y;
        let ty2 = (self.max.y - ray.origin.y) / ray.direction.y;

        let near = tx1.min(tx2).max(ty1.min(ty2));
        let far = tx1.max(tx2).min(ty1.max(ty2));

        // The ray's direction is as long as the ray, so `near` is a fraction of
        // it and scaling it back up gives centimetres.
        (far > near.max(0.0)).then(|| (ray.direction * near).length())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f64::consts::{FRAC_PI_2, PI};

    /// A block a metre east of the origin.
    fn block() -> Block {
        Block::new(Vec2::new(100.0, 0.0), BlockType::Wall)
    }

    #[test]
    fn a_block_covers_its_square() {
        let block = block();
        assert_eq!(block.min, Vec2::new(87.5, -12.5));
        assert_eq!(block.max, Vec2::new(112.5, 12.5));
        assert!(block.contains(Vec2::new(100.0, 0.0)));
        assert!(block.contains(block.min));
        assert!(!block.contains(block.max));
        assert!(!block.contains(Vec2::new(120.0, 0.0)));
    }

    #[test]
    fn a_ray_pointed_at_a_block_reaches_its_near_face() {
        let ray = Ray::new(Vec2::ZERO, 400.0, 0.0);
        let distance = block().intersect(&ray).expect("the ray points at it");

        assert!((distance - 87.5).abs() < 0.5, "{distance}");
    }

    #[test]
    fn a_ray_pointed_away_from_a_block_misses_it() {
        let ray = Ray::new(Vec2::ZERO, 400.0, PI);
        assert_eq!(block().intersect(&ray), None);
    }

    #[test]
    fn a_ray_pointed_past_a_block_misses_it() {
        let ray = Ray::new(Vec2::ZERO, 400.0, FRAC_PI_2);
        assert_eq!(block().intersect(&ray), None);
    }

    #[test]
    fn a_block_further_off_than_the_ray_is_long_still_reports_its_distance() {
        // The ray carries the range: the sensor decides what is out of range,
        // since `Ray::length` is a scale rather than a cut-off.
        let far = Block::new(Vec2::new(1000.0, 0.0), BlockType::Wall);
        let ray = Ray::new(Vec2::ZERO, 400.0, 0.0);
        let distance = far.intersect(&ray).expect("the ray points at it");

        assert!((distance - 987.5).abs() < 0.5, "{distance}");
    }

    #[test]
    fn a_ray_exactly_along_an_edge_does_not_produce_nonsense() {
        // Straight up the left-hand edge of the block: divides by zero on one
        // axis and takes the difference of two zeroes on the other.
        let edge = Block::new(Vec2::new(0.0, 100.0), BlockType::Wall);
        let ray = Ray::new(Vec2::new(-12.5, 0.0), 400.0, FRAC_PI_2);

        match edge.intersect(&ray) {
            None => {}
            Some(distance) => assert!(distance.is_finite(), "{distance}"),
        }
    }
}
