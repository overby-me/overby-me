//! GameApp: "Radikal Rally", a side-scrolling campaign minigame (`?app=game`).
//!
//! Drive the party's electric car from the suburbs to Christiansborg, hanging
//! election posters on the way. Every mechanic is a party position turned into
//! play: the battery is the green transition (collect power, solar-charge in
//! the sunbeams), border barriers are jumped or lifted by an EU flag (free
//! movement), rainbow strips give a free speed boost, and the ending is an
//! election night whose mood follows how many posters went up. Sprites live in
//! `assets/game/`; design notes in `docs/game.md`.
//!
//! The module splits into a pure `World` (course layout, physics, battery,
//! events) that unit-tests on the host, and a wasm-only layer: sprite cache,
//! canvas-2D renderer, particles and the `GameApp` component.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

use crate::i18n::{t, t_with};

const WORLD_H: f64 = 540.0;
/// Grass band top; the visual horizon every background layer sits on.
const HORIZON: f64 = 438.0;
const ROAD_TOP: f64 = 470.0;
/// Wheel line: where car, barriers and poster legs meet the road.
const BASE_Y: f64 = 511.0;

const CAR_W: f64 = 150.0;
const CAR_H: f64 = 95.0;
const MAX_SPEED: f64 = 430.0;
const REVERSE_MAX: f64 = 260.0;
const ACCEL: f64 = 520.0;
const DRAG: f64 = 260.0;
const GRAVITY: f64 = 1500.0;
const JUMP_VY: f64 = 760.0;
const JUMP_COST: f64 = 2.0;
/// Battery drain per second at full speed. Sized so the course cannot be done
/// on the starting charge alone: refusing to stop for green power loses.
const DRAIN_FULL_SPEED: f64 = 5.5;
/// Drain while standing still: without it a poster detour cost nothing, and
/// the battery only ever measured distance.
const DRAIN_IDLE: f64 = 0.9;
const SUN_CHARGE: f64 = 5.0;
const BATTERY_GAIN: f64 = 35.0;
const BOOST_SPEED: f64 = 660.0;
const BOOST_S: f64 = 1.8;
/// Altitude the car bottom must hold while over a barrier to clear it. Paired
/// with `JUMP_VY`/`GRAVITY` so a full-speed jump clears with margin while a
/// crawling hop clips the arm: momentum is part of the trick.
const BARRIER_CLEAR_ALT: f64 = 118.0;
/// Half-width of the barrier collision wall plus the car span that matters
/// (wheelbase, not the full sprite, so the drooping tail cannot snag the arm).
const BARRIER_OVERLAP: f64 = 71.0;
const PICKUP_RANGE: f64 = 75.0;
const POSTER_RANGE: f64 = 110.0;
/// How far above or below the car a poster or pickup can still be reached.
/// Every height in the course is chosen against these two.
const POSTER_REACH_V: f64 = 55.0;
const PICKUP_REACH_V: f64 = 70.0;
const FINISH_X: f64 = 15200.0;
const WORLD_RIGHT: f64 = 16400.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tier {
    Landslide,
    Elected,
    BelowThreshold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Intro,
    Playing,
    Won(Tier),
    Lost,
}

/// One tick's worth of things the UI should react to (toasts, particles).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Ev {
    Battery,
    Flag,
    BarrierOpen,
    Border,
    Rainbow,
    Sun,
    Poster,
    Won(Tier),
    Lost,
}

#[derive(Default, Clone, Copy)]
struct Input {
    left: bool,
    right: bool,
    /// Edge-triggered: true for the single tick after the key went down.
    jump: bool,
    action: bool,
}

struct Pickup {
    x: f64,
    /// Height above the road. Non-zero puts it on a platform deck, out of
    /// reach of a car driving underneath.
    alt: f64,
    taken: bool,
}

struct Barrier {
    x: f64,
    /// 0 closed to 1 fully raised; animated once an EU flag triggers it.
    lift: f64,
    opening: bool,
    /// Suppresses repeat toasts while the car grinds against the arm.
    hit_cool: f64,
}

/// What a poster spot is fixed to. Only the drawing differs; reaching one is
/// the same rule everywhere (`POSTER_REACH_V` around the car).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpotKind {
    Board,
    /// Panel up a light pole: only in reach near the top of a jump.
    Lamp,
    /// On a city platform, so the car has to be parked up there.
    Ledge,
}

struct Spot {
    x: f64,
    alt: f64,
    kind: SpotKind,
    hung: bool,
}

/// A city ledge the car can land on. One-way: the car passes up through the
/// deck and settles on it coming down, so a platform never traps anyone
/// underneath.
struct Platform {
    x0: f64,
    x1: f64,
    top: f64,
}

struct Strip {
    x0: f64,
    x1: f64,
    cool: f64,
}

struct World {
    phase: Phase,
    time_s: f64,
    car_x: f64,
    vx: f64,
    /// Height of the car bottom above the road; 0 is the road itself.
    alt: f64,
    vy: f64,
    /// Resting on a surface (road or platform deck), so a jump is available.
    grounded: bool,
    battery: f64,
    boost_s: f64,
    flag_held: bool,
    hung: usize,
    batteries: Vec<Pickup>,
    flags: Vec<Pickup>,
    barriers: Vec<Barrier>,
    spots: Vec<Spot>,
    platforms: Vec<Platform>,
    rainbows: Vec<Strip>,
    sun: Vec<(f64, f64)>,
    in_sun: bool,
}

/// Whether a car at (`cx`, `calt`) can hang this spot's poster. Shared with the
/// renderer so the highlight marks exactly what the action key would take.
fn spot_in_reach(s: &Spot, cx: f64, calt: f64) -> bool {
    (s.x - cx).abs() < POSTER_RANGE && (s.alt - calt).abs() < POSTER_REACH_V
}

impl World {
    /// The course, left to right, in five stretches that each add one demand:
    /// suburbs teach the jump, town adds light poles, the city adds platforms,
    /// the harbour tightens the charge, and the approach stacks a pole above a
    /// deck. Every platform top is under the standing-jump apex so the road can
    /// reach it, and the one pole above that height sits over a deck to jump
    /// from.
    fn new() -> Self {
        let ground = |x: f64| Pickup {
            x,
            alt: 0.0,
            taken: false,
        };
        let up = |(x, alt): (f64, f64)| Pickup {
            x,
            alt,
            taken: false,
        };
        let barrier = |x: f64| Barrier {
            x,
            lift: 0.0,
            opening: false,
            hit_cool: 0.0,
        };
        let spot = |(x, alt, kind): (f64, f64, SpotKind)| Spot {
            x,
            alt,
            kind,
            hung: false,
        };
        let platform = |(x0, x1, top): (f64, f64, f64)| Platform { x0, x1, top };
        let strip = |(x0, x1): (f64, f64)| Strip { x0, x1, cool: 0.0 };
        World {
            phase: Phase::Intro,
            time_s: 0.0,
            car_x: 140.0,
            vx: 0.0,
            alt: 0.0,
            vy: 0.0,
            grounded: true,
            battery: 100.0,
            boost_s: 0.0,
            flag_held: false,
            hung: 0,
            batteries: [1900.0, 3450.0, 8200.0, 10900.0, 12000.0, 14450.0]
                .map(ground)
                .into_iter()
                .chain(
                    [
                        (5050.0, 130.0),
                        (6750.0, 145.0),
                        (9700.0, 135.0),
                        (13250.0, 160.0),
                    ]
                    .map(up),
                )
                .collect(),
            flags: [3800.0, 9100.0, 11750.0].map(ground).into(),
            // The first barrier has no flag before it, so it teaches the jump.
            barriers: [2150.0, 4400.0, 7000.0, 10100.0, 12200.0]
                .map(barrier)
                .into(),
            spots: [
                (700.0, 0.0, SpotKind::Board),
                (1500.0, 0.0, SpotKind::Board),
                (2900.0, 160.0, SpotKind::Lamp),
                (3300.0, 0.0, SpotKind::Board),
                (4900.0, 130.0, SpotKind::Ledge),
                (5900.0, 0.0, SpotKind::Board),
                (6550.0, 145.0, SpotKind::Ledge),
                (7400.0, 165.0, SpotKind::Lamp),
                (8600.0, 0.0, SpotKind::Board),
                (9550.0, 135.0, SpotKind::Ledge),
                (10600.0, 160.0, SpotKind::Lamp),
                (11450.0, 150.0, SpotKind::Ledge),
                (12600.0, 0.0, SpotKind::Board),
                // Out of reach from the road: jump from the deck below it.
                (13350.0, 285.0, SpotKind::Lamp),
            ]
            .map(spot)
            .into(),
            platforms: [
                (4650.0, 5150.0, 130.0),
                (6350.0, 6800.0, 145.0),
                (9300.0, 9800.0, 135.0),
                (11200.0, 11650.0, 150.0),
                (13100.0, 13520.0, 160.0),
            ]
            .map(platform)
            .into(),
            rainbows: [
                (1150.0, 1400.0),
                (5350.0, 5600.0),
                (8800.0, 9050.0),
                (14000.0, 14400.0),
            ]
            .map(strip)
            .into(),
            sun: [(5750.0, 6650.0), (12550.0, 13050.0)].into(),
            in_sun: false,
        }
    }

    fn start(&mut self) {
        *self = World::new();
        self.phase = Phase::Playing;
    }

    /// A share rather than a count, so the thresholds survive a change to the
    /// course length.
    fn tier(&self) -> Tier {
        let total = self.spots.len();
        if self.hung * 100 >= total * 85 {
            Tier::Landslide
        } else if self.hung * 100 >= total * 50 {
            Tier::Elected
        } else {
            Tier::BelowThreshold
        }
    }

    /// Highest surface the car can settle on at `x`, given where it was before
    /// this step's fall. A deck only catches a car coming down onto it
    /// (`alt_prev` at or above the top), which is what lets one be jumped
    /// through from below.
    fn support_at(&self, x: f64, alt_prev: f64) -> f64 {
        let mut best = 0.0;
        for p in &self.platforms {
            if x >= p.x0 && x <= p.x1 && p.top > best && alt_prev >= p.top - 1.0 {
                best = p.top;
            }
        }
        best
    }

    fn tick(&mut self, dt: f64, inp: Input) -> Vec<Ev> {
        let mut evs = Vec::new();
        if self.phase != Phase::Playing {
            return evs;
        }
        self.time_s += dt;
        let powered = self.battery > 0.0;

        if self.boost_s > 0.0 {
            self.boost_s -= dt;
            self.vx = (self.vx + 900.0 * dt).min(BOOST_SPEED);
        } else {
            let mut ax = 0.0;
            if powered && inp.right {
                ax += ACCEL;
            }
            if powered && inp.left {
                ax -= ACCEL;
            }
            if ax == 0.0 {
                let drag = DRAG * dt;
                self.vx -= self.vx.clamp(-drag, drag);
            } else {
                self.vx = (self.vx + ax * dt).clamp(-REVERSE_MAX, MAX_SPEED);
            }
            // A boost may have left vx above the cap; bleed it off gently.
            if self.vx > MAX_SPEED {
                self.vx = (self.vx - 500.0 * dt).max(MAX_SPEED);
            }
        }

        if inp.jump && self.grounded && powered {
            self.vy = JUMP_VY;
            self.battery -= JUMP_COST;
            self.grounded = false;
        }
        let alt_prev = self.alt;
        self.vy -= GRAVITY * dt;
        self.alt += self.vy * dt;
        let support = self.support_at(self.car_x, alt_prev);
        self.grounded = self.alt <= support;
        if self.grounded {
            self.alt = support;
            self.vy = 0.0;
        }

        let old_x = self.car_x;
        self.car_x = (self.car_x + self.vx * dt).clamp(CAR_W / 2.0, FINISH_X + 60.0);

        for b in &mut self.barriers {
            b.hit_cool = (b.hit_cool - dt).max(0.0);
            if self.flag_held && !b.opening && b.x - self.car_x > 0.0 && b.x - self.car_x < 320.0 {
                b.opening = true;
                self.flag_held = false;
                evs.push(Ev::BarrierOpen);
            }
            if b.opening && b.lift < 1.0 {
                b.lift = (b.lift + dt / 0.8).min(1.0);
            }
            let blocked = b.lift < 0.4 && self.alt < BARRIER_CLEAR_ALT;
            if blocked && (self.car_x - b.x).abs() < BARRIER_OVERLAP {
                if old_x <= b.x {
                    self.car_x = b.x - BARRIER_OVERLAP;
                    if self.vx > 60.0 && b.hit_cool <= 0.0 {
                        b.hit_cool = 1.0;
                        self.battery -= 2.0;
                        evs.push(Ev::Border);
                    }
                    self.vx = -140.0;
                } else {
                    // Came down past the wall (the arm's far slope): scrape over
                    // rather than bounce back through a barrier already beaten.
                    self.car_x = b.x + BARRIER_OVERLAP;
                }
            }
        }

        let (cx, calt) = (self.car_x, self.alt);
        let in_reach =
            |p: &Pickup| (p.x - cx).abs() < PICKUP_RANGE && (p.alt - calt).abs() < PICKUP_REACH_V;
        for p in &mut self.batteries {
            if !p.taken && in_reach(p) {
                p.taken = true;
                self.battery = (self.battery + BATTERY_GAIN).min(100.0);
                evs.push(Ev::Battery);
            }
        }
        // Holding a flag already, leave the next one standing for later.
        if !self.flag_held {
            for p in &mut self.flags {
                if !p.taken && in_reach(p) {
                    p.taken = true;
                    self.flag_held = true;
                    evs.push(Ev::Flag);
                    break;
                }
            }
        }

        for s in &mut self.rainbows {
            s.cool = (s.cool - dt).max(0.0);
            // Painted on the road, so a car up on a deck passes over them.
            if self.alt <= 0.0 && self.car_x >= s.x0 && self.car_x <= s.x1 && s.cool <= 0.0 {
                s.cool = 3.0;
                self.boost_s = BOOST_S;
                evs.push(Ev::Rainbow);
            }
        }

        let inside_sun = self
            .sun
            .iter()
            .any(|&(a, b)| self.car_x >= a && self.car_x <= b);
        if inside_sun {
            self.battery = (self.battery + SUN_CHARGE * dt).min(100.0);
            if !self.in_sun {
                evs.push(Ev::Sun);
            }
        }
        self.in_sun = inside_sun;

        if inp.action {
            let near = self
                .spots
                .iter_mut()
                .filter(|s| !s.hung && spot_in_reach(s, cx, calt))
                .min_by(|a, b| (a.x - cx).abs().total_cmp(&(b.x - cx).abs()));
            if let Some(s) = near {
                s.hung = true;
                self.hung += 1;
                evs.push(Ev::Poster);
            }
        }

        // The boost is the point of the rainbow: distance that costs nothing.
        self.battery -= DRAIN_IDLE * dt;
        if self.boost_s <= 0.0 {
            self.battery -= DRAIN_FULL_SPEED * (self.vx.abs() / MAX_SPEED) * dt;
        }
        self.battery = self.battery.clamp(0.0, 100.0);

        if self.car_x + CAR_W / 2.0 >= FINISH_X {
            let tier = self.tier();
            self.phase = Phase::Won(tier);
            self.vx = 0.0;
            evs.push(Ev::Won(tier));
        } else if self.battery <= 0.0 && self.grounded && self.vx.abs() < 8.0 {
            self.phase = Phase::Lost;
            evs.push(Ev::Lost);
        }
        evs
    }
}

/// Phase mirrored into a signal so the DOM overlays re-render only on change,
/// while the canvas repaints every frame without touching Dioxus.
#[derive(Clone, Copy, PartialEq)]
enum UiPhase {
    Intro,
    Playing,
    Won(Tier, usize, usize),
    Lost,
}

fn ui_phase(w: &World) -> UiPhase {
    match w.phase {
        Phase::Intro => UiPhase::Intro,
        Phase::Playing => UiPhase::Playing,
        Phase::Won(t) => UiPhase::Won(t, w.hung, w.spots.len()),
        Phase::Lost => UiPhase::Lost,
    }
}

/// Held keys and unconsumed edges, shared between event handlers and the loop.
#[derive(Default)]
struct Held {
    left: bool,
    right: bool,
    jump_down: bool,
    act_down: bool,
    jump_edge: bool,
    act_edge: bool,
}

impl Held {
    fn snapshot(&mut self) -> Input {
        let inp = Input {
            left: self.left,
            right: self.right,
            jump: self.jump_edge,
            action: self.act_edge,
        };
        self.jump_edge = false;
        self.act_edge = false;
        inp
    }
}

struct Particle {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    ttl: f64,
    age: f64,
    size: f64,
    grav: f64,
    color: &'static str,
}

struct Toast {
    text: String,
    x: f64,
    age: f64,
}

const RAINBOW: [&str; 6] = [
    "#E40303", "#FF8C00", "#FFED00", "#008026", "#24408E", "#732982",
];

/// Purely visual state: particles, toasts, and a tiny self-contained RNG.
struct Fx {
    parts: Vec<Particle>,
    toasts: Vec<Toast>,
    rng: u64,
    firework_s: f64,
}

impl Fx {
    fn new() -> Self {
        Fx {
            parts: Vec::new(),
            toasts: Vec::new(),
            rng: 0x1234_5678_9abc_def0,
            firework_s: 0.0,
        }
    }

    fn rand(&mut self) -> f64 {
        self.rng = self
            .rng
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.rng >> 33) as f64) / f64::from(1u32 << 31)
    }

    fn toast(&mut self, text: String, x: f64) {
        self.toasts.push(Toast { text, x, age: 0.0 });
    }

    fn burst(&mut self, x: f64, y: f64, n: usize, speed: f64, colors: &[&'static str]) {
        for _ in 0..n {
            let a = self.rand() * std::f64::consts::TAU;
            let v = speed * (0.35 + 0.65 * self.rand());
            let color = colors[(self.rand() * colors.len() as f64) as usize % colors.len()];
            let ttl = 0.9 + self.rand() * 0.8;
            let size = 3.0 + self.rand() * 4.0;
            self.parts.push(Particle {
                x,
                y,
                vx: a.cos() * v,
                vy: a.sin() * v * 0.8,
                ttl,
                age: 0.0,
                size,
                grav: 350.0,
                color,
            });
        }
    }

    fn tick(&mut self, dt: f64, w: &World) {
        if w.boost_s > 0.0 {
            for (i, c) in RAINBOW.iter().enumerate() {
                self.parts.push(Particle {
                    x: w.car_x - CAR_W / 2.0,
                    y: BASE_Y - w.alt - 60.0 + (i as f64) * 7.0,
                    vx: -w.vx * 0.4,
                    vy: 0.0,
                    ttl: 0.5,
                    age: 0.0,
                    size: 6.0,
                    grav: 0.0,
                    color: c,
                });
            }
        }
        if let Phase::Won(tier) = w.phase {
            if tier != Tier::BelowThreshold {
                self.firework_s -= dt;
                if self.firework_s <= 0.0 {
                    self.firework_s = if tier == Tier::Landslide { 0.45 } else { 1.1 };
                    let x = FINISH_X + 150.0 + self.rand() * 800.0;
                    let y = 60.0 + self.rand() * 180.0;
                    self.burst(x, y, 36, 260.0, &RAINBOW);
                }
            }
        }
        for p in &mut self.parts {
            p.age += dt;
            p.vy += p.grav * dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        self.parts.retain(|p| p.age < p.ttl);
        for tst in &mut self.toasts {
            tst.age += dt;
        }
        self.toasts.retain(|t| t.age < 1.6);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;
    /// Apex of a standing jump, the ceiling every platform deck and lamp-post
    /// height in the course is chosen against.
    const JUMP_APEX: f64 = JUMP_VY * JUMP_VY / (2.0 * GRAVITY);

    fn run(w: &mut World, secs: f64, inp: Input) -> Vec<Ev> {
        let mut evs = Vec::new();
        let steps = (secs / DT) as usize;
        for _ in 0..steps {
            evs.extend(w.tick(DT, inp));
        }
        evs
    }

    fn playing() -> World {
        let mut w = World::new();
        w.start();
        w
    }

    fn right() -> Input {
        Input {
            right: true,
            ..Default::default()
        }
    }

    #[test]
    fn driving_drains_the_battery() {
        let mut w = playing();
        run(&mut w, 3.0, right());
        assert!(w.car_x > 400.0, "car moved: {}", w.car_x);
        assert!(
            w.battery < 100.0 && w.battery > 75.0,
            "drained some: {}",
            w.battery
        );
    }

    #[test]
    fn battery_pickup_recharges_once() {
        let mut w = playing();
        w.battery = 40.0;
        w.car_x = w.batteries[0].x - 200.0;
        w.vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(evs.contains(&Ev::Battery));
        assert!(w.battery > 60.0, "recharged: {}", w.battery);
        assert!(w.batteries[0].taken);
        w.car_x = w.batteries[0].x - 200.0;
        let evs = run(&mut w, 1.0, right());
        assert!(!evs.contains(&Ev::Battery));
    }

    #[test]
    fn barrier_blocks_a_grounded_car() {
        let mut w = playing();
        let bx = w.barriers[0].x;
        w.car_x = bx - 150.0;
        w.vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(evs.contains(&Ev::Border));
        assert!(w.car_x < bx, "still on the left: {} < {bx}", w.car_x);
    }

    #[test]
    fn full_speed_jump_clears_a_barrier() {
        let mut w = playing();
        let bx = w.barriers[0].x;
        w.car_x = bx - 200.0;
        w.vx = MAX_SPEED;
        let jump = Input {
            right: true,
            jump: true,
            ..Default::default()
        };
        let mut evs = w.tick(DT, jump);
        evs.extend(run(&mut w, 1.2, right()));
        assert!(!evs.contains(&Ev::Border), "no border stop");
        assert!(w.car_x > bx, "landed past: {} > {bx}", w.car_x);
    }

    #[test]
    fn eu_flag_lifts_the_next_barrier() {
        let mut w = playing();
        let fx = w.flags[0].x;
        let bx = w.barriers[1].x;
        w.car_x = fx - 100.0;
        w.vx = MAX_SPEED;
        let evs = run(&mut w, 3.0, right());
        assert!(evs.contains(&Ev::Flag));
        assert!(evs.contains(&Ev::BarrierOpen));
        assert!(!evs.contains(&Ev::Border));
        assert!(w.car_x > bx, "waved through: {} > {bx}", w.car_x);
        assert!(!w.flag_held, "the flag is spent");
    }

    #[test]
    fn poster_hangs_only_in_range_and_once() {
        let mut w = playing();
        w.car_x = w.spots[0].x + 50.0;
        let act = Input {
            action: true,
            ..Default::default()
        };
        let evs = w.tick(DT, act);
        assert!(evs.contains(&Ev::Poster));
        assert_eq!(w.hung, 1);
        let evs = w.tick(DT, act);
        assert!(!evs.contains(&Ev::Poster));
        assert_eq!(w.hung, 1);

        w.car_x = 100.0;
        let evs = w.tick(DT, act);
        assert!(!evs.contains(&Ev::Poster), "nothing in range");
    }

    #[test]
    fn a_jump_lands_the_car_on_a_platform_and_it_falls_off_the_end() {
        let mut w = playing();
        let p = (w.platforms[0].x0, w.platforms[0].x1, w.platforms[0].top);
        w.car_x = p.0 - 120.0;
        w.vx = 300.0;
        let jump = Input {
            right: true,
            jump: true,
            ..Default::default()
        };
        w.tick(DT, jump);
        run(&mut w, 0.9, right());
        assert!(w.grounded, "settled on the deck");
        assert!((w.alt - p.2).abs() < 1.0, "at deck height: {}", w.alt);
        assert!(w.car_x > p.0 && w.car_x < p.1, "on the deck: {}", w.car_x);

        run(&mut w, 2.0, right());
        assert!(w.car_x > p.1, "left the deck: {}", w.car_x);
        assert_eq!(w.alt, 0.0, "back on the road");
    }

    #[test]
    fn the_road_passes_under_a_platform() {
        let mut w = playing();
        let p0 = w.platforms[0].x0;
        w.car_x = p0 - 200.0;
        w.vx = MAX_SPEED;
        run(&mut w, 1.5, right());
        assert!(w.car_x > p0, "drove on: {}", w.car_x);
        assert_eq!(w.alt, 0.0, "stayed on the road, not lifted onto the deck");
    }

    #[test]
    fn a_lamp_post_poster_needs_the_car_in_the_air() {
        let mut w = playing();
        let lamp = w
            .spots
            .iter()
            .position(|s| s.kind == SpotKind::Lamp)
            .expect("course has a lamp post");
        let (lx, lalt) = (w.spots[lamp].x, w.spots[lamp].alt);
        assert!(lalt < JUMP_APEX, "reachable from the road: {lalt}");
        w.car_x = lx;
        let act = Input {
            action: true,
            ..Default::default()
        };
        let evs = w.tick(DT, act);
        assert!(!evs.contains(&Ev::Poster), "out of reach from the ground");

        // Jump under it and hang the poster near the top of the arc.
        w.tick(
            DT,
            Input {
                jump: true,
                ..Default::default()
            },
        );
        let mut hung = false;
        for _ in 0..(1.2 / DT) as usize {
            let inp = Input {
                action: (w.alt - lalt).abs() < POSTER_REACH_V,
                ..Default::default()
            };
            hung |= w.tick(DT, inp).contains(&Ev::Poster);
        }
        assert!(hung, "hung it from the air");
    }

    #[test]
    fn a_rooftop_battery_is_out_of_reach_from_the_road() {
        let mut w = playing();
        let high = w
            .batteries
            .iter()
            .position(|p| p.alt > 0.0)
            .expect("course has a rooftop battery");
        let (bx, balt) = (w.batteries[high].x, w.batteries[high].alt);
        w.battery = 40.0;
        w.car_x = bx;
        run(&mut w, 0.5, Input::default());
        assert!(!w.batteries[high].taken, "not scooped up from below");

        w.alt = balt;
        w.tick(DT, Input::default());
        assert!(w.batteries[high].taken, "collected from the deck");
    }

    #[test]
    fn standing_still_still_drains_the_battery() {
        let mut w = playing();
        run(&mut w, 5.0, Input::default());
        assert!(w.battery < 99.0, "idling costs charge: {}", w.battery);
        assert!(w.battery > 92.0, "but only slowly: {}", w.battery);
    }

    #[test]
    fn rainbow_strip_boosts_past_top_speed() {
        let mut w = playing();
        w.car_x = w.rainbows[0].x0 - 60.0;
        w.vx = MAX_SPEED;
        let mut boosted = false;
        for _ in 0..90 {
            w.tick(DT, right());
            boosted |= w.vx > MAX_SPEED + 50.0;
        }
        assert!(boosted, "went past top speed");
    }

    #[test]
    fn sun_zone_recharges_while_inside() {
        let mut w = playing();
        w.battery = 50.0;
        w.car_x = w.sun[0].0 + 100.0;
        let evs = run(&mut w, 2.0, Input::default());
        assert!(evs.contains(&Ev::Sun));
        assert!(w.battery > 55.0, "solar charged: {}", w.battery);
    }

    #[test]
    fn empty_battery_strands_the_car() {
        let mut w = playing();
        w.battery = 1.0;
        w.vx = MAX_SPEED;
        let evs = run(&mut w, 6.0, right());
        assert!(evs.contains(&Ev::Lost));
        assert_eq!(w.phase, Phase::Lost);
    }

    #[test]
    fn finishing_tiers_follow_the_poster_count() {
        for (hung, tier) in [
            (13, Tier::Landslide),
            (8, Tier::Elected),
            (3, Tier::BelowThreshold),
        ] {
            let mut w = playing();
            w.hung = hung;
            w.car_x = FINISH_X - 120.0;
            w.vx = MAX_SPEED;
            let evs = run(&mut w, 1.0, right());
            assert!(evs.contains(&Ev::Won(tier)), "{hung} posters -> {tier:?}");
            assert_eq!(w.phase, Phase::Won(tier));
        }
    }

    #[test]
    fn course_is_beatable_on_collected_power() {
        // Tuning invariant: a full-speed run that jumps the barriers and rolls
        // through the roadside batteries must arrive, not strand.
        let mut w = playing();
        let barriers: Vec<f64> = w.barriers.iter().map(|b| b.x).collect();
        for _ in 0..(120.0 / DT) as usize {
            let near_barrier = barriers
                .iter()
                .any(|&bx| bx - w.car_x > 0.0 && bx - w.car_x < 210.0);
            let inp = Input {
                right: true,
                jump: near_barrier && w.grounded && w.vx > 380.0,
                ..Default::default()
            };
            w.tick(DT, inp);
            if w.phase != Phase::Playing {
                break;
            }
        }
        assert!(
            matches!(w.phase, Phase::Won(_)),
            "reached the end: {:?} at x {}",
            w.phase,
            w.car_x
        );
    }
}

// Everything below is the wasm rendering/input layer.

const CANVAS_ID: &str = "radikal-game-canvas";

struct Sprites {
    car: web_sys::HtmlImageElement,
    car_poster: web_sys::HtmlImageElement,
    shadow: web_sys::HtmlImageElement,
    border: web_sys::HtmlImageElement,
    battery: web_sys::HtmlImageElement,
    flag_eu: web_sys::HtmlImageElement,
    flag_dk: web_sys::HtmlImageElement,
    christiansborg: web_sys::HtmlImageElement,
    bg: web_sys::HtmlImageElement,
    city: web_sys::HtmlImageElement,
    house: web_sys::HtmlImageElement,
    trees: web_sys::HtmlImageElement,
    windmills: web_sys::HtmlImageElement,
    sun: web_sys::HtmlImageElement,
    hud_battery: web_sys::HtmlImageElement,
    hud_poster: web_sys::HtmlImageElement,
    poster: web_sys::HtmlImageElement,
}

const A_CAR: Asset = asset!("/assets/game/car.png");
const A_CAR_POSTER: Asset = asset!("/assets/game/car-poster.png");
const A_SHADOW: Asset = asset!("/assets/game/car-shadow.png");
const A_BORDER: Asset = asset!("/assets/game/border.png");
const A_BATTERY: Asset = asset!("/assets/game/battery-powerup.png");
const A_FLAG_EU: Asset = asset!("/assets/game/flag-eu.png");
const A_FLAG_DK: Asset = asset!("/assets/game/flag-dk.png");
const A_CHRISTIANSBORG: Asset = asset!("/assets/game/christiansborg.png");
const A_BG: Asset = asset!("/assets/game/bg.png");
const A_CITY: Asset = asset!("/assets/game/parallax-city.png");
const A_HOUSE: Asset = asset!("/assets/game/parallax-house.png");
const A_TREES: Asset = asset!("/assets/game/parallax-trees.png");
const A_WINDMILLS: Asset = asset!("/assets/game/parallax-windmills.png");
const A_SUN: Asset = asset!("/assets/game/sunbeams.png");
const A_HUD_BATTERY: Asset = asset!("/assets/game/hud-battery.png");
const A_HUD_POSTER: Asset = asset!("/assets/game/hud-poster.png");
const A_POSTER: Asset = asset!("/assets/game/poster.png");

impl Sprites {
    fn load() -> Option<Sprites> {
        let img = |a: Asset| {
            let el = web_sys::HtmlImageElement::new().ok()?;
            el.set_src(&a.to_string());
            Some(el)
        };
        Some(Sprites {
            car: img(A_CAR)?,
            car_poster: img(A_CAR_POSTER)?,
            shadow: img(A_SHADOW)?,
            border: img(A_BORDER)?,
            battery: img(A_BATTERY)?,
            flag_eu: img(A_FLAG_EU)?,
            flag_dk: img(A_FLAG_DK)?,
            christiansborg: img(A_CHRISTIANSBORG)?,
            bg: img(A_BG)?,
            city: img(A_CITY)?,
            house: img(A_HOUSE)?,
            trees: img(A_TREES)?,
            windmills: img(A_WINDMILLS)?,
            sun: img(A_SUN)?,
            hud_battery: img(A_HUD_BATTERY)?,
            hud_poster: img(A_HUD_POSTER)?,
            poster: img(A_POSTER)?,
        })
    }
}

fn blit(
    ctx: &web_sys::CanvasRenderingContext2d,
    img: &web_sys::HtmlImageElement,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) {
    if img.complete() && img.natural_width() > 0 {
        let _ = ctx.draw_image_with_html_image_element_and_dw_and_dh(img, x, y, w, h);
    }
}

/// Canvas + 2D context, resized to the element's CSS box at device pixels.
/// Returns the backing size; None until the canvas is in the DOM.
fn game_canvas() -> Option<(
    web_sys::HtmlCanvasElement,
    web_sys::CanvasRenderingContext2d,
    f64,
    f64,
)> {
    use wasm_bindgen::JsCast;
    let win = web_sys::window()?;
    let canvas = win
        .document()?
        .get_element_by_id(CANVAS_ID)?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;
    let cw = f64::from(canvas.client_width());
    if cw <= 0.0 {
        return None;
    }
    let ch = (cw * 0.5).clamp(280.0, WORLD_H);
    let dpr = win.device_pixel_ratio().max(1.0);
    let (bw, bh) = ((cw * dpr).round(), (ch * dpr).round());
    if f64::from(canvas.width()) != bw || f64::from(canvas.height()) != bh {
        canvas.set_width(bw as u32);
        canvas.set_height(bh as u32);
        let _ = canvas.style().set_property("height", &format!("{ch}px"));
    }
    let ctx = canvas
        .get_context("2d")
        .ok()??
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .ok()?;
    Some((canvas, ctx, bw, bh))
}

fn draw_frame(
    ctx: &web_sys::CanvasRenderingContext2d,
    bw: f64,
    bh: f64,
    w: &World,
    spr: &Sprites,
    fx: &Fx,
    anim_t: f64,
) {
    let scale = bh / WORLD_H;
    let view_w = bw / scale;
    let cam = (w.car_x - view_w * 0.38).clamp(0.0, (WORLD_RIGHT - view_w).max(0.0));

    let _ = ctx.reset_transform();
    let _ = ctx.scale(scale, scale);

    ctx.set_fill_style_str("#BFE3F0");
    ctx.fill_rect(0.0, 0.0, view_w, WORLD_H);
    blit(ctx, &spr.bg, 0.0, 0.0, view_w, WORLD_H);

    // No parallax factor: the rays mark a world place, not scenery.
    ctx.set_global_alpha(0.85);
    for &(a, b) in &w.sun {
        blit(ctx, &spr.sun, a - 150.0 - cam, 0.0, b - a + 300.0, 660.0);
    }
    ctx.set_global_alpha(1.0);

    draw_parallax(ctx, spr, cam, view_w);
    draw_ground(ctx, w, cam, view_w);
    draw_entities(ctx, w, spr, cam, anim_t);
    draw_car(ctx, w, spr, cam, anim_t);
    draw_fx(ctx, fx, cam, view_w);

    // Election-night mood: a grey wash when the result disappoints.
    match w.phase {
        Phase::Won(Tier::BelowThreshold) => {
            ctx.set_fill_style_str("rgba(90, 90, 100, 0.45)");
            ctx.fill_rect(0.0, 0.0, view_w, WORLD_H);
        }
        Phase::Lost => {
            ctx.set_fill_style_str("rgba(40, 40, 60, 0.35)");
            ctx.fill_rect(0.0, 0.0, view_w, WORLD_H);
        }
        _ => {}
    }

    draw_hud(ctx, w, spr, view_w, anim_t);
}

fn draw_parallax(ctx: &web_sys::CanvasRenderingContext2d, spr: &Sprites, cam: f64, view_w: f64) {
    let f = 0.22;
    let tile = 640.0;
    let first = ((cam * f) / tile).floor() * tile;
    let mut x = first;
    ctx.set_global_alpha(0.9);
    while x < cam * f + view_w + tile {
        blit(ctx, &spr.city, x - cam * f, HORIZON - 176.0, 560.0, 180.0);
        x += tile;
    }
    ctx.set_global_alpha(1.0);

    let f = 0.55;
    for i in 0..6 {
        let wx = 500.0 + f64::from(i) * 1150.0;
        blit(
            ctx,
            &spr.windmills,
            wx - cam * f,
            HORIZON - 424.0,
            560.0,
            430.0,
        );
    }
    let f = 0.78;
    for i in 0..6 {
        let wx = 650.0 + f64::from(i) * 1500.0;
        blit(ctx, &spr.house, wx - cam * f, HORIZON - 212.0, 300.0, 218.0);
    }
    for i in 0..10 {
        let wx = 200.0 + f64::from(i) * 900.0;
        blit(ctx, &spr.trees, wx - cam * f, HORIZON - 230.0, 280.0, 234.0);
    }
}

fn draw_ground(ctx: &web_sys::CanvasRenderingContext2d, w: &World, cam: f64, view_w: f64) {
    ctx.set_fill_style_str("#8CC152");
    ctx.fill_rect(0.0, HORIZON, view_w, ROAD_TOP - HORIZON);
    ctx.set_fill_style_str("#4B4F54");
    ctx.fill_rect(0.0, ROAD_TOP, view_w, WORLD_H - ROAD_TOP);

    ctx.set_fill_style_str("#F4F4F4");
    let dash = 150.0;
    let first = (cam / dash).floor() * dash;
    let mut x = first;
    while x < cam + view_w + dash {
        ctx.fill_rect(x - cam, 502.0, 62.0, 7.0);
        x += dash;
    }

    for s in &w.rainbows {
        for (i, c) in RAINBOW.iter().enumerate() {
            ctx.set_global_alpha(0.85);
            ctx.set_fill_style_str(c);
            ctx.fill_rect(s.x0 - cam, 472.0 + (i as f64) * 5.4, s.x1 - s.x0, 5.4);
        }
        ctx.set_global_alpha(1.0);
    }
}

fn draw_entities(
    ctx: &web_sys::CanvasRenderingContext2d,
    w: &World,
    spr: &Sprites,
    cam: f64,
    anim_t: f64,
) {
    blit(
        ctx,
        &spr.christiansborg,
        FINISH_X + 50.0 - cam,
        ROAD_TOP - 450.0,
        1060.0,
        450.0,
    );
    blit(
        ctx,
        &spr.flag_dk,
        FINISH_X - 120.0 - cam,
        ROAD_TOP - 135.0,
        66.0,
        135.0,
    );
    blit(
        ctx,
        &spr.flag_eu,
        FINISH_X - 45.0 - cam,
        ROAD_TOP - 135.0,
        66.0,
        135.0,
    );

    for p in &w.platforms {
        let (x0, x1) = (p.x0 - cam, p.x1 - cam);
        let deck = BASE_Y - p.top;
        ctx.set_fill_style_str("#7C848C");
        ctx.fill_rect(x0, deck, x1 - x0, 12.0);
        ctx.set_fill_style_str("#98A0A8");
        ctx.fill_rect(x0, deck + 12.0, x1 - x0, 8.0);
        // Legs at the ends only: the gap between them is the visual promise
        // that a deck can be driven under.
        ctx.set_fill_style_str("#8A9299");
        for lx in [x0 + 6.0, x1 - 20.0] {
            ctx.fill_rect(lx, deck + 20.0, 14.0, BASE_Y - deck - 20.0);
        }
    }

    for s in &w.spots {
        let x = s.x - cam;
        let panel_y = BASE_Y - s.alt - 130.0;
        match s.kind {
            SpotKind::Lamp => {
                ctx.set_fill_style_str("#6E767E");
                ctx.fill_rect(x - 4.0, panel_y, 8.0, BASE_Y - s.alt - panel_y);
                ctx.fill_rect(x - 4.0, panel_y - 26.0, 34.0, 7.0);
                ctx.set_fill_style_str("#F6D66B");
                ctx.fill_rect(x + 22.0, panel_y - 22.0, 16.0, 12.0);
            }
            SpotKind::Board | SpotKind::Ledge => {
                ctx.set_fill_style_str("#9AA0A6");
                ctx.fill_rect(
                    x - 3.0,
                    panel_y + 44.0,
                    6.0,
                    BASE_Y - s.alt - panel_y - 44.0,
                );
            }
        }
        if s.hung {
            blit(ctx, &spr.poster, x - 27.0, panel_y - 42.0, 54.0, 86.0);
        } else {
            ctx.set_fill_style_str("#F3F3F3");
            ctx.fill_rect(x - 27.0, panel_y - 42.0, 54.0, 86.0);
            // Bright only when the action key would actually take it, so the
            // reach rule is legible: drive up, or jump, until it lights.
            let live = w.phase == Phase::Playing;
            let ready = live && spot_in_reach(s, w.car_x, w.alt);
            let near = live && (s.x - w.car_x).abs() < POSTER_RANGE * 1.6;
            ctx.set_global_alpha(if ready {
                0.55 + 0.45 * (anim_t * 6.0).sin()
            } else if near {
                0.5
            } else {
                0.3
            });
            ctx.set_stroke_style_str(if ready {
                "#E6007E"
            } else if near {
                "#F9A825"
            } else {
                "#9AA0A6"
            });
            ctx.set_line_width(4.0);
            ctx.stroke_rect(x - 27.0, panel_y - 42.0, 54.0, 86.0);
            ctx.set_global_alpha(1.0);
        }
    }

    for b in &w.barriers {
        let x = b.x - cam;
        ctx.save();
        let _ = ctx.translate(x - 95.0, BASE_Y);
        let _ = ctx.rotate(-b.lift * 1.2);
        blit(ctx, &spr.border, 0.0, -220.0, 220.0, 220.0);
        ctx.restore();
    }

    for p in &w.batteries {
        if !p.taken {
            let bob = 10.0 * (anim_t * 3.0 + p.x * 0.01).sin();
            blit(
                ctx,
                &spr.battery,
                p.x - cam - 31.0,
                BASE_Y - p.alt - 141.0 + bob,
                62.0,
                62.0,
            );
        }
    }
    for p in &w.flags {
        if !p.taken {
            blit(
                ctx,
                &spr.flag_eu,
                p.x - cam - 23.0,
                ROAD_TOP - 100.0,
                50.0,
                100.0,
            );
        }
    }
}

fn draw_car(
    ctx: &web_sys::CanvasRenderingContext2d,
    w: &World,
    spr: &Sprites,
    cam: f64,
    anim_t: f64,
) {
    let x = w.car_x - cam;
    let shadow_w = (145.0 - w.alt * 0.3).max(80.0);
    ctx.set_global_alpha((0.5 - w.alt * 0.002).max(0.15));
    blit(
        ctx,
        &spr.shadow,
        x - shadow_w / 2.0,
        BASE_Y + 1.0,
        shadow_w,
        11.0,
    );
    ctx.set_global_alpha(1.0);

    let body = if w.hung < w.spots.len() {
        &spr.car_poster
    } else {
        &spr.car
    };
    let top = BASE_Y - CAR_H - w.alt;
    ctx.save();
    let _ = ctx.translate(x, top + CAR_H / 2.0);
    let pitch = (-w.vy * 0.000_15).clamp(-0.1, 0.1);
    let _ = ctx.rotate(pitch);
    blit(ctx, body, -CAR_W / 2.0, -CAR_H / 2.0, CAR_W, CAR_H);
    ctx.restore();

    if w.flag_held {
        ctx.save();
        let _ = ctx.translate(x - 58.0, top - 44.0);
        let _ = ctx.rotate((anim_t * 4.0).sin() * 0.06);
        blit(ctx, &spr.flag_eu, 0.0, 0.0, 30.0, 60.0);
        ctx.restore();
    }
}

fn draw_fx(ctx: &web_sys::CanvasRenderingContext2d, fx: &Fx, cam: f64, view_w: f64) {
    for p in &fx.parts {
        let a = (1.0 - p.age / p.ttl).clamp(0.0, 1.0);
        ctx.set_global_alpha(a);
        ctx.set_fill_style_str(p.color);
        ctx.fill_rect(p.x - cam, p.y, p.size, p.size);
    }
    ctx.set_global_alpha(1.0);

    ctx.set_font("600 24px 'Atkinson Hyperlegible', system-ui, sans-serif");
    ctx.set_text_align("center");
    for (i, t) in fx.toasts.iter().enumerate() {
        let a = (1.0 - t.age / 1.6).clamp(0.0, 1.0);
        // Stacked upward and kept inside the view, so simultaneous toasts and
        // an edge-hugging car both stay readable.
        let y = 330.0 - t.age * 45.0 - (i as f64) * 30.0;
        let x = (t.x - cam).clamp(230.0, view_w - 230.0);
        ctx.set_global_alpha(a);
        ctx.set_stroke_style_str("#FFFFFF");
        ctx.set_line_width(5.0);
        let _ = ctx.stroke_text(&t.text, x, y);
        ctx.set_fill_style_str("#C2185B");
        let _ = ctx.fill_text(&t.text, x, y);
    }
    ctx.set_global_alpha(1.0);
    ctx.set_text_align("start");
}

fn draw_hud(
    ctx: &web_sys::CanvasRenderingContext2d,
    w: &World,
    spr: &Sprites,
    view_w: f64,
    anim_t: f64,
) {
    blit(ctx, &spr.hud_poster, 20.0, 14.0, 26.0, 42.0);
    ctx.set_font("700 30px 'Atkinson Hyperlegible', system-ui, sans-serif");
    ctx.set_fill_style_str("#20242A");
    let _ = ctx.fill_text(&format!("{}/{}", w.hung, w.spots.len()), 58.0, 46.0);

    // Fill before frame: the sprite overlays the fill and masks its edges.
    let (bx, by, bw2, bh2) = (view_w - 226.0, 14.0, 200.0, 30.0);
    let frac = w.battery / 100.0;
    let color = if frac > 0.5 {
        "#43A047"
    } else if frac > 0.25 {
        "#F9A825"
    } else {
        "#E53935"
    };
    let alpha = if frac < 0.15 {
        0.55 + 0.45 * (anim_t * 8.0).sin()
    } else {
        1.0
    };
    let s = bw2 / 632.0;
    ctx.set_global_alpha(alpha);
    ctx.set_fill_style_str(color);
    ctx.fill_rect(bx + 16.0 * s, by + 16.0 * s, 458.0 * s * frac, 64.0 * s);
    ctx.set_global_alpha(1.0);
    blit(ctx, &spr.hud_battery, bx, by, bw2, bh2);
}

fn toast_for(ev: Ev) -> Option<String> {
    match ev {
        Ev::Battery => Some(t("game.toastBattery")),
        Ev::Flag => Some(t("game.toastFlag")),
        Ev::BarrierOpen => Some(t("game.toastOpen")),
        Ev::Border => Some(t("game.toastBorder")),
        Ev::Rainbow => Some(t("game.toastRainbow")),
        Ev::Sun => Some(t("game.toastSun")),
        Ev::Poster => Some(t("game.toastPoster")),
        Ev::Won(_) | Ev::Lost => None,
    }
}

/// GameApp: the campaign minigame, node-independent like `?app=feedback` so
/// `/?app=game` works signed out. Kept off the app rail on purpose, in the
/// spirit of the `cow` easter egg.
#[component]
pub fn GameApp() -> Element {
    let mut phase = use_signal(|| UiPhase::Intro);
    let world = use_hook(|| Rc::new(RefCell::new(World::new())));
    let held = use_hook(|| Rc::new(RefCell::new(Held::default())));
    let fx = use_hook(|| Rc::new(RefCell::new(Fx::new())));
    let sprites = use_hook(|| Rc::new(Sprites::load()));

    let start = {
        let world = world.clone();
        let fx = fx.clone();
        move || {
            world.borrow_mut().start();
            *fx.borrow_mut() = Fx::new();
            phase.set(UiPhase::Playing);
        }
    };

    {
        let world = world.clone();
        let held = held.clone();
        let fx = fx.clone();
        let sprites = sprites.clone();
        use_future(move || {
            let world = world.clone();
            let held = held.clone();
            let fx = fx.clone();
            let sprites = sprites.clone();
            async move {
                let mut last = js_sys::Date::now();
                loop {
                    gloo_timers::future::TimeoutFuture::new(16).await;
                    let now = js_sys::Date::now();
                    // Clamped so a background tab does not fast-forward on return.
                    let dt = ((now - last) / 1000.0).clamp(0.0, 0.05);
                    last = now;

                    let inp = held.borrow_mut().snapshot();
                    let (evs, car_x) = {
                        let mut w = world.borrow_mut();
                        let evs = w.tick(dt, inp);
                        (evs, w.car_x)
                    };
                    {
                        let mut fx = fx.borrow_mut();
                        for ev in &evs {
                            if let Some(text) = toast_for(*ev) {
                                fx.toast(text, car_x);
                            }
                            match ev {
                                Ev::Rainbow => fx.burst(car_x, 400.0, 24, 220.0, &RAINBOW),
                                Ev::Poster => {
                                    fx.burst(car_x, 330.0, 14, 160.0, &["#E6007E", "#FFFFFF"])
                                }
                                _ => {}
                            }
                        }
                        fx.tick(dt, &world.borrow());
                    }

                    let ui = ui_phase(&world.borrow());
                    if *phase.peek() != ui {
                        phase.set(ui);
                    }

                    if let Some((_, ctx, bw, bh)) = game_canvas() {
                        let anim_t = now / 1000.0;
                        if let Some(spr) = sprites.as_ref() {
                            draw_frame(&ctx, bw, bh, &world.borrow(), spr, &fx.borrow(), anim_t);
                        }
                    }
                }
            }
        });
    }

    let on_key_down = {
        let held = held.clone();
        let mut start = start.clone();
        move |evt: Event<KeyboardData>| {
            let mut h = held.borrow_mut();
            match evt.key() {
                Key::ArrowRight => {
                    h.right = true;
                    evt.prevent_default();
                }
                Key::ArrowLeft => {
                    h.left = true;
                    evt.prevent_default();
                }
                Key::ArrowUp => {
                    if !h.jump_down {
                        h.jump_edge = true;
                    }
                    h.jump_down = true;
                    evt.prevent_default();
                }
                Key::ArrowDown => {
                    if !h.act_down {
                        h.act_edge = true;
                    }
                    h.act_down = true;
                    evt.prevent_default();
                }
                Key::Enter => {
                    if *phase.peek() == UiPhase::Playing {
                        if !h.act_down {
                            h.act_edge = true;
                        }
                        h.act_down = true;
                    } else {
                        drop(h);
                        start();
                    }
                    evt.prevent_default();
                }
                Key::Character(c) => match c.as_str() {
                    "d" | "D" => h.right = true,
                    "a" | "A" => h.left = true,
                    "w" | "W" | " " => {
                        if !h.jump_down {
                            h.jump_edge = true;
                        }
                        h.jump_down = true;
                        evt.prevent_default();
                    }
                    "e" | "E" => {
                        if !h.act_down {
                            h.act_edge = true;
                        }
                        h.act_down = true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    };

    let on_key_up = {
        let held = held.clone();
        move |evt: Event<KeyboardData>| {
            let mut h = held.borrow_mut();
            match evt.key() {
                Key::ArrowRight => h.right = false,
                Key::ArrowLeft => h.left = false,
                Key::ArrowUp => h.jump_down = false,
                Key::ArrowDown | Key::Enter => h.act_down = false,
                Key::Character(c) => match c.as_str() {
                    "d" | "D" => h.right = false,
                    "a" | "A" => h.left = false,
                    "w" | "W" | " " => h.jump_down = false,
                    "e" | "E" => h.act_down = false,
                    _ => {}
                },
                _ => {}
            }
        }
    };

    let hold = |field: fn(&mut Held, bool)| {
        let held = held.clone();
        move |down: bool| field(&mut held.borrow_mut(), down)
    };
    let hold_left = hold(|h, v| h.left = v);
    let hold_right = hold(|h, v| h.right = v);
    let tap_jump = {
        let held = held.clone();
        move || {
            let mut h = held.borrow_mut();
            h.jump_edge = true;
        }
    };
    let tap_act = {
        let held = held.clone();
        move || {
            let mut h = held.borrow_mut();
            h.act_edge = true;
        }
    };

    let overlay = match phase() {
        UiPhase::Playing => rsx! {},
        UiPhase::Intro => rsx! {
            GameOverlay {
                title: t("game.title"),
                body: t("game.intro"),
                extra: Some(t("game.controls")),
                button: t("game.start"),
                on_click: {
                    let mut start = start.clone();
                    move |_| start()
                },
            }
        },
        UiPhase::Won(tier, hung, total) => {
            let (n, total) = (hung.to_string(), total.to_string());
            let args = [("n", n.as_str()), ("total", total.as_str())];
            let (title, body) = match tier {
                Tier::Landslide => (
                    t("game.wonTitleLandslide"),
                    t_with("game.wonLandslide", &args),
                ),
                Tier::Elected => (t("game.wonTitleElected"), t_with("game.wonElected", &args)),
                Tier::BelowThreshold => (t("game.wonTitleBelow"), t_with("game.wonBelow", &args)),
            };
            rsx! {
                GameOverlay {
                    title,
                    body,
                    extra: None,
                    button: t("game.again"),
                    on_click: {
                        let mut start = start.clone();
                        move |_| start()
                    },
                }
            }
        }
        UiPhase::Lost => rsx! {
            GameOverlay {
                title: t("game.lostTitle"),
                body: t("game.lost"),
                extra: None,
                button: t("game.again"),
                on_click: {
                    let mut start = start.clone();
                    move |_| start()
                },
            }
        },
    };

    rsx! {
        div {
            style: "position:relative; width:100%; max-width:1200px; margin:0 auto; outline:none;",
            role: "application",
            aria_label: t("game.title"),
            tabindex: "0",
            onmounted: move |evt| async move {
                let _ = evt.set_focus(true).await;
            },
            onkeydown: on_key_down,
            onkeyup: on_key_up,
            canvas {
                id: CANVAS_ID,
                style: "display:block; width:100%; height:420px; border-radius:12px; background:#BFE3F0; touch-action:none;",
            }
            div {
                style: "position:absolute; left:12px; bottom:12px; display:flex; gap:10px;",
                HoldButton { label: "◀", on_hold: hold_left }
                HoldButton { label: "▶", on_hold: hold_right }
            }
            div {
                style: "position:absolute; right:12px; bottom:12px; display:flex; gap:10px;",
                TapButton { label: t("game.btnJump"), on_tap: tap_jump }
                TapButton { label: t("game.btnPoster"), on_tap: tap_act }
            }
            {overlay}
        }
    }
}

#[component]
fn GameOverlay(
    title: String,
    body: String,
    extra: Option<String>,
    button: String,
    on_click: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            style: "position:absolute; inset:0; display:flex; align-items:center; justify-content:center; background:rgba(10, 14, 24, 0.45); border-radius:12px; padding:16px;",
            // A plain `card`, not `app-card`: the app-card rule strips the very
            // background a floating dialog needs.
            div { class: "card", style: "max-width:560px; margin:0;",
                div {
                    class: "card-content",
                    style: "display:flex; flex-direction:column; gap:14px; padding:24px 28px; text-align:center;",
                    h2 { style: "margin:0;", "{title}" }
                    p { style: "margin:0;", "{body}" }
                    if let Some(extra) = extra {
                        p { class: "body-medium text-muted", style: "margin:0;", "{extra}" }
                    }
                    div {
                        button {
                            class: "btn btn-primary",
                            autofocus: true,
                            onclick: move |_| on_click.call(()),
                            "{button}"
                        }
                    }
                }
            }
        }
    }
}

const GAME_BTN_STYLE: &str = "min-width:64px; min-height:48px; font-size:20px; touch-action:none; user-select:none; -webkit-user-select:none; background:rgba(255,255,255,0.92); color:#20242A; border-radius:14px;";

/// Press-and-hold control (drive): down on press, up on release or the pointer
/// leaving, so a finger sliding off does not leave the car driving.
#[component]
fn HoldButton(label: String, on_hold: EventHandler<bool>) -> Element {
    rsx! {
        button {
            class: "btn btn-outlined",
            style: GAME_BTN_STYLE,
            onpointerdown: move |_| on_hold.call(true),
            onpointerup: move |_| on_hold.call(false),
            onpointerleave: move |_| on_hold.call(false),
            onpointercancel: move |_| on_hold.call(false),
            "{label}"
        }
    }
}

#[component]
fn TapButton(label: String, on_tap: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "btn btn-outlined",
            style: GAME_BTN_STYLE,
            onpointerdown: move |_| on_tap.call(()),
            "{label}"
        }
    }
}
