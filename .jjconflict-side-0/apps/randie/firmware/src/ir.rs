//! The infrared range finders.
//!
//! Two Sharp-style sensors, one pointing at the floor and one at the ceiling,
//! which is how the drone knows how high it is. They are analogue: the ADC
//! returns a number, and a table turns that into centimetres, because the
//! sensor's response is a curve nobody wants to evaluate on an 8-bit core.
//!
//! The curve doubles back at the near end, which is why the table starts at 19
//! and falls to 14 before jumping to 80: a reading below about 14 cm is
//! indistinguishable from one much further out. The firmware works around this
//! by never flying that low.

use crate::io::APin;

/// The furthest the sensor can see, in centimetres.
pub const IR_MAX_DIST_CM: u8 = 80;
/// The closest reading the firmware will report.
pub const IR_MIN_DIST_CM: u8 = 0;

/// The largest raw reading that still means "as far as I can see".
const MAX_DISTANCE_RAW_VALUE: u16 = 68;
/// The smallest raw reading that still means "as close as I can tell".
const MIN_DISTANCE_RAW_VALUE: u16 = 323;
/// The distance a reading at or below [`MAX_DISTANCE_RAW_VALUE`] stands for.
const MAX_DISTANCE: u8 = 80;
/// The distance a reading at or above [`MIN_DISTANCE_RAW_VALUE`] stands for.
const MIN_DISTANCE: u8 = 14;

/// Raw ADC reading to centimetres.
#[rustfmt::skip]
const IR_TO_CM: [u8; 256] = [
    19, 19, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 18, 17,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 15, 15,
    15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
    14, 14, 14, 14, 80, 79, 78, 76, 75, 74, 73, 72, 71, 70, 69, 68,
    67, 66, 65, 64, 64, 63, 62, 61, 60, 60, 59, 58, 58, 57, 56, 56,
    55, 54, 54, 53, 53, 52, 51, 51, 50, 50, 49, 49, 48, 48, 47, 47,
    46, 46, 46, 45, 45, 44, 44, 43, 43, 43, 42, 42, 41, 41, 41, 40,
    40, 40, 39, 39, 39, 38, 38, 38, 37, 37, 37, 37, 36, 36, 36, 35,
    35, 35, 35, 34, 34, 34, 34, 33, 33, 33, 33, 32, 32, 32, 32, 32,
    31, 31, 31, 31, 30, 30, 30, 30, 30, 30, 29, 29, 29, 29, 29, 28,
    28, 28, 28, 28, 28, 27, 27, 27, 27, 27, 27, 26, 26, 26, 26, 26,
    26, 25, 25, 25, 25, 25, 25, 25, 24, 24, 24, 24, 24, 24, 24, 24,
    23, 23, 23, 23, 23, 23, 23, 23, 22, 22, 22, 22, 22, 22, 22, 22,
    22, 21, 21, 21, 21, 21, 21, 21, 21, 21, 21, 20, 20, 20, 20, 20,
    20, 20, 20, 20, 20, 20, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19,
];

/// One infrared sensor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ir {
    /// The analogue pin the sensor is wired to.
    pub pin: APin,
    /// The latest reading, in centimetres.
    pub value: u8,
}

impl Ir {
    /// A sensor on the given pin, with nothing read yet.
    #[must_use]
    pub const fn new(pin: APin) -> Self {
        Self { pin, value: 0 }
    }

    /// Turns a raw ADC reading into a distance and records it.
    ///
    /// The C read the ADC itself and then threw the answer away: `ir_read`
    /// returned the distance but never stored it, and its one caller ignored
    /// the return value, so `ir->value` stayed at whatever it was. Here the
    /// caller supplies the reading, since there is no ADC, and the result is
    /// both stored and returned.
    pub fn read(&mut self, raw: u16) -> u8 {
        self.value = if raw >= MIN_DISTANCE_RAW_VALUE {
            MIN_DISTANCE
        } else if raw <= MAX_DISTANCE_RAW_VALUE {
            MAX_DISTANCE
        } else {
            IR_TO_CM[usize::from(raw as u8)]
        };

        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ends_of_the_range_are_clamped() {
        let mut ir = Ir::new(APin::A0);
        assert_eq!(ir.read(0), MAX_DISTANCE);
        assert_eq!(ir.read(MAX_DISTANCE_RAW_VALUE), MAX_DISTANCE);
        assert_eq!(ir.read(1000), MIN_DISTANCE);
        assert_eq!(ir.read(MIN_DISTANCE_RAW_VALUE), MIN_DISTANCE);
    }

    #[test]
    fn a_reading_is_stored_as_well_as_returned() {
        let mut ir = Ir::new(APin::A1);
        assert_eq!(ir.read(100), ir.value);
        assert_eq!(ir.value, 53);
    }

    #[test]
    fn the_table_is_monotonic_over_the_useful_range() {
        // Past the fold at index 68 the response falls off cleanly, which is
        // the part the firmware relies on.
        for raw in 69..255 {
            assert!(IR_TO_CM[raw] <= IR_TO_CM[raw - 1]);
        }
    }

    #[test]
    fn every_reading_is_within_the_sensors_range() {
        for distance in IR_TO_CM {
            assert!((IR_MIN_DIST_CM..=IR_MAX_DIST_CM).contains(&distance));
        }
    }
}
