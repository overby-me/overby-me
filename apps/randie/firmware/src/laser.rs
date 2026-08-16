//! The laser range finder.
//!
//! Three beams, left, front and right, reported in centimetres. A reading of
//! [`LASER_MAX_DISTANCE_CM`] means nothing came back: either the room is bigger
//! than the beam reaches, or the beam went through a window.

use crate::io::Tx;

/// How far the module can see. Also what it reports when it sees nothing.
pub const LASER_MAX_DISTANCE_CM: u16 = 2200;

/// The laser module's readings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Laser {
    /// The pin the module is wired to.
    pub pin: Tx,
    /// The latest reading to the left, in centimetres.
    pub val_left: u16,
    /// The latest reading to the right, in centimetres.
    pub val_right: u16,
    /// The latest reading straight ahead, in centimetres.
    pub val_front: u16,
}

impl Laser {
    /// A module on the given pin, with nothing read yet.
    #[must_use]
    pub const fn new(pin: Tx) -> Self {
        Self {
            pin,
            val_left: 0,
            val_right: 0,
            val_front: 0,
        }
    }

    /// Decodes the four bytes the module sends for one reading.
    ///
    /// This reproduces the arithmetic in `laser_read_dist`, which shifts the
    /// bytes by 3, 2, 1 and 0 bits rather than by 24, 16, 8 and 0, and so
    /// cannot reconstruct a four-byte number. There is no module here to check
    /// a corrected version against, and the simulator supplies readings
    /// directly, so the port keeps what the C computes rather than inventing a
    /// wire format.
    #[must_use]
    pub fn decode(bytes: [u8; 4]) -> u16 {
        let mut result: u16 = 0;
        for (index, byte) in bytes.iter().enumerate() {
            result = result.wrapping_add(u16::from(*byte) << (3 - index));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_module_has_read_nothing() {
        let laser = Laser::new(Tx::Tx1);
        assert_eq!(laser.val_front, 0);
        assert_eq!(laser.val_left, 0);
        assert_eq!(laser.val_right, 0);
    }

    #[test]
    fn decoding_matches_the_shifts_the_c_uses() {
        assert_eq!(Laser::decode([1, 0, 0, 0]), 8);
        assert_eq!(Laser::decode([0, 0, 0, 1]), 1);
        assert_eq!(Laser::decode([1, 1, 1, 1]), 15);
    }
}
