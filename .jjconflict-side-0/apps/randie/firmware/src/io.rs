//! The board's pins, as the firmware names them.
//!
//! The C `io` module is two things at once: a set of names for the ATmega328p's
//! pins, and a hardware abstraction layer with an AVR implementation
//! (`io-avr.c`) and a mock one (`io-mock.c`) that the tests and the simulator
//! were built against.
//!
//! Only the names survive the port. The simulator never drove a real pin: it
//! wrote sensor readings straight into the sensor structs and read the flight
//! controller's duty cycles straight back out, so the mock's ring buffers of
//! pretend pin traffic had nothing to carry. What the pin names still record is
//! the wiring, which is genuine information about the drone, so the sensor
//! structs keep the field.

/// The board's EEPROM, in bytes. The map is stored there, and at four fields
/// to the byte a 64x64 map fills it exactly.
pub const EEPROM_SIZE: usize = 1024;

/// Milliseconds in a second.
pub const MS_PR_SEC: u32 = 1000;

/// Whether a pin is driven or sensed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinMode {
    /// The pin receives input from an external source.
    Input,
    /// The pin transmits output to an external source.
    Output,
}

/// A digital level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DVal {
    Low,
    High,
}

/// A digital pin, numbered as on the Arduino Uno's header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DPin {
    P0,
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
    P7,
    P8,
    P9,
    P10,
    P11,
    P12,
    P13,
}

/// An analog pin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum APin {
    A0,
    A1,
    A2,
    A3,
    A4,
    A5,
}

/// A serial receive pin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rx {
    Rx0,
    UsbRx,
}

/// A serial transmit pin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tx {
    Tx1,
    UsbTx,
}

/// A pin that can carry a PWM signal. The flight controller is driven by four
/// of these.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pwm {
    Pwm3,
    Pwm5,
    Pwm6,
    Pwm9,
    Pwm10,
    Pwm11,
}
