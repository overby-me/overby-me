//! The drone: where it really is, and how the flight controller moves it.
//!
//! The firmware sets four duty cycles and believes the flight controller does
//! the rest. Here that belief is made good: each channel becomes a speed, each
//! speed becomes a distance over the tick, and the accelerometer and gyro are
//! filled back in with what the drone would have felt. The drone's real
//! position is never shown to the firmware.

use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

use randie_firmware::Firmware;
use randie_firmware::fix16::Fix16;
use randie_firmware::io::MS_PR_SEC;
use randie_firmware::ir::{IR_MAX_DIST_CM, IR_MIN_DIST_CM};

use crate::block::Block;
use crate::sensors::{
    LASER_LENGTH, SONAR_LENGTH, SONAR_RAYS, SONAR_SPAN_DEGREES, SimLaser, SimSonar,
};
use crate::vector::Vec2;

/// How fast the drone turns at full yaw, in radians a second.
pub const ROTATION_SPEED: f64 = FRAC_PI_4;
/// How fast it strafes at full roll, in centimetres a second.
pub const STRAFE_SPEED: f64 = 20.0;
/// How fast it flies at full pitch, in centimetres a second.
pub const MOVEMENT_SPEED: f64 = 100.0;
/// How fast it climbs at full throttle, in centimetres a second.
pub const ALTITUDE_SPEED: f64 = 10.0;
/// The duty cycle that means "hold still". The simulator drives the flight
/// controller with 0, 1 and 2 rather than pulse widths, so a channel reads
/// directly as backwards, still or forwards.
pub const FC_OFFSET: f64 = 1.0;
/// How often the navigator runs, in milliseconds. The same period the
/// scheduler gives it on the board.
pub const NAV_UPDATE_TIME: u32 = 100;
/// How high the room is, in centimetres.
pub const ROOM_HEIGHT: f64 = 300.0;

/// How wide the drone is, in centimetres.
pub const DRONE_SIZE: f64 = 50.0;

/// The drone.
#[derive(Clone, Debug)]
pub struct Drone {
    /// Where it really is, in centimetres.
    pub pos: Vec2,
    /// Which way it really points, in radians anticlockwise from east.
    pub angle: f64,
    /// How wide it is, in centimetres.
    pub size: f64,
    /// How high it really is, in centimetres off the floor.
    pub height: f64,
    /// The forward sonar.
    pub sonar: SimSonar,
    /// The three-beam laser.
    pub laser: SimLaser,
    /// The firmware flying it.
    pub firmware: Firmware,
    /// When the navigator last ran, in milliseconds of simulated time.
    last_nav_update: u32,
}

impl Drone {
    /// A drone at rest.
    #[must_use]
    pub fn new(pos: Vec2, size: f64) -> Self {
        let mut firmware = Firmware::new();

        // Duty cycles of 0, 1 and 2 rather than the board's pulse widths, so
        // that a channel reads directly as a direction.
        firmware.rep.fc.duty.min = 0;
        firmware.rep.fc.duty.mid = 1;
        firmware.rep.fc.duty.max = 2;
        firmware.rep.fc.rotate_stop();
        firmware.rep.fc.move_stop();

        // On the floor, so the downward sensor reads its longest.
        firmware.rep.ir_bottom.value = IR_MAX_DIST_CM;

        Self {
            pos,
            angle: 0.0,
            size,
            height: 0.0,
            sonar: SimSonar::new(
                pos,
                0.0,
                SONAR_RAYS,
                SONAR_SPAN_DEGREES.to_radians(),
                SONAR_LENGTH,
            ),
            laser: SimLaser::new(pos, 0.0, LASER_LENGTH),
            firmware,
            last_nav_update: 0,
        }
    }

    /// One tick: read the sensors, run the navigator if it is due, and fly
    /// wherever the flight controller has been left pointing.
    pub fn update(&mut self, blocks: &[Block], time: u32, delta_time: u32) {
        self.sonar
            .update(self.pos, self.angle, blocks, &mut self.firmware.rep.sonar);
        self.laser
            .update(self.pos, self.angle, blocks, &mut self.firmware.rep.laser);

        if time.wrapping_sub(self.last_nav_update) >= NAV_UPDATE_TIME {
            self.firmware.navigation();
            self.last_nav_update = time;
        }

        self.update_from_fc(delta_time);
    }

    /// Puts the drone back where it started, with a blank map and a firmware
    /// that has forgotten everything.
    pub fn reset(&mut self, pos: Vec2) {
        let size = self.size;
        *self = Self::new(pos, size);
    }

    /// What a duty cycle means in centimetres or radians a second.
    fn velocity(duty: u16, speed: f64) -> f64 {
        (f64::from(duty) - FC_OFFSET) * speed
    }

    /// How far that gets in one tick.
    fn distance(speed: f64, delta_time: u32) -> f64 {
        speed * (f64::from(delta_time) / f64::from(MS_PR_SEC))
    }

    /// What the accelerometer would have felt over one tick.
    ///
    /// The C divided by `DeltaTime / MS_PR_SEC` with both sides integers,
    /// which is zero for any tick shorter than a second, and then divided by
    /// it -- so every acceleration it recorded was an infinity. It also read
    /// the previous velocity out of the fixed-point word without converting
    /// it, comparing a raw 1310720 against a plain 20. Neither showed, because
    /// nothing reads the accelerometer: the navigator works from the velocity
    /// and the gyro. Both are done properly here.
    fn acceleration(previous: f64, current: f64, delta_time: u32) -> f64 {
        (current - previous) / (f64::from(delta_time) / f64::from(MS_PR_SEC))
    }

    fn update_from_fc(&mut self, delta_time: u32) {
        self.update_yaw(delta_time);
        self.update_pitch(delta_time);
        self.update_roll(delta_time);
        self.update_throttle(delta_time);
    }

    /// Turns on the spot. A yaw duty above neutral turns the drone clockwise,
    /// which is a falling angle.
    fn update_yaw(&mut self, delta_time: u32) {
        let velocity = Self::velocity(self.firmware.rep.fc.yaw, ROTATION_SPEED);
        let turned = Self::distance(velocity, delta_time);

        self.angle -= turned;
        if self.angle >= TAU {
            self.angle -= TAU;
        } else if self.angle <= 0.0 {
            self.angle += TAU;
        }

        self.firmware.rep.fc.gyro = Fix16::from_f64(-velocity);
    }

    /// Flies forwards or backwards.
    fn update_pitch(&mut self, delta_time: u32) {
        let velocity = Self::velocity(self.firmware.rep.fc.pitch, MOVEMENT_SPEED);
        let flown = Self::distance(velocity, delta_time);

        self.pos += Vec2::from_polar(flown, self.angle);

        let previous = self.firmware.rep.fc.vel.y.to_f64();
        self.firmware.rep.fc.acc.y =
            Fix16::from_f64(Self::acceleration(previous, velocity, delta_time));
        self.firmware.rep.fc.vel.y = Fix16::from_f64(velocity);
    }

    /// Strafes left or right, which is ninety degrees off the heading.
    fn update_roll(&mut self, delta_time: u32) {
        let velocity = Self::velocity(self.firmware.rep.fc.roll, STRAFE_SPEED);
        let strafed = Self::distance(velocity, delta_time);

        self.pos += Vec2::from_polar(strafed, self.angle + FRAC_PI_2);

        let previous = self.firmware.rep.fc.vel.x.to_f64();
        self.firmware.rep.fc.acc.x =
            Fix16::from_f64(Self::acceleration(previous, velocity, delta_time));
        self.firmware.rep.fc.vel.x = Fix16::from_f64(velocity);
    }

    /// Climbs or descends, and tells the two infrared sensors about it.
    fn update_throttle(&mut self, delta_time: u32) {
        let velocity = Self::velocity(self.firmware.rep.fc.throttle, ALTITUDE_SPEED);
        let climbed = Self::distance(velocity, delta_time);

        self.height += climbed;

        self.firmware.rep.ir_bottom.value = clamp_to_sensor(self.height);
        self.firmware.rep.ir_top.value = clamp_to_sensor(ROOM_HEIGHT - self.height);

        let previous = self.firmware.rep.fc.vel.z.to_f64();
        self.firmware.rep.fc.acc.z =
            Fix16::from_f64(Self::acceleration(previous, velocity, delta_time));
        self.firmware.rep.fc.vel.z = Fix16::from_f64(velocity);
    }
}

/// What an infrared sensor reads at a given distance.
///
/// The C converted the distance to a `uint16_t` before comparing, which is
/// undefined for the negative distance a drone driven below the floor
/// produces, and in practice wrapped to something enormous that read as the
/// far end of the range. Clamping first keeps a drone underground reading zero,
/// which is at least the truth about what is under it.
fn clamp_to_sensor(distance: f64) -> u8 {
    let distance = distance.clamp(0.0, f64::from(u16::MAX)) as u16;

    if distance >= u16::from(IR_MAX_DIST_CM) {
        IR_MAX_DIST_CM
    } else if distance < u16::from(IR_MIN_DIST_CM) {
        IR_MIN_DIST_CM
    } else {
        distance as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drone() -> Drone {
        Drone::new(Vec2::ZERO, DRONE_SIZE)
    }

    #[test]
    fn a_new_drone_sits_still() {
        let mut drone = drone();
        let start = drone.pos;

        for _ in 0..10 {
            drone.update_from_fc(10);
        }

        assert_eq!(drone.pos, start);
        assert_eq!(drone.height, 0.0);
    }

    #[test]
    fn full_pitch_flies_it_forward_at_the_stated_speed() {
        let mut drone = drone();
        drone.firmware.rep.fc.move_forward();

        // A hundred ticks of ten milliseconds is a second, at a metre a second.
        for _ in 0..100 {
            drone.update_from_fc(10);
        }

        assert!(
            (drone.pos.x - MOVEMENT_SPEED).abs() < 0.001,
            "{:?}",
            drone.pos
        );
        assert!(drone.pos.y.abs() < 0.001);
    }

    #[test]
    fn full_roll_strafes_it_sideways() {
        let mut drone = drone();
        drone.firmware.rep.fc.move_right();

        for _ in 0..100 {
            drone.update_from_fc(10);
        }

        // Rolling right at a heading of zero moves it north, since the C takes
        // the orthogonal by adding ninety degrees.
        assert!(drone.pos.y.abs() > 0.0);
        assert!(drone.pos.x.abs() < 0.001);
    }

    #[test]
    fn full_yaw_turns_it_and_fills_in_the_gyro() {
        let mut drone = drone();
        drone.firmware.rep.fc.rotate_right();

        drone.update_from_fc(10);

        // Turning right is a falling angle, which wraps round through a turn.
        assert!(drone.angle > TAU - 0.01);
        assert!(drone.firmware.rep.fc.gyro.to_f64() < 0.0);
    }

    #[test]
    fn the_throttle_moves_it_between_the_floor_and_the_ceiling() {
        let mut drone = drone();
        drone.firmware.rep.fc.move_up();

        // Ten centimetres a second for four seconds.
        for _ in 0..400 {
            drone.update_from_fc(10);
        }

        assert!((drone.height - 40.0).abs() < 0.001);
        assert_eq!(drone.firmware.rep.ir_bottom.value, 40);
        // The ceiling is three metres up, well past what the sensor reads.
        assert_eq!(drone.firmware.rep.ir_top.value, IR_MAX_DIST_CM);
    }

    #[test]
    fn a_drone_below_the_floor_reads_zero_rather_than_the_far_end() {
        assert_eq!(clamp_to_sensor(-5.0), 0);
        assert_eq!(clamp_to_sensor(0.0), 0);
        assert_eq!(clamp_to_sensor(40.0), 40);
        assert_eq!(clamp_to_sensor(1000.0), IR_MAX_DIST_CM);
    }

    #[test]
    fn the_navigator_runs_on_its_own_period() {
        let mut drone = drone();
        let blocks = [];

        // Ninety milliseconds is not yet a navigator period.
        for tick in 0..9 {
            drone.update(&blocks, tick * 10, 10);
        }
        assert_eq!(drone.firmware.nav.task, randie_firmware::nav::Task::Idle);

        // The hundredth millisecond is.
        drone.update(&blocks, 100, 10);
        assert_ne!(drone.firmware.nav.task, randie_firmware::nav::Task::Idle);
    }
}
