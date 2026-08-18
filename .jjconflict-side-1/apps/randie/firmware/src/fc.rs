//! The flight controller.
//!
//! The drone carries a Naze32 Rev6 that does the actual flying. The firmware
//! talks to it the way a radio would: four PWM channels, each held at one of
//! three duty cycles standing for full-back, neutral and full-forward. So
//! "move forward" here means "hold the pitch channel at its maximum", and how
//! fast that turns out to be is the flight controller's business, not ours.
//!
//! The accelerometer and velocity readings come back the other way and are what
//! the navigator dead-reckons from.

use crate::fix16::Fix16;
use crate::io::Tx;
use crate::log::{Log, Sender};

/// The three duty cycles a channel is driven at.
///
/// On the drone these are pulse widths in timer ticks around 1 ms, 1.5 ms and
/// 2 ms. The simulator sets them to 0, 1 and 2 so that a channel reads directly
/// as backwards, still or forwards.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Duty {
    /// Full speed backwards on the axis.
    pub min: u16,
    /// No speed on the axis.
    pub mid: u16,
    /// Full speed ahead on the axis.
    pub max: u16,
}

/// A reading on the three body axes.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Axes {
    /// Left/right.
    pub x: Fix16,
    /// Forwards/backwards.
    pub y: Fix16,
    /// Up/down.
    pub z: Fix16,
}

/// The flight controller, as the firmware sees it.
#[derive(Clone, Debug)]
pub struct Fc {
    /// The duty cycles the channels are held at.
    pub duty: Duty,
    /// The latest accelerometer reading.
    pub acc: Axes,
    /// Velocity, accumulated from acceleration.
    pub vel: Axes,
    /// The serial pin the controller is wired to.
    pub serial: Tx,
    /// Milliseconds since the controller was last scheduled.
    pub deltatime: u16,
    /// Rotation about the vertical axis.
    pub yaw: u16,
    /// Forwards and backwards.
    pub pitch: u16,
    /// Left and right.
    pub roll: u16,
    /// Up and down.
    pub throttle: u16,
    /// Rotational velocity, in radians a second.
    pub gyro: Fix16,
}

impl Fc {
    /// A controller on the given pin, with `ms` as the shortest pulse. The
    /// neutral and full pulses are 1.5 and 2 times that.
    ///
    /// The C left the acceleration and velocity vectors on whatever `malloc`
    /// handed back; here they start at zero.
    #[must_use]
    pub fn new(serial: Tx, ms: u16) -> Self {
        Self {
            duty: Duty {
                min: ms,
                mid: (f32::from(ms) * 1.5) as u16,
                max: ms * 2,
            },
            acc: Axes::default(),
            vel: Axes::default(),
            serial,
            deltatime: 0,
            yaw: 0,
            pitch: 0,
            roll: 0,
            throttle: 0,
            gyro: Fix16::ZERO,
        }
    }

    /// Arms the controller: yaw hard over, throttle down.
    pub fn arm(&mut self) {
        self.yaw = self.duty.max;
        self.throttle = self.duty.min;
        self.pitch = self.duty.mid;
        self.roll = self.duty.mid;
    }

    /// Disarms the controller.
    pub fn disarm(&mut self) {
        self.yaw = self.duty.min;
        self.throttle = self.duty.min;
        self.pitch = self.duty.mid;
        self.roll = self.duty.mid;
    }

    /// Turns anticlockwise about the drone's own axis.
    pub fn rotate_left(&mut self) {
        self.yaw = self.duty.min;
    }

    /// Turns clockwise about the drone's own axis.
    pub fn rotate_right(&mut self) {
        self.yaw = self.duty.max;
    }

    /// Stops turning.
    pub fn rotate_stop(&mut self) {
        self.yaw = self.duty.mid;
    }

    /// Strafes left.
    pub fn move_left(&mut self) {
        self.roll = self.duty.min;
    }

    /// Strafes right.
    pub fn move_right(&mut self) {
        self.roll = self.duty.max;
    }

    /// Flies forward.
    pub fn move_forward(&mut self) {
        self.pitch = self.duty.max;
    }

    /// Flies backwards.
    pub fn move_back(&mut self) {
        self.pitch = self.duty.min;
    }

    /// Climbs.
    pub fn move_up(&mut self) {
        self.throttle = self.duty.max;
    }

    /// Descends.
    pub fn move_down(&mut self) {
        self.throttle = self.duty.min;
    }

    /// Stops on every axis.
    pub fn move_stop(&mut self) {
        self.throttle = self.duty.mid;
        self.pitch = self.duty.mid;
        self.roll = self.duty.mid;
        self.yaw = self.duty.mid;
    }

    /// Sets the acceleration reading. For debugging and for the simulator,
    /// which computes what the accelerometer would have felt.
    pub fn set_acceleration(&mut self, x: f32, y: f32, z: f32) {
        self.acc = Axes {
            x: Fix16::from_f32(x),
            y: Fix16::from_f32(y),
            z: Fix16::from_f32(z),
        };
    }

    /// Sets the velocity. The C wrote its `z` argument into the acceleration
    /// vector instead of the velocity one; this writes all three where they
    /// belong.
    pub fn set_velocity(&mut self, x: f32, y: f32, z: f32) {
        self.vel = Axes {
            x: Fix16::from_f32(x),
            y: Fix16::from_f32(y),
            z: Fix16::from_f32(z),
        };
    }

    /// Accumulates an acceleration over `deltatime` seconds into the velocity.
    pub fn update_velocity(&mut self, acceleration: &Axes, deltatime: f32) {
        let dt = Fix16::from_f32(deltatime);
        self.vel = Axes {
            x: self.vel.x + acceleration.x * dt,
            y: self.vel.y + acceleration.y * dt,
            z: self.vel.z + acceleration.z * dt,
        };
    }

    /// Reads the accelerometer over the serial link.
    ///
    /// Never implemented: the C logs an error and hands back a canned half a
    /// unit of forward acceleration. Kept, so that a caller that reaches for it
    /// finds out the same way.
    pub fn read_acceleration(&self, log: &mut Log) -> Axes {
        log.error(Sender::Fc, "This function is not supported yet.");

        Axes {
            x: Fix16::ZERO,
            y: Fix16::from_f32(0.5),
            z: Fix16::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fc() -> Fc {
        Fc::new(Tx::Tx1, 63)
    }

    #[test]
    fn the_duty_cycles_bracket_the_neutral_one() {
        let fc = fc();
        assert_eq!(fc.duty.min, 63);
        assert_eq!(fc.duty.mid, 94);
        assert_eq!(fc.duty.max, 126);
    }

    #[test]
    fn arming_holds_yaw_over_and_throttle_down() {
        let mut fc = fc();
        fc.arm();
        assert_eq!(fc.yaw, fc.duty.max);
        assert_eq!(fc.throttle, fc.duty.min);
        assert_eq!(fc.pitch, fc.duty.mid);
        assert_eq!(fc.roll, fc.duty.mid);
    }

    #[test]
    fn stopping_neutralizes_every_channel() {
        let mut fc = fc();
        fc.move_forward();
        fc.rotate_right();
        fc.move_up();
        fc.move_stop();

        assert_eq!(fc.pitch, fc.duty.mid);
        assert_eq!(fc.yaw, fc.duty.mid);
        assert_eq!(fc.throttle, fc.duty.mid);
        assert_eq!(fc.roll, fc.duty.mid);
    }

    #[test]
    fn velocity_accumulates_acceleration() {
        let mut fc = fc();
        let acceleration = Axes {
            x: Fix16::ZERO,
            y: Fix16::from_int(10),
            z: Fix16::ZERO,
        };

        fc.update_velocity(&acceleration, 0.5);
        fc.update_velocity(&acceleration, 0.5);

        assert!((fc.vel.y.to_f64() - 10.0).abs() < 0.01);
    }

    #[test]
    fn setting_the_velocity_leaves_the_acceleration_alone() {
        let mut fc = fc();
        fc.set_velocity(1.0, 2.0, 3.0);

        assert!((fc.vel.z.to_f64() - 3.0).abs() < 0.01);
        assert_eq!(fc.acc.z, Fix16::ZERO);
    }
}
