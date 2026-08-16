//! Randie: firmware for an indoor navigation drone.
//!
//! A Rust port of the C firmware at <https://github.com/prozum/randie>. The
//! drone is an ATmega328p bolted to a quadcopter's flight controller, with a
//! three-beam laser range finder, one forward sonar, and an infrared sensor
//! pointing at the floor and another at the ceiling. It flies along walls, and
//! builds a map of the room out of what it bumps into.
//!
//! Everything here is the platform-independent half: the navigator, the map,
//! the sensor models, the filters and the fixed-point arithmetic they run on.
//! It is `no_std`, so it still builds for the board.
//!
//! # What is not here
//!
//! Three parts of the C tree are hardware and stop at the port's edge:
//!
//! - `io-avr.c`, which reads and writes the ATmega's pins, ADC, EEPROM and
//!   UART. The simulator was always built against `io-mock.c` instead, and
//!   what it mocked was mostly ring buffers of pretend pin traffic; the map's
//!   EEPROM backing is in [`map`], and the pin names are in [`io`].
//! - `task.c`, the cyclic executive. It drives the flight controller's four
//!   PWM channels by hand, spinning on a hardware timer to release each edge,
//!   and there is nothing to spin on here. What it schedules is what matters,
//!   and the constants below record it: a 100 ms major cycle of five 20 ms
//!   minor cycles, each opening with a pulse to the flight controller, with
//!   the sonar read in the fourth and the navigator run in the fifth. The
//!   simulator calls [`Firmware::navigation`] on that same 100 ms period.
//! - `gdb.c`, a GDB remote-serial stub for debugging on the board over the
//!   UART, which has no counterpart in a browser.

#![no_std]

extern crate alloc;

pub mod datafusion;
pub mod fc;
pub mod fix16;
pub mod io;
pub mod ir;
pub mod kalman;
pub mod laser;
pub mod log;
pub mod map;
pub mod matrix;
pub mod nav;
pub mod search;
pub mod sonar;

use crate::io::DPin;
use crate::log::Log;
use crate::map::Map;
use crate::nav::{Nav, Rep};

pub use crate::fix16::Fix16;

/// One millisecond, in timer ticks at the board's clock and prescaler. This is
/// also the shortest pulse the flight controller reads, meaning full backwards.
pub const ONE_MS: u16 = 63;
/// A millisecond and a half in ticks: the flight controller's neutral pulse.
pub const ONE_AND_A_HALF_MS: u16 = 94;
/// Two milliseconds in ticks: the flight controller's full-forward pulse.
pub const TWO_MS: u16 = 125;
/// The scheduler's major cycle, 100 ms in ticks. Everything runs once per
/// major cycle.
pub const MAJOR_CYCLE: u16 = 6250;
/// The scheduler's minor cycle, 20 ms in ticks. Five make a major cycle.
pub const MINOR_CYCLE: u16 = 1125;
/// When the scheduler's timer wraps: a millisecond after the major cycle, so
/// the last slot has somewhere to overrun into.
pub const SCHEDULER_OVERFLOW: u16 = MAJOR_CYCLE + ONE_MS;

/// The pin the flight controller's yaw channel is wired to.
pub const YAW_PIN: DPin = DPin::P8;
/// The pin the flight controller's roll channel is wired to.
pub const ROLL_PIN: DPin = DPin::P9;
/// The pin the flight controller's pitch channel is wired to.
pub const PITCH_PIN: DPin = DPin::P10;
/// The pin the flight controller's throttle channel is wired to.
pub const THROTTLE_PIN: DPin = DPin::P11;

/// How long the sonar's echo is waited for, in ticks. Sound covers 4.4 m in
/// that time, comfortably past the 2.2 m of room the sensor can report.
pub const SONAR_TIMEOUT: u16 = 13 * ONE_MS;

/// Everything the firmware owns.
///
/// The C kept these as file-scope pointers in `task.h` that every module
/// reached for. Gathering them into one value is the same arrangement written
/// down, and it lets a caller run two drones at once, which the simulator has
/// no use for but a test does.
#[derive(Clone, Debug)]
pub struct Firmware {
    /// The flight controller and the sensors.
    pub rep: Rep,
    /// Where the drone thinks it is, and what it is doing about it.
    pub nav: Nav,
    /// What it has worked out about the room.
    pub map: Map,
    /// What it has complained about.
    pub log: Log,
}

impl Default for Firmware {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for Map {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Map")
            .field("width", &self.width())
            .field("height", &self.height())
            .finish_non_exhaustive()
    }
}
