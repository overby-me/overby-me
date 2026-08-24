//! GameApp: "Radikal Rally", a side-scrolling campaign minigame (`?app=game`).
//!
//! Drive the party's electric car from the suburbs to Christiansborg, hanging
//! election posters on the way. Every mechanic is a party position turned into
//! play: the battery is the green transition, border barriers are jumped or
//! lifted by an EU flag, an EU flag also waives the bridge toll, rainbow strips
//! give a free speed boost, books widen your reach, the fossil lobby's truck
//! oils the road, and the wind helps or hinders depending on whether you are in
//! the windmills. Sprites live in `assets/game/`; design notes in `docs/game.md`.
//!
//! The module splits into a pure `World` (course layout, physics, battery,
//! events) that unit-tests on the host, and a wasm-only layer: sprite cache,
//! canvas-2D renderer, particles and the `GameApp` component. Two cars share
//! one `World`, so single player is just a race with one entrant.

use std::cell::{Cell, RefCell};
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
/// How long a stack of books widens the reach, and by how much.
const STUDY_S: f64 = 10.0;
const STUDY_MUL: f64 = 2.0;
/// How long an oil slick keeps the steering crossed: long enough to cost the
/// stretch, short enough to steer out of.
const SKID_S: f64 = 1.3;
/// Sideways push and the drain it adds or spares, inside a wind zone.
const WIND_ACCEL: f64 = 165.0;
const WIND_DRAIN: f64 = 0.45;
/// Charge the bridge toll takes, and so also the reserve a flagless car needs
/// to be let through at all.
const TOLL_COST: f64 = 18.0;
/// The fossil lobby's truck: slower than a car at full tilt, so it can be
/// overtaken, but it oils the road behind it while it runs.
const TRUCK_SPEED: f64 = 374.0;
const TRUCK_WAKE: f64 = 780.0;
const SLICK_EVERY_S: f64 = 0.7;
const SLICK_RANGE: f64 = 62.0;
/// In a race, how far apart the two cars may drift before the trailing one is
/// dragged along. Under a screen width, so neither player is ever off-camera.
const RACE_SPREAD: f64 = 880.0;
/// Levels in a campaign. Each is generated from the campaign seed and its own
/// index, so the same seed always replays the same five courses.
const LEVELS: usize = 5;
/// Road left beyond the finish for Christiansborg to stand on.
const RUN_OFF: f64 = 1200.0;

/// A small deterministic generator. A seed must replay the same course on
/// every machine, so nothing here may reach for the clock or a platform RNG.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn unit(&mut self) -> f64 {
        (self.next() as f64) / f64::from(1u32 << 31)
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// Index into a slice of `len` items; `len` must not be zero.
    fn idx(&mut self, len: usize) -> usize {
        (self.next() as usize) % len
    }
}

/// What a level is allowed to contain. Each index adds one kind of trouble, so
/// the campaign teaches the course rather than dropping everything at once.
struct Recipe {
    len: f64,
    poles: bool,
    oil: bool,
    buses: bool,
    toll: bool,
    winds: usize,
    suns: usize,
    rainbows: usize,
}

fn recipe(level: usize) -> Recipe {
    let l = level.min(LEVELS - 1);
    let lf = l as f64;
    Recipe {
        len: 8200.0 + 2100.0 * lf,
        poles: l >= 1,
        oil: l >= 2,
        buses: l >= 3,
        toll: l >= 3,
        winds: if l >= 2 { l - 1 } else { 0 },
        suns: 1 + l / 3,
        rainbows: 2 + l / 2,
    }
}

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
    Over,
}

/// How a finished run is judged. A solo run is measured against the course; a
/// race is measured against the other car.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    /// A level cleared with more to come; the campaign carries on.
    LevelCleared {
        level: usize,
        hung: usize,
        total: usize,
    },
    /// The last level cleared, judged on every board of every level.
    Campaign {
        tier: Tier,
        hung: usize,
        total: usize,
    },
    Stranded,
    Race {
        /// None is a draw on posters.
        winner: Option<usize>,
        hung: (usize, usize),
    },
}

/// A share rather than a count, so the thresholds survive a change to the
/// course length, which the generator changes every level.
fn tier_of(hung: usize, total: usize) -> Tier {
    if hung * 100 >= total * 85 {
        Tier::Landslide
    } else if hung * 100 >= total * 50 {
        Tier::Elected
    } else {
        Tier::BelowThreshold
    }
}

/// One tick's worth of things the UI should react to (toasts, particles),
/// paired with the car it happened to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Ev {
    Battery,
    Books,
    Flag,
    BarrierOpen,
    Border,
    Rainbow,
    Sun,
    Poster,
    Oil,
    TruckBolts,
    TollPaid,
    TollWaived,
    TollBlocked,
    /// True for a tailwind, false for a headwind.
    Wind(bool),
    Over,
}

#[derive(Default, Clone, Copy)]
struct Input {
    left: bool,
    right: bool,
    /// Edge-triggered: true for the single tick after the key went down.
    jump: bool,
    action: bool,
}

impl Input {
    /// Both key sets drive the same car when there is only one.
    fn merged(a: Input, b: Input) -> Input {
        Input {
            left: a.left || b.left,
            right: a.right || b.right,
            jump: a.jump || b.jump,
            action: a.action || b.action,
        }
    }
}

#[derive(Clone)]
struct Car {
    x: f64,
    vx: f64,
    /// Height of the car bottom above the road; 0 is the road itself.
    alt: f64,
    vy: f64,
    /// Resting on a surface (road or platform deck), so a jump is available.
    grounded: bool,
    /// Index of the deck being stood on, so a moving one carries the car.
    riding: Option<usize>,
    battery: f64,
    boost_s: f64,
    study_s: f64,
    skid_s: f64,
    flag_held: bool,
    hung: usize,
    in_sun: bool,
    /// Sign of the wind last tick, so entering a zone announces itself once.
    wind_sign: i8,
    finished: bool,
}

impl Car {
    fn new(x: f64) -> Self {
        Car {
            x,
            vx: 0.0,
            alt: 0.0,
            vy: 0.0,
            grounded: true,
            riding: None,
            battery: 100.0,
            boost_s: 0.0,
            study_s: 0.0,
            skid_s: 0.0,
            flag_held: false,
            hung: 0,
            in_sun: false,
            wind_sign: 0,
            finished: false,
        }
    }

    /// Out of charge and no longer rolling: it will not move again by itself.
    fn stranded(&self) -> bool {
        self.battery <= 0.0 && self.grounded && self.vx.abs() < 8.0
    }

    fn reach(&self) -> (f64, f64) {
        let mul = if self.study_s > 0.0 { STUDY_MUL } else { 1.0 };
        (POSTER_RANGE * mul, POSTER_REACH_V * mul)
    }
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

/// The bridge gate: opened once, by an EU flag or by charge.
struct Toll {
    x: f64,
    open: bool,
    hit_cool: f64,
}

/// What a poster spot is fixed to. Only the drawing differs; reaching one is
/// the same rule everywhere (the car's reach around it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SpotKind {
    Board,
    /// Panel up a light pole: only in reach near the top of a jump.
    Lamp,
    /// At deck height, so the car has to be parked up there. Over a moving
    /// deck that means riding it into position.
    Ledge,
}

struct Spot {
    x: f64,
    alt: f64,
    kind: SpotKind,
    hung: bool,
}

/// Back-and-forth travel for a deck that is a vehicle rather than a building.
struct Motion {
    span: f64,
    speed: f64,
    phase: f64,
}

impl Motion {
    fn offset_at(&self, time_s: f64) -> f64 {
        self.span * 0.5 * (1.0 + (time_s * self.speed + self.phase).sin())
    }
}

/// A city ledge the car can land on. One-way: the car passes up through the
/// deck and settles on it coming down, so a platform never traps anyone
/// underneath.
struct Platform {
    x0: f64,
    x1: f64,
    top: f64,
    motion: Option<Motion>,
    /// Current travel offset; 0 for a building.
    offset: f64,
}

impl Platform {
    fn span(&self) -> (f64, f64) {
        (self.x0 + self.offset, self.x1 + self.offset)
    }
}

struct Strip {
    x0: f64,
    x1: f64,
    cool: f64,
}

/// A stretch of weather. Positive force is a tailwind out of the windmills,
/// negative a headwind that costs charge.
struct Wind {
    x0: f64,
    x1: f64,
    force: f64,
}

struct Slick {
    x: f64,
}

/// The fossil lobby's lorry. It sits still until a car comes up behind it,
/// then bolts, oiling the road until it turns off.
struct Truck {
    x: f64,
    end: f64,
    rolling: bool,
    woke: bool,
    drop_cd: f64,
}

struct World {
    phase: Phase,
    outcome: Option<Outcome>,
    time_s: f64,
    /// Campaign seed and which of its levels this is.
    seed: u64,
    level: usize,
    players: usize,
    /// Finish line; Christiansborg stands just beyond it.
    len: f64,
    /// Posters hung and boards offered in the levels already cleared.
    banked: (usize, usize),
    cars: Vec<Car>,
    batteries: Vec<Pickup>,
    books: Vec<Pickup>,
    flags: Vec<Pickup>,
    barriers: Vec<Barrier>,
    toll: Option<Toll>,
    spots: Vec<Spot>,
    platforms: Vec<Platform>,
    rainbows: Vec<Strip>,
    winds: Vec<Wind>,
    truck: Option<Truck>,
    slicks: Vec<Slick>,
    sun: Vec<(f64, f64)>,
}

/// Whether a car can hang this spot's poster from where it is. Shared with the
/// renderer so the highlight marks exactly what the action key would take.
fn spot_in_reach(s: &Spot, car: &Car) -> bool {
    let (rh, rv) = car.reach();
    (s.x - car.x).abs() < rh && (s.alt - car.alt).abs() < rv
}

/// One stretch of course. The generator lays these left to right, each
/// reporting where the next may start, which is what keeps a random course
/// spaced rather than piled up.
#[derive(Clone, Copy)]
enum Seg {
    Boards,
    Barrier,
    Poles,
    Deck,
    Bus,
    Oil,
}

impl World {
    fn blank(level: usize, seed: u64, players: usize, len: f64) -> Self {
        World {
            phase: Phase::Intro,
            outcome: None,
            time_s: 0.0,
            seed,
            level,
            players,
            len,
            banked: (0, 0),
            // Two cars start a length apart so neither hides the other.
            cars: (0..players.max(1))
                .map(|i| Car::new(140.0 + 170.0 * i as f64))
                .collect(),
            batteries: Vec::new(),
            books: Vec::new(),
            flags: Vec::new(),
            barriers: Vec::new(),
            toll: None,
            spots: Vec::new(),
            platforms: Vec::new(),
            rainbows: Vec::new(),
            winds: Vec::new(),
            truck: None,
            slicks: Vec::new(),
            sun: Vec::new(),
        }
    }

    /// Build one level: a spine of segments laid left to right, then resources
    /// and weather laid over it. Deck tops stay under the standing-jump apex
    /// and no stretch outruns `GROUND_REACH`, so the course is always
    /// drivable, which the generator tests assert over many seeds.
    #[expect(
        clippy::too_many_lines,
        reason = "one table of segment shapes; splitting it would scatter the course's proportions across helpers"
    )]
    fn generate(level: usize, seed: u64, players: usize) -> Self {
        /// Furthest a car may go without passing a battery it can reach from
        /// the road, so skipping every rooftop still finishes the level.
        const GROUND_REACH: f64 = 2000.0;

        let r = recipe(level);
        let mut rng = Rng::new(seed ^ (level as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
        let mut w = World::blank(level, seed, players, r.len);

        // The tail is left clear so the bridge (and the flag that waives it)
        // has room after the last barrier.
        let walk_end = r.len - if r.toll { 2400.0 } else { 1500.0 };
        let mut x = 950.0;
        let mut last_barrier = -9000.0;
        let (mut buses, mut oils) = (0, 0);
        while x < walk_end {
            let mut bag = vec![Seg::Boards, Seg::Boards, Seg::Deck];
            if x - last_barrier > 1600.0 {
                bag.push(Seg::Barrier);
            }
            if r.poles {
                bag.push(Seg::Poles);
            }
            if r.buses && buses < 2 {
                bag.push(Seg::Bus);
            }
            // Never in the opening stretch: the oil is a complication, not an
            // introduction.
            if r.oil && oils < 1 && x > r.len * 0.4 {
                bag.push(Seg::Oil);
            }
            // Once the dice have had most of the course to roll a feature the
            // recipe promises, place it outright: no seed may hand out a level
            // indistinguishable from the one before it.
            let forced = if w.barriers.is_empty() && x > r.len * 0.4 {
                Some(Seg::Barrier)
            } else if r.oil && oils == 0 && x > r.len * 0.55 {
                Some(Seg::Oil)
            } else if r.buses && buses == 0 && x > r.len * 0.65 {
                Some(Seg::Bus)
            } else {
                None
            };
            x = match forced.unwrap_or(bag[rng.idx(bag.len())]) {
                Seg::Boards => {
                    let mut c = x;
                    for _ in 0..=rng.idx(3) {
                        w.spots.push(Spot {
                            x: c,
                            alt: 0.0,
                            kind: SpotKind::Board,
                            hung: false,
                        });
                        c += rng.range(520.0, 880.0);
                    }
                    c + 200.0
                }
                Seg::Barrier => {
                    // A flag before some of them, so free movement is an option
                    // as well as the jump. Never before the first, which is
                    // where the jump is learned.
                    if last_barrier > 0.0 && rng.chance(0.55) {
                        w.flags.push(Pickup {
                            x: x + 90.0,
                            alt: 0.0,
                            taken: false,
                        });
                    }
                    let bx = x + 340.0;
                    w.barriers.push(Barrier {
                        x: bx,
                        lift: 0.0,
                        opening: false,
                        hit_cool: 0.0,
                    });
                    last_barrier = bx;
                    bx + 640.0
                }
                Seg::Poles => {
                    let mut c = x;
                    for _ in 0..=rng.idx(2) {
                        w.spots.push(Spot {
                            x: c,
                            alt: rng.range(145.0, 175.0),
                            kind: SpotKind::Lamp,
                            hung: false,
                        });
                        c += rng.range(620.0, 820.0);
                    }
                    c + 200.0
                }
                Seg::Deck => {
                    let wide = rng.range(380.0, 540.0);
                    let top = rng.range(118.0, 162.0);
                    w.platforms.push(Platform {
                        x0: x,
                        x1: x + wide,
                        top,
                        motion: None,
                        offset: 0.0,
                    });
                    w.spots.push(Spot {
                        x: x + wide * 0.5,
                        alt: top,
                        kind: SpotKind::Ledge,
                        hung: false,
                    });
                    // Now and then a pole above the deck, out of reach from the
                    // road: the only way to it is a jump from up there.
                    if r.poles && rng.chance(0.35) {
                        w.spots.push(Spot {
                            x: x + wide * 0.5 + rng.range(-90.0, 90.0),
                            alt: top + rng.range(115.0, 138.0),
                            kind: SpotKind::Lamp,
                            hung: false,
                        });
                    }
                    x + wide + rng.range(420.0, 700.0)
                }
                Seg::Bus => {
                    buses += 1;
                    let wide = rng.range(260.0, 340.0);
                    let span = rng.range(420.0, 700.0);
                    let top = rng.range(125.0, 150.0);
                    let motion = Motion {
                        span,
                        speed: rng.range(0.5, 0.85),
                        phase: rng.range(0.0, std::f64::consts::TAU),
                    };
                    let offset = motion.offset_at(0.0);
                    w.platforms.push(Platform {
                        x0: x,
                        x1: x + wide,
                        top,
                        motion: Some(motion),
                        offset,
                    });
                    // Fixed over the middle of the route, so it is only in
                    // reach while the bus happens to be passing under it.
                    w.spots.push(Spot {
                        x: x + wide * 0.5 + span * 0.5,
                        alt: top,
                        kind: SpotKind::Ledge,
                        hung: false,
                    });
                    x + wide + span + rng.range(400.0, 650.0)
                }
                Seg::Oil => {
                    oils += 1;
                    let run = rng.range(900.0, 1500.0);
                    w.truck = Some(Truck {
                        x: x + 240.0,
                        end: x + 240.0 + run,
                        rolling: false,
                        woke: false,
                        drop_cd: 0.0,
                    });
                    // Room to stop skidding before whatever comes next.
                    x + 240.0 + run + 750.0
                }
            };
        }

        if r.toll {
            let tx = r.len - 950.0;
            // Placed after every barrier, so the flag meant for the bridge
            // cannot be spent lifting one on the way to it.
            w.flags.push(Pickup {
                x: tx - 700.0,
                alt: 0.0,
                taken: false,
            });
            w.toll = Some(Toll {
                x: tx,
                open: false,
                hit_cool: 0.0,
            });
        }

        let mut bx = 1150.0;
        let mut last_ground = 0.0;
        while bx < r.len - 300.0 {
            let deck = w
                .platforms
                .iter()
                .find(|p| p.motion.is_none() && bx > p.x0 + 40.0 && bx < p.x1 - 40.0);
            let alt = match deck {
                Some(p) if bx - last_ground < GROUND_REACH && rng.chance(0.6) => p.top,
                _ => {
                    last_ground = bx;
                    0.0
                }
            };
            w.batteries.push(Pickup {
                x: bx,
                alt,
                taken: false,
            });
            bx += rng.range(950.0, 1400.0);
        }

        let mut kx = rng.range(1800.0, 2600.0);
        while kx < r.len - 800.0 {
            w.books.push(Pickup {
                x: kx,
                alt: 0.0,
                taken: false,
            });
            kx += rng.range(2800.0, 4200.0);
        }

        // Weather in bands across the course, starting with a tailwind so the
        // first one met is a gift rather than a tax.
        for i in 0..r.winds {
            let band = (r.len - 2600.0) / r.winds as f64;
            let x0 = 1500.0 + band * i as f64 + rng.range(0.0, band * 0.3);
            w.winds.push(Wind {
                x0,
                x1: x0 + rng.range(700.0, 1100.0),
                force: if i % 2 == 0 { 1.0 } else { -1.0 },
            });
        }
        for i in 0..r.suns {
            let band = (r.len - 2600.0) / r.suns as f64;
            let a = 1500.0 + band * i as f64 + rng.range(0.0, band * 0.4);
            w.sun.push((a, a + rng.range(600.0, 1000.0)));
        }

        let mut placed = 0;
        for _ in 0..80 {
            if placed == r.rainbows {
                break;
            }
            let x0 = rng.range(900.0, r.len - 1200.0);
            let x1 = x0 + rng.range(200.0, 330.0);
            // A boost into a closed barrier is a crash, not a reward.
            let clear = !w
                .barriers
                .iter()
                .any(|b| b.x > x0 - 200.0 && b.x < x1 + 900.0)
                && !w
                    .rainbows
                    .iter()
                    .any(|s| x0 < s.x1 + 400.0 && x1 > s.x0 - 400.0);
            if clear {
                w.rainbows.push(Strip { x0, x1, cool: 0.0 });
                placed += 1;
            }
        }
        w
    }

    /// A fresh campaign at level one.
    fn campaign(seed: u64, players: usize) -> Self {
        let mut w = World::generate(0, seed, players);
        w.phase = Phase::Playing;
        w
    }

    /// Bank this level's boards and generate the next of the same campaign.
    fn next_level(&mut self) {
        let banked = (
            self.banked.0 + self.cars[0].hung,
            self.banked.1 + self.spots.len(),
        );
        *self = World::generate(self.level + 1, self.seed, self.players);
        self.banked = banked;
        self.phase = Phase::Playing;
    }

    /// The same level again, from the same seed, keeping what earlier levels
    /// earned: a flat battery costs the level, not the campaign.
    fn retry_level(&mut self) {
        let banked = self.banked;
        *self = World::generate(self.level, self.seed, self.players);
        self.banked = banked;
        self.phase = Phase::Playing;
    }

    fn racing(&self) -> bool {
        self.cars.len() > 1
    }

    /// Highest surface a car at `x` can settle on, and which deck it is, given
    /// where it was before this step's fall. A deck only catches a car coming
    /// down onto it, which is what lets one be jumped through from below.
    fn support_at(&self, x: f64, alt_prev: f64) -> (f64, Option<usize>) {
        let mut best = (0.0, None);
        for (i, p) in self.platforms.iter().enumerate() {
            let (x0, x1) = p.span();
            if x >= x0 && x <= x1 && p.top > best.0 && alt_prev >= p.top - 1.0 {
                best = (p.top, Some(i));
            }
        }
        best
    }

    fn wind_at(&self, x: f64) -> f64 {
        self.winds
            .iter()
            .find(|w| x >= w.x0 && x <= w.x1)
            .map_or(0.0, |w| w.force)
    }

    fn tick(&mut self, dt: f64, inputs: &[Input]) -> Vec<(usize, Ev)> {
        let mut evs = Vec::new();
        if self.phase != Phase::Playing {
            return evs;
        }
        self.time_s += dt;

        // Decks move before the cars, so a rider is carried by this tick's
        // delta rather than sliding off a deck that moved out from under it.
        let mut deck_dx = vec![0.0; self.platforms.len()];
        for (i, p) in self.platforms.iter_mut().enumerate() {
            if let Some(m) = &p.motion {
                let o = m.offset_at(self.time_s);
                deck_dx[i] = o - p.offset;
                p.offset = o;
            }
        }
        self.step_truck(dt, &mut evs);

        for i in 0..self.cars.len() {
            let inp = inputs.get(i).copied().unwrap_or_default();
            let mut car = self.cars[i].clone();
            self.step_car(i, &mut car, dt, inp, &deck_dx, &mut evs);
            self.cars[i] = car;
        }

        // Neither player may drive the other off the screen.
        if self.racing() {
            let (a, b) = (self.cars[0].x, self.cars[1].x);
            if (a - b).abs() > RACE_SPREAD {
                let behind = usize::from(a > b);
                self.cars[behind].x = a.max(b) - RACE_SPREAD;
            }
        }

        self.settle(&mut evs);
        evs
    }

    fn step_truck(&mut self, dt: f64, evs: &mut Vec<(usize, Ev)>) {
        let lead = self
            .cars
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.x.total_cmp(&b.1.x));
        let (Some((who, lead)), Some(truck)) = (lead, self.truck.as_mut()) else {
            return;
        };
        if !truck.woke && lead.x > truck.x - TRUCK_WAKE {
            truck.woke = true;
            truck.rolling = true;
            evs.push((who, Ev::TruckBolts));
        }
        if !truck.rolling {
            return;
        }
        truck.x += TRUCK_SPEED * dt;
        truck.drop_cd -= dt;
        if truck.drop_cd <= 0.0 {
            truck.drop_cd = SLICK_EVERY_S;
            let drop = truck.x - 95.0;
            self.slicks.push(Slick { x: drop });
        }
        if truck.x >= truck.end {
            truck.rolling = false;
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one car's tick in course order; splitting it would scatter a single pass across helpers that all need the same car"
    )]
    fn step_car(
        &mut self,
        who: usize,
        car: &mut Car,
        dt: f64,
        inp: Input,
        deck_dx: &[f64],
        evs: &mut Vec<(usize, Ev)>,
    ) {
        if car.finished {
            return;
        }
        car.study_s = (car.study_s - dt).max(0.0);
        car.skid_s = (car.skid_s - dt).max(0.0);
        let powered = car.battery > 0.0;
        // Oil crosses the controls, so the way out of a skid is to steer into
        // it. Everything below reads the swapped pair, never the raw input.
        let (left, right) = if car.skid_s > 0.0 {
            (inp.right, inp.left)
        } else {
            (inp.left, inp.right)
        };

        if car.boost_s > 0.0 {
            car.boost_s -= dt;
            car.vx = (car.vx + 900.0 * dt).min(BOOST_SPEED);
        } else {
            let mut ax = 0.0;
            if powered && right {
                ax += ACCEL;
            }
            if powered && left {
                ax -= ACCEL;
            }
            if ax == 0.0 {
                // Oil is slippery as well as crossed: it barely scrubs speed.
                let drag = DRAG * dt * if car.skid_s > 0.0 { 0.25 } else { 1.0 };
                car.vx -= car.vx.clamp(-drag, drag);
            } else {
                car.vx = (car.vx + ax * dt).clamp(-REVERSE_MAX, MAX_SPEED);
            }
            if car.vx > MAX_SPEED {
                car.vx = (car.vx - 500.0 * dt).max(MAX_SPEED);
            }
        }

        let wind = self.wind_at(car.x);
        car.vx += wind * WIND_ACCEL * dt;
        let sign = wind.partial_cmp(&0.0).map_or(0, |o| match o {
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
        });
        if sign != car.wind_sign {
            car.wind_sign = sign;
            if sign != 0 {
                evs.push((who, Ev::Wind(sign > 0)));
            }
        }

        if inp.jump && car.grounded && powered {
            car.vy = JUMP_VY;
            car.battery -= JUMP_COST;
            car.grounded = false;
        }
        let alt_prev = car.alt;
        car.vy -= GRAVITY * dt;
        car.alt += car.vy * dt;
        let (support, deck) = self.support_at(car.x, alt_prev);
        car.grounded = car.alt <= support;
        if car.grounded {
            car.alt = support;
            car.vy = 0.0;
            car.riding = deck;
        } else {
            car.riding = None;
        }
        if let Some(d) = car.riding {
            car.x += deck_dx[d];
        }

        let old_x = car.x;
        car.x = (car.x + car.vx * dt).clamp(CAR_W / 2.0, self.len + 60.0);

        for b in &mut self.barriers {
            b.hit_cool = (b.hit_cool - dt).max(0.0);
            if car.flag_held && !b.opening && b.x - car.x > 0.0 && b.x - car.x < 320.0 {
                b.opening = true;
                car.flag_held = false;
                evs.push((who, Ev::BarrierOpen));
            }
            if b.opening && b.lift < 1.0 {
                b.lift = (b.lift + dt / 0.8).min(1.0);
            }
            let blocked = b.lift < 0.4 && car.alt < BARRIER_CLEAR_ALT;
            if blocked && (car.x - b.x).abs() < BARRIER_OVERLAP {
                if old_x <= b.x {
                    car.x = b.x - BARRIER_OVERLAP;
                    if car.vx > 60.0 && b.hit_cool <= 0.0 {
                        b.hit_cool = 1.0;
                        car.battery -= 2.0;
                        evs.push((who, Ev::Border));
                    }
                    car.vx = -140.0;
                } else {
                    // Came down past the wall (the arm's far slope): scrape over
                    // rather than bounce back through a barrier already beaten.
                    car.x = b.x + BARRIER_OVERLAP;
                }
            }
        }

        if let Some(toll) = self.toll.as_mut() {
            toll.hit_cool = (toll.hit_cool - dt).max(0.0);
            if !toll.open && car.x > toll.x - 150.0 && old_x <= car.x {
                if car.flag_held {
                    car.flag_held = false;
                    toll.open = true;
                    evs.push((who, Ev::TollWaived));
                } else if car.battery > TOLL_COST {
                    car.battery -= TOLL_COST;
                    toll.open = true;
                    evs.push((who, Ev::TollPaid));
                } else if car.x > toll.x - BARRIER_OVERLAP {
                    car.x = toll.x - BARRIER_OVERLAP;
                    car.vx = -120.0;
                    if toll.hit_cool <= 0.0 {
                        toll.hit_cool = 1.5;
                        evs.push((who, Ev::TollBlocked));
                    }
                }
            }
        }

        let (cx, calt) = (car.x, car.alt);
        let near =
            |p: &Pickup| (p.x - cx).abs() < PICKUP_RANGE && (p.alt - calt).abs() < PICKUP_REACH_V;
        for p in &mut self.batteries {
            if !p.taken && near(p) {
                p.taken = true;
                car.battery = (car.battery + BATTERY_GAIN).min(100.0);
                evs.push((who, Ev::Battery));
            }
        }
        for p in &mut self.books {
            if !p.taken && near(p) {
                p.taken = true;
                car.study_s = STUDY_S;
                evs.push((who, Ev::Books));
            }
        }
        // Holding a flag already, leave the next one standing for later.
        if !car.flag_held {
            for p in &mut self.flags {
                if !p.taken && near(p) {
                    p.taken = true;
                    car.flag_held = true;
                    evs.push((who, Ev::Flag));
                    break;
                }
            }
        }

        // Sitting in the oil keeps you in it: the skid only restarts once the
        // last one has worn off, so the way out is to leave the slick.
        if car.grounded
            && car.alt <= 0.0
            && car.skid_s <= 0.0
            && self.slicks.iter().any(|s| (s.x - cx).abs() < SLICK_RANGE)
        {
            car.skid_s = SKID_S;
            evs.push((who, Ev::Oil));
        }

        for s in &mut self.rainbows {
            s.cool = (s.cool - dt).max(0.0);
            // Painted on the road, so a car up on a deck passes over them.
            if car.alt <= 0.0 && car.x >= s.x0 && car.x <= s.x1 && s.cool <= 0.0 {
                s.cool = 3.0;
                car.boost_s = BOOST_S;
                evs.push((who, Ev::Rainbow));
            }
        }

        let inside_sun = self.sun.iter().any(|&(a, b)| car.x >= a && car.x <= b);
        if inside_sun {
            car.battery = (car.battery + SUN_CHARGE * dt).min(100.0);
            if !car.in_sun {
                evs.push((who, Ev::Sun));
            }
        }
        car.in_sun = inside_sun;

        if inp.action {
            let near = self
                .spots
                .iter_mut()
                .filter(|s| !s.hung && spot_in_reach(s, car))
                .min_by(|a, b| (a.x - cx).abs().total_cmp(&(b.x - cx).abs()));
            if let Some(s) = near {
                s.hung = true;
                car.hung += 1;
                evs.push((who, Ev::Poster));
            }
        }

        // The boost is the point of the rainbow: distance that costs nothing.
        car.battery -= DRAIN_IDLE * dt;
        if car.boost_s <= 0.0 {
            let gust = 1.0 - wind * WIND_DRAIN;
            car.battery -= DRAIN_FULL_SPEED * (car.vx.abs() / MAX_SPEED) * gust * dt;
        }
        car.battery = car.battery.clamp(0.0, 100.0);

        if car.x + CAR_W / 2.0 >= self.len {
            car.finished = true;
            car.vx = 0.0;
        }
    }

    /// End the run once there is nothing left to play for: someone reached
    /// Christiansborg, or nobody can move again.
    fn settle(&mut self, evs: &mut Vec<(usize, Ev)>) {
        let done = self.cars.iter().any(|c| c.finished);
        let stuck = self.cars.iter().all(|c| c.stranded() || c.finished);
        if !done && !stuck {
            return;
        }
        let (hung, total) = (self.cars[0].hung, self.spots.len());
        self.outcome = Some(if self.racing() {
            let (a, b) = (self.cars[0].hung, self.cars[1].hung);
            Outcome::Race {
                winner: match a.cmp(&b) {
                    std::cmp::Ordering::Greater => Some(0),
                    std::cmp::Ordering::Less => Some(1),
                    std::cmp::Ordering::Equal => None,
                },
                hung: (a, b),
            }
        } else if !done {
            Outcome::Stranded
        } else if self.level + 1 < LEVELS {
            Outcome::LevelCleared {
                level: self.level,
                hung,
                total,
            }
        } else {
            let (hung, total) = (self.banked.0 + hung, self.banked.1 + total);
            Outcome::Campaign {
                tier: tier_of(hung, total),
                hung,
                total,
            }
        });
        self.phase = Phase::Over;
        evs.push((0, Ev::Over));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;
    /// Apex of a standing jump, the ceiling every platform deck and lamp-post
    /// height in the course is chosen against.
    const JUMP_APEX: f64 = JUMP_VY * JUMP_VY / (2.0 * GRAVITY);

    /// A hand-built course with one of everything at known places. The
    /// mechanics are tested against this rather than a generated level, so a
    /// mechanic test cannot start failing because a seed stopped rolling the
    /// feature it needed. The generator gets its own tests below.
    fn fixture(players: usize) -> World {
        let mut w = World::blank(0, 7, players, 12000.0);
        let ground = |x: f64| Pickup {
            x,
            alt: 0.0,
            taken: false,
        };
        w.spots = vec![
            Spot {
                x: 700.0,
                alt: 0.0,
                kind: SpotKind::Board,
                hung: false,
            },
            Spot {
                x: 2900.0,
                alt: 160.0,
                kind: SpotKind::Lamp,
                hung: false,
            },
            Spot {
                x: 4900.0,
                alt: 130.0,
                kind: SpotKind::Ledge,
                hung: false,
            },
            Spot {
                x: 8950.0,
                alt: 140.0,
                kind: SpotKind::Ledge,
                hung: false,
            },
        ];
        w.platforms = vec![
            Platform {
                x0: 4650.0,
                x1: 5150.0,
                top: 130.0,
                motion: None,
                offset: 0.0,
            },
            {
                let motion = Motion {
                    span: 620.0,
                    speed: 0.62,
                    phase: 0.0,
                };
                let offset = motion.offset_at(0.0);
                Platform {
                    x0: 8500.0,
                    x1: 8800.0,
                    top: 140.0,
                    motion: Some(motion),
                    offset,
                }
            },
        ];
        w.barriers = [2150.0, 4400.0]
            .map(|x| Barrier {
                x,
                lift: 0.0,
                opening: false,
                hit_cool: 0.0,
            })
            .into();
        w.batteries = [1900.0, 3450.0, 6500.0, 8000.0, 9400.0, 10700.0]
            .map(ground)
            .into();
        w.batteries.push(Pickup {
            x: 5050.0,
            alt: 130.0,
            taken: false,
        });
        w.books = vec![ground(2650.0)];
        w.flags = vec![ground(3800.0)];
        w.rainbows = vec![Strip {
            x0: 1150.0,
            x1: 1400.0,
            cool: 0.0,
        }];
        w.winds = vec![
            Wind {
                x0: 5900.0,
                x1: 6900.0,
                force: 1.0,
            },
            Wind {
                x0: 7100.0,
                x1: 8100.0,
                force: -1.0,
            },
        ];
        w.sun = vec![(6000.0, 6900.0)];
        w.truck = Some(Truck {
            x: 5850.0,
            end: 6980.0,
            rolling: false,
            woke: false,
            drop_cd: 0.0,
        });
        w.toll = Some(Toll {
            x: 11050.0,
            open: false,
            hit_cool: 0.0,
        });
        w.phase = Phase::Playing;
        w
    }

    fn playing() -> World {
        fixture(1)
    }

    fn racing() -> World {
        fixture(2)
    }

    fn run(w: &mut World, secs: f64, inp: Input) -> Vec<(usize, Ev)> {
        let mut evs = Vec::new();
        for _ in 0..(secs / DT) as usize {
            evs.extend(w.tick(DT, &[inp, inp]));
        }
        evs
    }

    fn saw(evs: &[(usize, Ev)], ev: Ev) -> bool {
        evs.iter().any(|&(_, e)| e == ev)
    }

    fn right() -> Input {
        Input {
            right: true,
            ..Default::default()
        }
    }

    fn act() -> Input {
        Input {
            action: true,
            ..Default::default()
        }
    }

    /// The car, in a one-player world.
    fn car(w: &World) -> &Car {
        &w.cars[0]
    }

    fn toll_x(w: &World) -> f64 {
        w.toll.as_ref().expect("fixture has a bridge").x
    }

    fn truck_x(w: &World) -> f64 {
        w.truck.as_ref().expect("fixture has a truck").x
    }

    #[test]
    fn driving_drains_the_battery() {
        let mut w = playing();
        run(&mut w, 3.0, right());
        assert!(car(&w).x > 400.0, "car moved: {}", car(&w).x);
        let b = car(&w).battery;
        assert!((75.0..99.0).contains(&b), "drained some: {b}");
    }

    #[test]
    fn battery_pickup_recharges_once() {
        let mut w = playing();
        w.cars[0].battery = 40.0;
        w.cars[0].x = w.batteries[0].x - 200.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::Battery));
        assert!(car(&w).battery > 60.0, "recharged: {}", car(&w).battery);
        assert!(w.batteries[0].taken);
        w.cars[0].x = w.batteries[0].x - 200.0;
        let evs = run(&mut w, 1.0, right());
        assert!(!saw(&evs, Ev::Battery));
    }

    #[test]
    fn barrier_blocks_a_grounded_car() {
        let mut w = playing();
        let bx = w.barriers[0].x;
        w.cars[0].x = bx - 150.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::Border));
        assert!(car(&w).x < bx, "still on the left: {} < {bx}", car(&w).x);
    }

    #[test]
    fn full_speed_jump_clears_a_barrier() {
        let mut w = playing();
        let bx = w.barriers[0].x;
        w.cars[0].x = bx - 200.0;
        w.cars[0].vx = MAX_SPEED;
        let jump = Input {
            right: true,
            jump: true,
            ..Default::default()
        };
        let mut evs = w.tick(DT, &[jump]);
        evs.extend(run(&mut w, 1.2, right()));
        assert!(!saw(&evs, Ev::Border), "no border stop");
        assert!(car(&w).x > bx, "landed past: {} > {bx}", car(&w).x);
    }

    #[test]
    fn eu_flag_lifts_the_next_barrier() {
        let mut w = playing();
        let fx = w.flags[0].x;
        let bx = w.barriers[1].x;
        w.cars[0].x = fx - 100.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 3.0, right());
        assert!(saw(&evs, Ev::Flag));
        assert!(saw(&evs, Ev::BarrierOpen));
        assert!(!saw(&evs, Ev::Border));
        assert!(car(&w).x > bx, "waved through: {} > {bx}", car(&w).x);
        assert!(!car(&w).flag_held, "the flag is spent");
    }

    #[test]
    fn poster_hangs_only_in_range_and_once() {
        let mut w = playing();
        w.cars[0].x = w.spots[0].x + 50.0;
        let evs = w.tick(DT, &[act()]);
        assert!(saw(&evs, Ev::Poster));
        assert_eq!(car(&w).hung, 1);
        let evs = w.tick(DT, &[act()]);
        assert!(!saw(&evs, Ev::Poster));
        assert_eq!(car(&w).hung, 1);

        w.cars[0].x = 100.0;
        let evs = w.tick(DT, &[act()]);
        assert!(!saw(&evs, Ev::Poster), "nothing in range");
    }

    #[test]
    fn a_jump_lands_the_car_on_a_platform_and_it_falls_off_the_end() {
        let mut w = playing();
        let p = (w.platforms[0].x0, w.platforms[0].x1, w.platforms[0].top);
        w.cars[0].x = p.0 - 120.0;
        w.cars[0].vx = 300.0;
        let jump = Input {
            right: true,
            jump: true,
            ..Default::default()
        };
        w.tick(DT, &[jump]);
        run(&mut w, 0.9, right());
        assert!(car(&w).grounded, "settled on the deck");
        assert!(
            (car(&w).alt - p.2).abs() < 1.0,
            "at deck height: {}",
            car(&w).alt
        );
        assert!(
            car(&w).x > p.0 && car(&w).x < p.1,
            "on the deck: {}",
            car(&w).x
        );

        run(&mut w, 2.0, right());
        assert!(car(&w).x > p.1, "left the deck: {}", car(&w).x);
        assert_eq!(car(&w).alt, 0.0, "back on the road");
    }

    #[test]
    fn the_road_passes_under_a_platform() {
        let mut w = playing();
        let p0 = w.platforms[0].x0;
        w.cars[0].x = p0 - 200.0;
        w.cars[0].vx = MAX_SPEED;
        run(&mut w, 1.5, right());
        assert!(car(&w).x > p0, "drove on: {}", car(&w).x);
        assert_eq!(
            car(&w).alt,
            0.0,
            "stayed on the road, not lifted onto the deck"
        );
    }

    #[test]
    fn a_moving_deck_carries_its_rider() {
        let mut w = playing();
        let i = w
            .platforms
            .iter()
            .position(|p| p.motion.is_some())
            .expect("course has a moving deck");
        // Park on the deck and hold still: any change in x is the deck's doing.
        let (x0, x1) = w.platforms[i].span();
        w.cars[0].x = (x0 + x1) * 0.5;
        w.cars[0].alt = w.platforms[i].top;
        w.tick(DT, &[Input::default()]);
        assert_eq!(car(&w).riding, Some(i), "standing on the deck");
        let before = car(&w).x;
        run(&mut w, 0.6, Input::default());
        assert!(
            (car(&w).x - before).abs() > 20.0,
            "carried along: {before} -> {}",
            car(&w).x
        );
        assert!(car(&w).grounded, "still aboard");
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
        w.cars[0].x = lx;
        let evs = w.tick(DT, &[act()]);
        assert!(!saw(&evs, Ev::Poster), "out of reach from the ground");

        // Jump under it and hang the poster near the top of the arc.
        w.tick(
            DT,
            &[Input {
                jump: true,
                ..Default::default()
            }],
        );
        let mut hung = false;
        for _ in 0..(1.2 / DT) as usize {
            let inp = Input {
                action: (car(&w).alt - lalt).abs() < POSTER_REACH_V,
                ..Default::default()
            };
            hung |= saw(&w.tick(DT, &[inp]), Ev::Poster);
        }
        assert!(hung, "hung it from the air");
    }

    #[test]
    fn books_widen_the_reach() {
        let mut w = playing();
        let sx = w.spots[0].x;
        // A gap outside the plain reach but inside the studied one.
        let gap = POSTER_RANGE + 30.0;
        assert!(gap < POSTER_RANGE * STUDY_MUL);
        w.cars[0].x = sx + gap;
        assert!(
            !saw(&w.tick(DT, &[act()]), Ev::Poster),
            "too far to argue from"
        );

        w.cars[0].study_s = STUDY_S;
        assert!(
            saw(&w.tick(DT, &[act()]), Ev::Poster),
            "in reach once studied"
        );
    }

    #[test]
    fn books_are_picked_up_and_run_out() {
        let mut w = playing();
        w.cars[0].x = w.books[0].x - 150.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::Books));
        assert!(car(&w).study_s > 0.0, "studying");
        run(&mut w, STUDY_S + 0.5, Input::default());
        assert_eq!(car(&w).study_s, 0.0, "wears off");
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
        w.cars[0].battery = 40.0;
        w.cars[0].x = bx;
        run(&mut w, 0.5, Input::default());
        assert!(!w.batteries[high].taken, "not scooped up from below");

        w.cars[0].alt = balt;
        w.tick(DT, &[Input::default()]);
        assert!(w.batteries[high].taken, "collected from the deck");
    }

    #[test]
    fn standing_still_still_drains_the_battery() {
        let mut w = playing();
        run(&mut w, 5.0, Input::default());
        let b = car(&w).battery;
        assert!((92.0..99.0).contains(&b), "idles down slowly: {b}");
    }

    #[test]
    fn the_fossil_truck_bolts_and_oils_the_road() {
        let mut w = playing();
        let parked = truck_x(&w);
        w.cars[0].x = parked - TRUCK_WAKE - 50.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::TruckBolts), "it noticed the car behind it");
        assert!(!w.slicks.is_empty(), "dropping oil");
        assert!(truck_x(&w) > parked, "and running");
    }

    #[test]
    fn oil_crosses_the_steering_until_it_wears_off() {
        let mut w = playing();
        w.slicks.push(Slick { x: 3000.0 });
        w.cars[0].x = 2960.0;
        w.cars[0].vx = 200.0;
        let evs = w.tick(DT, &[right()]);
        assert!(saw(&evs, Ev::Oil));

        // Holding "forward" now drives backwards, which is the whole hazard.
        let before = car(&w).vx;
        run(&mut w, 0.4, right());
        assert!(
            car(&w).vx < before,
            "throttle works against you: {before} -> {}",
            car(&w).vx
        );

        // Off the oil, the skid times out and the controls come back.
        w.cars[0].x = 4000.0;
        run(&mut w, SKID_S, Input::default());
        assert_eq!(car(&w).skid_s, 0.0, "grip comes back");
        let before = car(&w).vx;
        run(&mut w, 0.3, right());
        assert!(car(&w).vx > before, "and forward is forward again");
    }

    #[test]
    fn wind_pushes_and_changes_what_the_drive_costs() {
        let head = w_drain(-1.0);
        let tail = w_drain(1.0);
        assert!(
            head > tail,
            "a headwind costs more than a tailwind: {head} vs {tail}"
        );

        /// Charge spent driving one second inside a wind zone of `force`.
        fn w_drain(force: f64) -> f64 {
            let mut w = playing();
            let zone = w
                .winds
                .iter()
                .position(|z| z.force == force)
                .expect("course has this wind");
            w.cars[0].x = w.winds[zone].x0 + 60.0;
            w.cars[0].vx = MAX_SPEED;
            let before = w.cars[0].battery;
            run(&mut w, 1.0, right());
            before - w.cars[0].battery
        }
    }

    #[test]
    fn wind_announces_itself_on_entry() {
        let mut w = playing();
        let zone = w.winds.iter().position(|z| z.force > 0.0).unwrap();
        w.cars[0].x = w.winds[zone].x0 - 120.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::Wind(true)), "a tailwind is announced");
    }

    /// Roll a car up to the bridge gate with the given charge, past the last
    /// battery and rainbow so only the toll moves the numbers.
    fn at_the_gate(battery: f64) -> World {
        let mut w = playing();
        for p in &mut w.batteries {
            p.taken = true;
        }
        w.cars[0].x = toll_x(&w) - 260.0;
        w.cars[0].vx = MAX_SPEED;
        w.cars[0].battery = battery;
        w
    }

    #[test]
    fn an_eu_flag_waives_the_bridge_toll() {
        let mut w = at_the_gate(80.0);
        w.cars[0].flag_held = true;
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::TollWaived));
        assert!(!car(&w).flag_held, "the flag paid for it");
        assert!(car(&w).battery > 80.0 - TOLL_COST, "and the charge did not");
        assert!(car(&w).x > toll_x(&w), "let through");
    }

    #[test]
    fn without_a_flag_the_toll_takes_charge() {
        let mut w = at_the_gate(80.0);
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::TollPaid));
        assert!(car(&w).battery < 80.0 - TOLL_COST, "it was charged");
        assert!(car(&w).x > toll_x(&w), "and let through");
    }

    #[test]
    fn a_car_too_flat_to_pay_is_turned_back() {
        let mut w = at_the_gate(TOLL_COST - 5.0);
        let evs = run(&mut w, 1.0, right());
        assert!(saw(&evs, Ev::TollBlocked));
        assert!(car(&w).x < toll_x(&w), "held at the gate: {}", car(&w).x);
    }

    #[test]
    fn empty_battery_strands_the_car() {
        let mut w = playing();
        w.cars[0].battery = 1.0;
        w.cars[0].vx = MAX_SPEED;
        let evs = run(&mut w, 6.0, right());
        assert!(saw(&evs, Ev::Over));
        assert_eq!(w.outcome, Some(Outcome::Stranded));
    }

    /// Put the car on the finish line with the gate already open, so only the
    /// arrival is under test.
    fn at_the_line(w: &mut World, back: f64) {
        if let Some(toll) = w.toll.as_mut() {
            toll.open = true;
        }
        let len = w.len;
        for (i, c) in w.cars.iter_mut().enumerate() {
            c.x = len - back - 700.0 * i as f64;
            c.vx = MAX_SPEED;
        }
    }

    #[test]
    fn clearing_a_level_offers_the_next_one() {
        let mut w = playing();
        w.cars[0].hung = 3;
        at_the_line(&mut w, 120.0);
        run(&mut w, 1.0, right());
        assert_eq!(
            w.outcome,
            Some(Outcome::LevelCleared {
                level: 0,
                hung: 3,
                total: w.spots.len()
            }),
            "level one of five is not the end of the campaign"
        );
    }

    #[test]
    fn the_last_level_scores_every_level_together() {
        for (banked, hung, tier) in [
            ((18, 20), 3, Tier::Landslide),
            ((10, 20), 2, Tier::Elected),
            ((2, 20), 1, Tier::BelowThreshold),
        ] {
            let mut w = playing();
            w.level = LEVELS - 1;
            w.banked = banked;
            w.cars[0].hung = hung;
            at_the_line(&mut w, 120.0);
            run(&mut w, 1.0, right());
            let total = banked.1 + w.spots.len();
            assert_eq!(
                w.outcome,
                Some(Outcome::Campaign {
                    tier,
                    hung: banked.0 + hung,
                    total
                }),
                "{banked:?} banked + {hung} -> {tier:?}"
            );
        }
    }

    #[test]
    fn a_race_ends_when_the_first_car_arrives_and_posters_decide_it() {
        let mut w = racing();
        at_the_line(&mut w, 200.0);
        w.cars[0].hung = 2;
        w.cars[1].hung = 5;
        run(&mut w, 1.5, right());
        assert_eq!(w.phase, Phase::Over, "the arrival ended it");
        assert_eq!(
            w.outcome,
            Some(Outcome::Race {
                winner: Some(1),
                hung: (2, 5)
            }),
            "posters win the race, not the finish line"
        );
    }

    #[test]
    fn a_race_keeps_both_cars_on_one_screen() {
        let mut w = racing();
        w.cars[0].vx = MAX_SPEED;
        run(&mut w, 6.0, right());
        let gap = (w.cars[0].x - w.cars[1].x).abs();
        assert!(gap <= RACE_SPREAD + 1.0, "dragged along: {gap}");
    }

    #[test]
    fn one_spot_cannot_be_hung_by_both_racers() {
        let mut w = racing();
        let sx = w.spots[0].x;
        w.cars[0].x = sx;
        w.cars[1].x = sx;
        w.tick(DT, &[act(), act()]);
        assert_eq!(w.cars[0].hung + w.cars[1].hung, 1, "one poster, one board");
    }

    /// A competent driver that ignores every poster: flat out, jumping the
    /// barriers and steering out of the oil. It is the floor a course has to
    /// clear, since a level nobody can even drive is not a level.
    fn autopilot(w: &mut World, budget_s: f64) -> bool {
        let barriers: Vec<f64> = w.barriers.iter().map(|b| b.x).collect();
        for _ in 0..(budget_s / DT) as usize {
            let c = car(w);
            let near_barrier = barriers
                .iter()
                .any(|&bx| bx - c.x > 0.0 && bx - c.x < 210.0);
            let skidding = c.skid_s > 0.0;
            let inp = Input {
                right: !skidding,
                left: skidding,
                jump: near_barrier && c.grounded && c.vx > 380.0,
                ..Default::default()
            };
            w.tick(DT, &[inp]);
            if w.phase != Phase::Playing {
                return w.cars[0].finished;
            }
        }
        false
    }

    #[test]
    fn the_fixture_course_is_beatable_on_collected_power() {
        let mut w = playing();
        assert!(autopilot(&mut w, 140.0), "arrived at x {}", car(&w).x);
    }

    #[test]
    fn every_generated_level_can_be_driven_to_the_end() {
        // The generator's one hard promise. Twelve campaigns across all five
        // levels, each driven by the autopilot on the charge it can pick up
        // from the road alone.
        for seed in 0..12u64 {
            for level in 0..LEVELS {
                let mut w = World::generate(level, seed.wrapping_mul(0x9E37_79B9), 1);
                w.phase = Phase::Playing;
                let budget = w.len / 200.0 + 40.0;
                assert!(
                    autopilot(&mut w, budget),
                    "seed {seed} level {level} (len {}) stalled at x {} on {}%",
                    w.len,
                    car(&w).x,
                    car(&w).battery.round()
                );
            }
        }
    }

    #[test]
    fn a_generated_level_is_reproducible_and_seed_specific() {
        let shape = |w: &World| {
            (
                w.spots.iter().map(|s| s.x as i64).collect::<Vec<_>>(),
                w.barriers.iter().map(|b| b.x as i64).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            shape(&World::generate(2, 99, 1)),
            shape(&World::generate(2, 99, 1)),
            "the same seed replays the same course"
        );
        assert_ne!(
            shape(&World::generate(2, 99, 1)),
            shape(&World::generate(2, 100, 1)),
            "a different seed is a different course"
        );
        assert_ne!(
            shape(&World::generate(2, 99, 1)),
            shape(&World::generate(3, 99, 1)),
            "so is a different level of the same campaign"
        );
    }

    #[test]
    fn generated_levels_grow_and_add_their_features_in_order() {
        let mut last_len = 0.0;
        for level in 0..LEVELS {
            let w = World::generate(level, 4242, 1);
            assert!(w.len > last_len, "level {level} is longer than the last");
            last_len = w.len;
            assert!(!w.spots.is_empty(), "level {level} has boards");
            assert!(!w.barriers.is_empty(), "level {level} has barriers");

            let has = |k: SpotKind| w.spots.iter().any(|s| s.kind == k);
            assert_eq!(has(SpotKind::Lamp), level >= 1, "poles from level two");
            assert_eq!(w.truck.is_some(), level >= 2, "the lorry from level three");
            assert_eq!(w.toll.is_some(), level >= 3, "the bridge from level four");
            assert_eq!(
                w.platforms.iter().any(|p| p.motion.is_some()),
                level >= 3,
                "moving decks from level four"
            );
        }
    }

    #[test]
    fn generated_courses_stay_within_reach() {
        for seed in 0..24u64 {
            let level = (seed as usize) % LEVELS;
            let w = World::generate(level, seed.wrapping_mul(31), 1);

            for p in &w.platforms {
                assert!(
                    p.top < JUMP_APEX,
                    "seed {seed}: a deck at {} is above the jump",
                    p.top
                );
            }
            // A pole is either reachable from the road, or from a deck under it.
            for s in w.spots.iter().filter(|s| s.kind == SpotKind::Lamp) {
                let from_road = s.alt < JUMP_APEX + POSTER_REACH_V;
                let from_deck = w.platforms.iter().any(|p| {
                    s.x >= p.x0 - 120.0
                        && s.x <= p.x1 + 120.0
                        && s.alt < p.top + JUMP_APEX + POSTER_REACH_V
                });
                assert!(from_road || from_deck, "seed {seed}: a pole at {}", s.alt);
            }
            // Every ledge over a moving deck must sit inside its travel.
            for s in w.spots.iter().filter(|s| s.kind == SpotKind::Ledge) {
                let landed_on = w.platforms.iter().any(|p| {
                    (p.top - s.alt).abs() < 1.0
                        && s.x >= p.x0
                        && s.x <= p.x1 + p.motion.as_ref().map_or(0.0, |m| m.span)
                });
                assert!(landed_on, "seed {seed}: a ledge at {} has no deck", s.x);
            }
        }
    }

    #[test]
    fn a_campaign_banks_each_level_and_a_retry_keeps_it() {
        let mut w = World::campaign(5, 1);
        w.cars[0].hung = 4;
        let first_total = w.spots.len();
        w.next_level();
        assert_eq!(w.level, 1, "moved on");
        assert_eq!(w.banked, (4, first_total), "the cleared level is banked");
        assert_eq!(car(&w).hung, 0, "the new level starts empty");

        let shape: Vec<i64> = w.spots.iter().map(|s| s.x as i64).collect();
        w.cars[0].hung = 2;
        w.retry_level();
        assert_eq!(w.banked, (4, first_total), "a retry costs only this level");
        assert_eq!(car(&w).hung, 0);
        assert_eq!(
            w.spots.iter().map(|s| s.x as i64).collect::<Vec<_>>(),
            shape,
            "and replays the same course"
        );
    }
}

// Everything below is the wasm rendering/input layer.

const CANVAS_ID: &str = "radikal-game-canvas";

/// Phase mirrored into a signal so the DOM overlays re-render only on change,
/// while the canvas repaints every frame without touching Dioxus.
#[derive(Clone, Copy, PartialEq)]
enum UiPhase {
    Intro,
    Playing,
    Over(Outcome),
}

fn ui_phase(w: &World) -> UiPhase {
    match (w.phase, w.outcome) {
        (Phase::Intro, _) => UiPhase::Intro,
        (Phase::Over, Some(o)) => UiPhase::Over(o),
        _ => UiPhase::Playing,
    }
}

/// Held keys and unconsumed edges for one player.
#[derive(Default)]
struct Keys {
    left: bool,
    right: bool,
    jump_down: bool,
    act_down: bool,
    jump_edge: bool,
    act_edge: bool,
}

impl Keys {
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

    fn press_jump(&mut self) {
        if !self.jump_down {
            self.jump_edge = true;
        }
        self.jump_down = true;
    }

    fn press_act(&mut self) {
        if !self.act_down {
            self.act_edge = true;
        }
        self.act_down = true;
    }
}

/// Arrow keys are always player one; the letter keys are player two, and are
/// folded into player one when nobody is sitting in that seat.
#[derive(Default)]
struct Held {
    p: [Keys; 2],
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
        for c in &w.cars {
            if c.boost_s > 0.0 {
                for (i, col) in RAINBOW.iter().enumerate() {
                    self.parts.push(Particle {
                        x: c.x - CAR_W / 2.0,
                        y: BASE_Y - c.alt - 60.0 + (i as f64) * 7.0,
                        vx: -c.vx * 0.4,
                        vy: 0.0,
                        ttl: 0.5,
                        age: 0.0,
                        size: 6.0,
                        grav: 0.0,
                        color: col,
                    });
                }
            }
        }
        if let Some(truck) = w.truck.as_ref().filter(|t| t.rolling) {
            let (x, y) = (truck.x + 96.0, BASE_Y - 118.0);
            self.parts.push(Particle {
                x,
                y,
                vx: -60.0,
                vy: -40.0,
                ttl: 1.1,
                age: 0.0,
                size: 9.0,
                grav: -30.0,
                color: "#5A5A5A",
            });
        }
        let celebrating = matches!(
            w.outcome,
            Some(
                Outcome::Campaign {
                    tier: Tier::Landslide | Tier::Elected,
                    ..
                } | Outcome::LevelCleared { .. }
                    | Outcome::Race { .. }
            )
        );
        if w.phase == Phase::Over && celebrating {
            self.firework_s -= dt;
            if self.firework_s <= 0.0 {
                self.firework_s = 0.5;
                let x = w.len + 150.0 + self.rand() * 800.0;
                let y = 60.0 + self.rand() * 180.0;
                self.burst(x, y, 36, 260.0, &RAINBOW);
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
    // A race is framed on the pair; a solo run sits the car left of centre so
    // the road ahead is what fills the screen.
    let (focus, lead) = if w.racing() {
        ((w.cars[0].x + w.cars[1].x) * 0.5, 0.5)
    } else {
        (w.cars[0].x, 0.38)
    };
    let cam = (focus - view_w * lead).clamp(0.0, (w.len + RUN_OFF - view_w).max(0.0));

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
    draw_wind(ctx, w, cam, view_w, anim_t);
    draw_entities(ctx, w, spr, cam, anim_t);
    let boot_empty = w.spots.iter().all(|s| s.hung);
    for (idx, c) in w.cars.iter().enumerate() {
        let style = CarStyle {
            idx,
            racing: w.racing(),
            boot_empty,
        };
        draw_car(ctx, c, &style, spr, cam, anim_t);
    }
    draw_fx(ctx, fx, cam, view_w);

    // Election-night mood: a grey wash when the result disappoints.
    match w.outcome {
        Some(Outcome::Campaign {
            tier: Tier::BelowThreshold,
            ..
        }) => {
            ctx.set_fill_style_str("rgba(90, 90, 100, 0.45)");
            ctx.fill_rect(0.0, 0.0, view_w, WORLD_H);
        }
        Some(Outcome::Stranded) => {
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
    for i in 0..10 {
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
    for i in 0..10 {
        let wx = 650.0 + f64::from(i) * 1500.0;
        blit(ctx, &spr.house, wx - cam * f, HORIZON - 212.0, 300.0, 218.0);
    }
    for i in 0..16 {
        let wx = 200.0 + f64::from(i) * 900.0;
        blit(ctx, &spr.trees, wx - cam * f, HORIZON - 230.0, 280.0, 234.0);
    }
}

/// Where the water starts and stops, around the toll booth that gates it.
/// Empty on a level with no bridge.
fn bridge_span(w: &World) -> Option<(f64, f64)> {
    w.toll.as_ref().map(|t| (t.x - 230.0, t.x + 250.0))
}

fn draw_ground(ctx: &web_sys::CanvasRenderingContext2d, w: &World, cam: f64, view_w: f64) {
    ctx.set_fill_style_str("#8CC152");
    ctx.fill_rect(0.0, HORIZON, view_w, ROAD_TOP - HORIZON);
    ctx.set_fill_style_str("#4B4F54");
    ctx.fill_rect(0.0, ROAD_TOP, view_w, WORLD_H - ROAD_TOP);

    // The water the bridge crosses, drawn over the road it replaces. It has to
    // stop short of the finish, or Christiansborg stands in the sea.
    let bridge = bridge_span(w);
    if let Some((w0, w1)) = bridge {
        ctx.set_fill_style_str("#4E86B8");
        ctx.fill_rect(w0 - cam, HORIZON, w1 - w0, WORLD_H - HORIZON);
        ctx.set_fill_style_str("#6FA3CD");
        let mut wx = w0 + 40.0;
        while wx < w1 - 70.0 {
            ctx.fill_rect(wx - cam, 486.0, 70.0, 5.0);
            wx += 120.0;
        }
        ctx.set_fill_style_str("#6E767E");
        ctx.fill_rect(w0 - cam, ROAD_TOP, w1 - w0, 14.0);
        ctx.fill_rect(w0 - cam, 502.0, w1 - w0, 9.0);
    }

    ctx.set_fill_style_str("#F4F4F4");
    let dash = 150.0;
    let first = (cam / dash).floor() * dash;
    let mut x = first;
    while x < cam + view_w + dash {
        // No lane markings on the bridge deck: it has its own kerb lines.
        let on_bridge = bridge.is_some_and(|(w0, w1)| x >= w0 - dash && x <= w1);
        if !on_bridge {
            ctx.fill_rect(x - cam, 502.0, 62.0, 7.0);
        }
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

    for s in &w.slicks {
        ctx.set_global_alpha(0.75);
        ctx.set_fill_style_str("#241E2B");
        ctx.fill_rect(s.x - cam - SLICK_RANGE, 494.0, SLICK_RANGE * 2.0, 15.0);
        ctx.set_fill_style_str("#6C4F86");
        ctx.fill_rect(s.x - cam - SLICK_RANGE + 12.0, 497.0, 28.0, 5.0);
        ctx.set_global_alpha(1.0);
    }
}

/// Wind as streaks blowing the way it pushes, so the cost of a stretch is
/// visible before the battery explains it.
fn draw_wind(
    ctx: &web_sys::CanvasRenderingContext2d,
    w: &World,
    cam: f64,
    view_w: f64,
    anim_t: f64,
) {
    for z in &w.winds {
        if z.x1 < cam || z.x0 > cam + view_w {
            continue;
        }
        ctx.set_global_alpha(0.55);
        ctx.set_stroke_style_str(if z.force > 0.0 { "#FFFFFF" } else { "#6B7A8C" });
        ctx.set_line_width(3.0);
        for i in 0..14 {
            let row = f64::from(i);
            // Euclidean, so a headwind's negative drift still lands inside the
            // zone rather than off its left edge.
            let drift = (anim_t * 210.0 * z.force + row * 137.0).rem_euclid(z.x1 - z.x0);
            let x = z.x0 + drift - cam;
            let y = 150.0 + row * 21.0;
            ctx.begin_path();
            ctx.move_to(x, y);
            ctx.line_to(x + 46.0 * z.force, y);
            ctx.stroke();
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
        w.len + 50.0 - cam,
        ROAD_TOP - 450.0,
        1060.0,
        450.0,
    );
    blit(
        ctx,
        &spr.flag_dk,
        w.len - 120.0 - cam,
        ROAD_TOP - 135.0,
        66.0,
        135.0,
    );
    blit(
        ctx,
        &spr.flag_eu,
        w.len - 45.0 - cam,
        ROAD_TOP - 135.0,
        66.0,
        135.0,
    );

    for p in &w.platforms {
        let (x0, x1) = p.span();
        let (x0, x1) = (x0 - cam, x1 - cam);
        let deck = BASE_Y - p.top;
        if p.motion.is_some() {
            // Drawn as a vehicle, so a moving deck reads as something to catch
            // rather than to climb.
            ctx.set_fill_style_str("#C9463C");
            ctx.fill_rect(x0, deck, x1 - x0, 74.0);
            ctx.set_fill_style_str("#E8E8E8");
            ctx.fill_rect(x0 + 14.0, deck + 14.0, (x1 - x0) * 0.34, 30.0);
            ctx.fill_rect(x1 - (x1 - x0) * 0.28, deck + 14.0, (x1 - x0) * 0.2, 30.0);
            ctx.set_fill_style_str("#2B2B2B");
            for wx in [x0 + 34.0, x1 - 56.0] {
                ctx.fill_rect(wx, deck + 66.0, 22.0, 20.0);
            }
        } else {
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
            SpotKind::Board => {
                ctx.set_fill_style_str("#9AA0A6");
                ctx.fill_rect(
                    x - 3.0,
                    panel_y + 44.0,
                    6.0,
                    BASE_Y - s.alt - panel_y - 44.0,
                );
            }
            // A ledge over a moving deck has nothing to stand on, so it hangs
            // from a short bracket rather than a leg to the road.
            SpotKind::Ledge => {
                let footed = w.platforms.iter().any(|p| {
                    p.motion.is_none() && (p.top - s.alt).abs() < 1.0 && s.x >= p.x0 && s.x <= p.x1
                });
                let leg = if footed {
                    BASE_Y - s.alt - panel_y - 44.0
                } else {
                    36.0
                };
                ctx.set_fill_style_str("#9AA0A6");
                ctx.fill_rect(x - 3.0, panel_y + 44.0, 6.0, leg);
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
            let ready = live && w.cars.iter().any(|c| spot_in_reach(s, c));
            let near = live
                && w.cars
                    .iter()
                    .any(|c| (s.x - c.x).abs() < POSTER_RANGE * 1.6);
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

    draw_toll(ctx, w, cam);
    draw_truck(ctx, w, cam);

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
    for p in &w.books {
        if !p.taken {
            let bob = 8.0 * (anim_t * 3.0 + p.x * 0.01).sin();
            let (x, y) = (p.x - cam - 26.0, BASE_Y - p.alt - 108.0 + bob);
            for (i, c) in ["#C2185B", "#1E88E5", "#43A047"].iter().enumerate() {
                ctx.set_fill_style_str(c);
                ctx.fill_rect(x, y + (i as f64) * 15.0, 52.0, 12.0);
                ctx.set_fill_style_str("#FFFFFF");
                ctx.fill_rect(x + 4.0, y + (i as f64) * 15.0 + 3.0, 44.0, 3.0);
            }
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

fn draw_toll(ctx: &web_sys::CanvasRenderingContext2d, w: &World, cam: f64) {
    let Some(toll) = w.toll.as_ref() else { return };
    let x = toll.x - cam;
    ctx.set_fill_style_str("#EDEDED");
    ctx.fill_rect(x - 96.0, BASE_Y - 128.0, 56.0, 128.0);
    ctx.set_fill_style_str("#8FC4E8");
    ctx.fill_rect(x - 86.0, BASE_Y - 112.0, 36.0, 32.0);
    ctx.set_fill_style_str("#24408E");
    ctx.fill_rect(x - 96.0, BASE_Y - 150.0, 56.0, 22.0);
    ctx.set_fill_style_str("#FFD617");
    ctx.fill_rect(x - 74.0, BASE_Y - 144.0, 11.0, 11.0);

    ctx.save();
    let _ = ctx.translate(x - 40.0, BASE_Y - 96.0);
    let _ = ctx.rotate(if toll.open { -1.25 } else { 0.0 });
    ctx.set_fill_style_str("#D8D8D8");
    ctx.fill_rect(0.0, 0.0, 150.0, 12.0);
    ctx.set_fill_style_str("#C9463C");
    for i in 0..4 {
        ctx.fill_rect(f64::from(i) * 38.0 + 10.0, 0.0, 19.0, 12.0);
    }
    ctx.restore();
}

fn draw_truck(ctx: &web_sys::CanvasRenderingContext2d, w: &World, cam: f64) {
    let Some(truck) = w.truck.as_ref() else {
        return;
    };
    let x = truck.x - cam;
    ctx.set_fill_style_str("#3B3F46");
    ctx.fill_rect(x - 110.0, BASE_Y - 112.0, 150.0, 84.0);
    ctx.set_fill_style_str("#5A6068");
    ctx.fill_rect(x + 40.0, BASE_Y - 82.0, 62.0, 54.0);
    ctx.set_fill_style_str("#8FC4E8");
    ctx.fill_rect(x + 62.0, BASE_Y - 74.0, 32.0, 24.0);
    ctx.set_fill_style_str("#2B2B2B");
    ctx.fill_rect(x + 86.0, BASE_Y - 132.0, 13.0, 26.0);
    ctx.set_fill_style_str("#1E1E1E");
    for wx in [x - 86.0, x - 24.0, x + 62.0] {
        ctx.fill_rect(wx, BASE_Y - 30.0, 30.0, 30.0);
    }
    // The drum on the tailgate: the cargo, and the hazard it leaves behind.
    ctx.set_fill_style_str("#111111");
    ctx.fill_rect(x - 92.0, BASE_Y - 96.0, 40.0, 46.0);
    ctx.set_fill_style_str("#C9463C");
    ctx.fill_rect(x - 84.0, BASE_Y - 86.0, 24.0, 8.0);
}

/// What tells one car from another on screen, and what it is carrying.
struct CarStyle {
    idx: usize,
    racing: bool,
    boot_empty: bool,
}

fn draw_car(
    ctx: &web_sys::CanvasRenderingContext2d,
    car: &Car,
    style: &CarStyle,
    spr: &Sprites,
    cam: f64,
    anim_t: f64,
) {
    let CarStyle {
        idx,
        racing,
        boot_empty,
    } = *style;
    let x = car.x - cam;
    let shadow_w = (145.0 - car.alt * 0.3).max(80.0);
    ctx.set_global_alpha((0.5 - car.alt * 0.002).max(0.15));
    blit(
        ctx,
        &spr.shadow,
        x - shadow_w / 2.0,
        BASE_Y + 1.0,
        shadow_w,
        11.0,
    );
    ctx.set_global_alpha(1.0);

    let body = if boot_empty {
        &spr.car
    } else {
        &spr.car_poster
    };
    let top = BASE_Y - CAR_H - car.alt;
    ctx.save();
    let _ = ctx.translate(x, top + CAR_H / 2.0);
    // A skid slews the car; a jump pitches it.
    let lean = if car.skid_s > 0.0 {
        (anim_t * 22.0).sin() * 0.11
    } else {
        (-car.vy * 0.000_15).clamp(-0.1, 0.1)
    };
    let _ = ctx.rotate(lean);
    if racing && idx == 1 {
        ctx.set_filter("hue-rotate(155deg) saturate(1.4)");
    }
    blit(ctx, body, -CAR_W / 2.0, -CAR_H / 2.0, CAR_W, CAR_H);
    ctx.set_filter("none");
    ctx.restore();

    if racing {
        ctx.set_font("700 22px 'Atkinson Hyperlegible', system-ui, sans-serif");
        ctx.set_text_align("center");
        ctx.set_fill_style_str(if idx == 0 { "#E6007E" } else { "#0EA5A5" });
        let _ = ctx.fill_text(&format!("P{}", idx + 1), x, top - 12.0);
        ctx.set_text_align("start");
    }
    if car.study_s > 0.0 {
        ctx.set_global_alpha(0.4 + 0.3 * (anim_t * 5.0).sin());
        ctx.set_stroke_style_str("#1E88E5");
        ctx.set_line_width(3.0);
        let (rh, rv) = car.reach();
        ctx.stroke_rect(x - rh, top + CAR_H / 2.0 - rv, rh * 2.0, rv * 2.0);
        ctx.set_global_alpha(1.0);
    }
    if car.flag_held {
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
    let total = w.spots.len();
    for (i, c) in w.cars.iter().enumerate() {
        let y = 14.0 + (i as f64) * 46.0;
        blit(ctx, &spr.hud_poster, 20.0, y, 26.0, 42.0);
        ctx.set_font("700 30px 'Atkinson Hyperlegible', system-ui, sans-serif");
        ctx.set_fill_style_str(if w.racing() && i == 1 {
            "#0A7C7C"
        } else {
            "#20242A"
        });
        let label = if w.racing() {
            format!("P{} {}/{}", i + 1, c.hung, total)
        } else {
            format!("{}/{}  ·  L{}/{}", c.hung, total, w.level + 1, LEVELS)
        };
        let _ = ctx.fill_text(&label, 58.0, y + 32.0);

        // The battery bar: a fill inside the sprite frame, amber then pulsing
        // red as the charge runs out.
        let (bx, by, bw2, bh2) = (view_w - 226.0, y, 200.0, 30.0);
        let frac = c.battery / 100.0;
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
}

fn toast_for(ev: Ev) -> Option<String> {
    let key = match ev {
        Ev::Battery => "game.toastBattery",
        Ev::Books => "game.toastBooks",
        Ev::Flag => "game.toastFlag",
        Ev::BarrierOpen => "game.toastOpen",
        Ev::Border => "game.toastBorder",
        Ev::Rainbow => "game.toastRainbow",
        Ev::Sun => "game.toastSun",
        Ev::Poster => "game.toastPoster",
        Ev::Oil => "game.toastOil",
        Ev::TruckBolts => "game.toastTruck",
        Ev::TollPaid => "game.toastToll",
        Ev::TollWaived => "game.toastTollFree",
        Ev::TollBlocked => "game.toastTollBlocked",
        Ev::Wind(true) => "game.toastTailwind",
        Ev::Wind(false) => "game.toastHeadwind",
        Ev::Over => return None,
    };
    Some(t(key))
}

/// GameApp: the campaign minigame, node-independent like `?app=feedback` so
/// `/?app=game` works signed out. Kept off the app rail on purpose, in the
/// spirit of the `cow` easter egg.
#[component]
pub fn GameApp() -> Element {
    let mut phase = use_signal(|| UiPhase::Intro);
    let world = use_hook(|| Rc::new(RefCell::new(World::generate(0, 1, 1))));
    let held = use_hook(|| Rc::new(RefCell::new(Held::default())));
    let fx = use_hook(|| Rc::new(RefCell::new(Fx::new())));
    let sprites = use_hook(|| Rc::new(Sprites::load()));
    // Read by the input loop, which has no access to the signal's reader.
    let players = use_hook(|| Rc::new(Cell::new(1usize)));

    // A fresh campaign of freshly generated levels: the clock is the only
    // entropy a wasm page has, and it only has to differ between playthroughs.
    let start = {
        let world = world.clone();
        let fx = fx.clone();
        let players = players.clone();
        move |n: usize| {
            players.set(n);
            let seed = js_sys::Date::now() as u64;
            *world.borrow_mut() = World::campaign(seed, n);
            *fx.borrow_mut() = Fx::new();
            phase.set(UiPhase::Playing);
        }
    };

    // Carry on inside the campaign already running, rather than reseeding it.
    let advance = {
        let world = world.clone();
        let fx = fx.clone();
        move |next: bool| {
            let mut w = world.borrow_mut();
            if next {
                w.next_level();
            } else {
                w.retry_level();
            }
            drop(w);
            *fx.borrow_mut() = Fx::new();
            phase.set(UiPhase::Playing);
        }
    };

    {
        let world = world.clone();
        let held = held.clone();
        let fx = fx.clone();
        let sprites = sprites.clone();
        let players = players.clone();
        use_future(move || {
            let world = world.clone();
            let held = held.clone();
            let fx = fx.clone();
            let sprites = sprites.clone();
            let players = players.clone();
            async move {
                let mut last = js_sys::Date::now();
                loop {
                    gloo_timers::future::TimeoutFuture::new(16).await;
                    let now = js_sys::Date::now();
                    // Clamped so a background tab does not fast-forward on return.
                    let dt = ((now - last) / 1000.0).clamp(0.0, 0.05);
                    last = now;

                    let inputs = {
                        let mut h = held.borrow_mut();
                        let (a, b) = (h.p[0].snapshot(), h.p[1].snapshot());
                        if players.get() > 1 {
                            vec![a, b]
                        } else {
                            vec![Input::merged(a, b)]
                        }
                    };
                    let evs = world.borrow_mut().tick(dt, &inputs);
                    {
                        let w = world.borrow();
                        let mut fx = fx.borrow_mut();
                        for &(who, ev) in &evs {
                            let x = w.cars.get(who).map_or(0.0, |c| c.x);
                            if let Some(text) = toast_for(ev) {
                                fx.toast(text, x);
                            }
                            match ev {
                                Ev::Rainbow => fx.burst(x, 400.0, 24, 220.0, &RAINBOW),
                                Ev::Poster => {
                                    fx.burst(x, 330.0, 14, 160.0, &["#E6007E", "#FFFFFF"])
                                }
                                Ev::Books => fx.burst(x, 380.0, 12, 140.0, &["#1E88E5", "#FFFFFF"]),
                                Ev::Oil => fx.burst(x, 500.0, 16, 120.0, &["#241E2B", "#6C4F86"]),
                                _ => {}
                            }
                        }
                        fx.tick(dt, &w);
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
            let playing = *phase.peek() == UiPhase::Playing;
            let mut h = held.borrow_mut();
            match evt.key() {
                Key::ArrowRight => {
                    h.p[0].right = true;
                    evt.prevent_default();
                }
                Key::ArrowLeft => {
                    h.p[0].left = true;
                    evt.prevent_default();
                }
                Key::ArrowUp => {
                    h.p[0].press_jump();
                    evt.prevent_default();
                }
                Key::ArrowDown => {
                    h.p[0].press_act();
                    evt.prevent_default();
                }
                Key::Enter => {
                    if playing {
                        h.p[0].press_act();
                    } else {
                        drop(h);
                        start(1);
                    }
                    evt.prevent_default();
                }
                Key::Character(c) => match c.as_str() {
                    "d" | "D" => h.p[1].right = true,
                    "a" | "A" => h.p[1].left = true,
                    "w" | "W" => h.p[1].press_jump(),
                    " " => {
                        h.p[0].press_jump();
                        evt.prevent_default();
                    }
                    "s" | "S" | "e" | "E" => h.p[1].press_act(),
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
                Key::ArrowRight => h.p[0].right = false,
                Key::ArrowLeft => h.p[0].left = false,
                Key::ArrowUp => h.p[0].jump_down = false,
                Key::ArrowDown | Key::Enter => h.p[0].act_down = false,
                Key::Character(c) => match c.as_str() {
                    "d" | "D" => h.p[1].right = false,
                    "a" | "A" => h.p[1].left = false,
                    "w" | "W" => h.p[1].jump_down = false,
                    " " => h.p[0].jump_down = false,
                    "s" | "S" | "e" | "E" => h.p[1].act_down = false,
                    _ => {}
                },
                _ => {}
            }
        }
    };

    // Touch controls drive player one; a race needs two keyboards' worth of
    // hands, which a phone does not have.
    let hold = |field: fn(&mut Keys, bool)| {
        let held = held.clone();
        move |down: bool| field(&mut held.borrow_mut().p[0], down)
    };
    let hold_left = hold(|k, v| k.left = v);
    let hold_right = hold(|k, v| k.right = v);
    let tap_jump = {
        let held = held.clone();
        move || held.borrow_mut().p[0].jump_edge = true
    };
    let tap_act = {
        let held = held.clone();
        move || held.borrow_mut().p[0].act_edge = true
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
                    move |_| start(1)
                },
                alt_button: Some(t("game.race")),
                on_alt: {
                    let mut start = start.clone();
                    move |_| start(2)
                },
            }
        },
        UiPhase::Over(outcome) => {
            let (title, body) = outcome_text(outcome);
            // What the run offers next depends on where it stopped: another
            // level, the same one again, or a whole new campaign.
            let (primary, onward) = match outcome {
                Outcome::LevelCleared { .. } => (t("game.nextLevel"), Some(true)),
                Outcome::Stranded => (t("game.retryLevel"), Some(false)),
                _ => (t("game.again"), None),
            };
            rsx! {
                GameOverlay {
                    title,
                    body,
                    extra: None,
                    button: primary,
                    on_click: {
                        let mut start = start.clone();
                        let mut advance = advance.clone();
                        let n = players.get();
                        move |_| match onward {
                            Some(next) => advance(next),
                            None => start(n),
                        }
                    },
                    alt_button: Some(t("game.newCampaign")),
                    on_alt: {
                        let mut start = start.clone();
                        move |_| start(1)
                    },
                }
            }
        }
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

fn outcome_text(outcome: Outcome) -> (String, String) {
    match outcome {
        Outcome::LevelCleared { level, hung, total } => {
            let (n, total) = (hung.to_string(), total.to_string());
            let (done, all) = ((level + 1).to_string(), LEVELS.to_string());
            (
                t_with("game.levelCleared", &[("level", done.as_str())]),
                t_with(
                    "game.levelScore",
                    &[
                        ("n", n.as_str()),
                        ("total", total.as_str()),
                        ("next", (level + 2).to_string().as_str()),
                        ("all", all.as_str()),
                    ],
                ),
            )
        }
        Outcome::Campaign { tier, hung, total } => {
            let (n, total) = (hung.to_string(), total.to_string());
            let args = [("n", n.as_str()), ("total", total.as_str())];
            let (title, body) = match tier {
                Tier::Landslide => ("game.wonTitleLandslide", "game.wonLandslide"),
                Tier::Elected => ("game.wonTitleElected", "game.wonElected"),
                Tier::BelowThreshold => ("game.wonTitleBelow", "game.wonBelow"),
            };
            (t(title), t_with(body, &args))
        }
        Outcome::Stranded => (t("game.lostTitle"), t("game.lost")),
        Outcome::Race { winner, hung } => {
            let (a, b) = (hung.0.to_string(), hung.1.to_string());
            let args = [("a", a.as_str()), ("b", b.as_str())];
            let title = match winner {
                Some(0) => t_with("game.raceWon", &[("p", "1")]),
                Some(_) => t_with("game.raceWon", &[("p", "2")]),
                None => t("game.raceDraw"),
            };
            (title, t_with("game.raceScore", &args))
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
    alt_button: Option<String>,
    on_alt: EventHandler<()>,
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
                    if !body.is_empty() {
                        p { style: "margin:0;", "{body}" }
                    }
                    if let Some(extra) = extra {
                        p { class: "body-medium text-muted", style: "margin:0;", "{extra}" }
                    }
                    div { style: "display:flex; gap:10px; justify-content:center; flex-wrap:wrap;",
                        button {
                            class: "btn btn-primary",
                            autofocus: true,
                            onclick: move |_| on_click.call(()),
                            "{button}"
                        }
                        if let Some(alt) = alt_button {
                            button {
                                class: "btn btn-outlined",
                                onclick: move |_| on_alt.call(()),
                                "{alt}"
                            }
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
