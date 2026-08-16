//! The ultrasonic range finder.
//!
//! One forward-facing sensor with a wide cone. It sees what the laser cannot,
//! a window, and the navigator compares the two to tell glass from wall. The
//! cone is also why it is only trusted at certain angles: a wide beam that
//! catches a wall off to one side reads shorter than the wall in front.

use crate::fix16::Fix16;
use crate::io::DPin;

/// The shortest pulse worth believing, in timer ticks. Below this the echo
/// comes back before the sensor has stopped talking, about 2 cm out.
pub const MIN_OUTPUT: u16 = 110;

/// The sonar's reading.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sonar {
    /// The pin the ping goes out on.
    pub trig: DPin,
    /// The pin the echo comes back on.
    pub echo: DPin,
    /// Whether the latest reading is worth anything: an echo did come back
    /// before the timeout.
    pub valid: bool,
    /// The latest reading, in centimetres.
    pub value: u16,
}

impl Sonar {
    /// A sonar on the given trigger and echo pins.
    #[must_use]
    pub const fn new(trig: DPin, echo: DPin) -> Self {
        Self {
            trig,
            echo,
            valid: false,
            value: 0,
        }
    }

    /// Converts a round-trip time in milliseconds to a distance in
    /// centimetres. Sound covers 34.32 cm a millisecond and makes the trip
    /// twice, so the distance is half of what it travelled.
    #[must_use]
    pub fn to_centimeters(millis: u16) -> u16 {
        let speed_of_sound = Fix16::from_f32(34.32);
        let half = Fix16::from_f32(0.5);

        (Fix16::from_int(i32::from(millis)) * speed_of_sound * half).to_int() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_sonar_has_nothing_to_say() {
        let sonar = Sonar::new(DPin::P2, DPin::P3);
        assert!(!sonar.valid);
        assert_eq!(sonar.value, 0);
    }

    #[test]
    fn a_millisecond_of_flight_is_about_seventeen_centimetres() {
        assert_eq!(Sonar::to_centimeters(1), 17);
        assert_eq!(Sonar::to_centimeters(0), 0);
        assert_eq!(Sonar::to_centimeters(10), 172);
    }

    #[test]
    fn the_far_end_of_the_range_stays_in_range() {
        // The scheduler gives the echo 13 ms, which is about 4.4 m of travel
        // and so about 2.2 m of room.
        assert_eq!(Sonar::to_centimeters(13), 223);
    }
}
