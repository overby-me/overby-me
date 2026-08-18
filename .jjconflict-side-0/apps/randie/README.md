# Randie

<!-- publish:begin -->
> Part of the [overby.me monorepo](https://tangled.org/overby.me/overby.me), where this lives in
> [`apps/randie`](https://tangled.org/overby.me/overby.me/tree/main/apps/randie) and where all development happens.
>
> It is also published on its own, as
> [tangled.org/overby.me/randie](https://tangled.org/overby.me/randie) and
> [github.com/overby-me/randie](https://github.com/overby-me/randie). Both
> are read-only mirrors, rebuilt from the monorepo with
> [josh](https://github.com/josh-project/josh): a commit made to either is
> overwritten by the next sync, so please open issues and pull requests on the
> monorepo.
<!-- publish:end -->

Firmware for an indoor navigation drone, and the simulator it is flown in.

A Rust port of [prozum/randie](https://github.com/prozum/randie), a university
project from 2016: an ATmega328p bolted to a quadcopter's flight controller,
with a three-beam laser range finder, one forward sonar, and infrared sensors
pointing at the floor and the ceiling. It flies along walls, turns at corners,
and builds a map of the room out of what it flies past. The original firmware
is C and its simulator, Randsim, is C++ over SDL2; here both are Rust, and the
simulator's front end is [Dioxus](https://dioxuslabs.com) on a canvas.

The drone never knows where it is. Position comes from integrating the flight
controller's velocity, so it drifts, and the map drifts with it. Watching that
happen is most of what the simulator is for.

## Running it

```bash
just dev              # hot-reloading dev server
just build            # a release bundle in target/dx/randie/release/web/public
just test             # the firmware, the world, and the view onto it
just trace 300        # fly for five minutes and print the map, no browser
just browser          # run the built bundle in headless chromium and check it
```

Drag on the room to draw a wall, drag starting from a wall to rub it out.
Scroll to zoom, arrows or right-drag to pan. `space` runs and pauses, `s`
steps, `r` puts the drone back, `f` reframes the room, `q` and `w` choose
between a wall and a window.

## What is where

| | |
|-|-|
| `firmware/` | The drone's own code: the navigator, the map, the sensor models, the Kalman filters, and the Q16.16 arithmetic they run on. `no_std`, no dependencies, so it still builds for the board. |
| `sim/` | The room: 25 cm blocks, ray-cast sensors, and a flight model that turns the flight controller's four duty cycles into movement. No rendering, so a scenario runs under `cargo test`. |
| `src/` | The page: a canvas the room is drawn on, and a panel of readouts. |
| `sim/maps/default.txt` | The room the simulator opens in. `#` is a wall, `&` a window, a space is floor. |

## How it flies

The navigator runs ten times a second. Each round it reads the sensors into a
set of flags, dead-reckons its position, marks the map, and takes one step of a
state machine:

- **Idle**: nothing in the way, so set off forward.
- **Move forward**: until something is within 60 cm, then turn the corner.
- **Follow forward**: hold a wall on one side and fly along it.
- **Follow further**: the wall fell away, so carry on 50 cm in case it comes
  back.
- **Follow check**: it did not, so turn 90° to point the sonar at whatever is
  there. The laser goes through glass and the sonar does not, so a wall that
  the laser stopped seeing but the sonar still can is a window.
- **Follow turn**: a corner, so turn 90° away from the wall.

`just trace` prints the resulting circuit and the map at the end of it.

## What the port changed

The port follows the C closely, including its quirks. Where it does not, the
module says so at the point of departure. In short:

- **Fixed to keep the port working.** `search.c` does not run at all: it
  dereferences uninitialized pointers, builds its four neighbours by writing
  through the same array element four times, and scans a list with a loop that
  walks the wrong way. Its one caller is an empty function, which is presumably
  why nobody noticed. `search.rs` is the A\* that module describes.
- **Fixed because the sibling module shows the intent.** The data-fusion filter
  updated its covariance from the state rather than from the covariance, and
  subtracted its gain from a raw `1`, the integer, which is 0.000015 in
  Q16.16. The single-sensor filter beside it writes both lines correctly.
- **Fixed because it was plumbing, not modelling.** An infrared reading that
  was computed and then not stored; a velocity setter that wrote its third
  argument into the acceleration; an acceleration computed by dividing by an
  integer that was always zero; map writes that could land one row up.
- **Kept, because changing it would change what the drone does.** The map is
  drawn from the side-facing lasers only: the branch that would record what is
  in front computes the answer and then discards it, both of its arms commented
  out. The sonar's reliability check is never called. The alignment routine is
  never called and turns through a distance in centimetres read as radians.
- **Bounded, because firmware should not hang.** Both filters' calibration
  loops ran until they converged, with nothing to stop them if they did not.

The simulator's front end is a rewrite rather than a transcription, since SDL2
and a canvas have little in common, but it draws the same things in the same
colours. Three differences are deliberate: the view frames the room instead of
opening on a fixed offset that assumed a 1000-pixel window; zooming keeps the
point under the cursor where it is; and each laser beam is drawn as far as its
reading rather than at full length through whatever it hit.

## What is not ported

Three parts of the C tree are hardware, and stop at the port's edge:

- `io-avr.c`, which drives the ATmega's pins, ADC, EEPROM and UART. The
  simulator was always built against the mock instead. What the mock had that
  mattered (the EEPROM the map lives in) is in `firmware/src/map.rs`.
- `task.c`, the cyclic executive. It bit-bangs the flight controller's four PWM
  channels by spinning on a hardware timer, and there is no timer here. What it
  schedules is recorded in `firmware/src/lib.rs`, and the simulator runs the
  navigator on the same 100 ms period.
- `gdb.c`, a GDB stub for debugging over the board's serial port.

The vendored copies of libfixmath and simavr are not ported either: the first
is reimplemented in `firmware/src/fix16.rs` in the configuration the project
compiled it with, and the second is an AVR emulator for tests that ran the real
firmware image.