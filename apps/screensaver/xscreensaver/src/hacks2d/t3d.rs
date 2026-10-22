//! Port of `hacks/t3d.c`.
//!
//! ```text
//! t3d -- Flying Balls Clock Demo
//!    by Bernd Paysan , paysan@informatik.tu-muenchen.de
//!
//!    Copy, modify, and distribute T3D either under GPL  version 2 or newer,
//!    or under the standard MIT/X license notice.
//!
//!   partly based on flying balls demo by Georg Acher,
//!   acher@informatik.tu-muenchen.de
//!   (developed on HP9000/720 (55 MIPS,20 MFLOPS) )
//!   NO warranty at all ! Complaints to /dev/null !
//!
//!   4-Jan-99 jwz@jwz.org -- adapted to xscreensaver framework, to take
//!                           advantage of the command-line options provided
//!                           by screenhack.c.
//! ```
//!
//! A working analog clock with no face and no hands, only bubbles. Twenty-four
//! of them stand in a ring for the hours, alternating large and small with every
//! sixth one larger again, and each hand is three more bubbles marching outward
//! and shrinking by the golden ratio. One in the middle throbs once a second.
//!
//! The ring is not flat. Every bubble's height off the plane is a cosine of its
//! angle from the second hand, times a sine of a five-minute cycle, so a
//! standing wave rolls round the clock and reverses; and the whole assembly
//! tilts side to side on top of that. The clock is genuinely readable, and
//! genuinely hard to read.
//!
//! Each bubble is drawn as ten filled circles of decreasing radius, stepped
//! along the diagonal and stepping up a twelve-entry grey ramp, which is enough
//! to read as a lit sphere. They are painted back to front by distance, which
//! is the whole of the hidden-surface handling.
//!
//! Two upstream paths are not here. The fast one caches those stacks of circles
//! as sprites and stamps them with `GXor` through a mask, which is
//! `#ifndef HAVE_JWXYZ`: upstream's own modern builds draw the circles directly,
//! as this does, and the same picture comes out. The colour-cycling options
//! (`-hsvcycle`, `-rgb`, `-hsv`) are also gone; they animate a read-write
//! colormap that no TrueColor display has, they are absent from the settings
//! panel upstream too, and with upstream's own default saturation of zero the
//! cycle is a no-op regardless. The background cycling, which needs none of
//! that, is here.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{BLACK, XColor};
use crate::runtime::{
    About, Dpy, Gc, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent,
};

/// The length every basis vector is renormalised to each frame.
const NORM: f64 = 20.0;
/// How many bubbles the clock can have: the ring, the middle one and three
/// hands of three.
const MAX_BALLS: usize = 100;
/// How many entries the grey ramp has.
const COLORS: usize = 12;

#[derive(Clone, Copy, Default)]
struct Kugel {
    x: f64,
    y: f64,
    z: f64,
    r: f64,
    /// Distance from the eye, which is what they are sorted on.
    d: f64,
    /// Projected radius. Negative, because the eye is at negative zoom.
    r1: f64,
    x1: i32,
    y1: i32,
}

struct T3d {
    maxk: usize,
    timewait: u32,
    kugeln: [Kugel; MAX_BALLS],

    /// The eye, and the basis it looks along: `x` and `y` span the screen and
    /// `v` is their cross product.
    a: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
    v: [f64; 3],
    zoom: f64,
    speed: f64,
    vspeed: f64,
    vturn: f64,

    startx: i32,
    starty: i32,
    mag: f64,
    minutes: bool,
    cycl: bool,
    movef: f64,
    wobber: f64,
    cycle: f64,

    gc: Gc,
    scrn_width: i32,
    scrn_height: i32,
    buffer: Pixmap,
    colors: [XColor; COLORS],

    /// The pointer, tracked from events the way upstream polls for it with
    /// `XQueryPointer`.
    px: i32,
    py: i32,
    buttons: u32,
}

/// How many bubbles stand in the ring.
fn kmax(minutes: bool) -> usize {
    if minutes { 60 } else { 24 }
}

fn vektorprodukt(f1: [f64; 3], f2: [f64; 3]) -> [f64; 3] {
    [
        f1[1] * f2[2] - f1[2] * f2[1],
        f1[2] * f2[0] - f1[0] * f2[2],
        f1[0] * f2[1] - f1[1] * f2[0],
    ]
}

/// Rotate `f1` about `f2` by `winkel`.
fn turn(f1: &mut [f64; 3], f2: [f64; 3], winkel: f64) {
    let temp = vektorprodukt(*f1, f2);
    let s = f1[0] * f2[0] + f1[1] * f2[1] + f1[2] * f2[2];
    let sx = [s * f2[0], s * f2[1], s * f2[2]];
    let (sa, ca) = (winkel.sin(), winkel.cos());
    for i in 0..3 {
        f1[i] = ca * (f1[i] - sx[i]) + sa * temp[i] + sx[i];
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let mut maxk = 34;
    let minutes = d.res.bool("minutes");
    if minutes {
        maxk += 60 - 24;
    }

    let cycle = {
        let f = d.res.float("cycle");
        let f = if f <= 0.0 || f > 60.0 { 6.0 } else { f };
        60.0 / f
    };
    let mag = 10.0 * d.res.float("mag");

    // The grey ramp the bubbles are shaded with, at the white the hack ships
    // with. Upstream's read-write colormap animation of it is not here.
    let mut colors = [XColor::default(); COLORS];
    let (r, g, b) = (1.0f64, 1.0f64, 1.0f64);
    for (n1, c) in colors.iter_mut().enumerate() {
        let n = 30 + 3 * n1 as i32;
        *c = XColor::from_rgb16(
            (1023 + n * (1024.0 * r) as i32) as u16,
            (1023 + n * (1024.0 * g) as i32) as u16,
            (1023 + n * (1024.0 * b) as i32) as u16,
        );
    }

    let (w, h) = (d.width(), d.height());
    let mut st = T3d {
        maxk,
        timewait: d.res.int("delay").max(0) as u32,
        kugeln: [Kugel::default(); MAX_BALLS],
        a: [0.0, 0.0, -10.0],
        x: [10.0, 0.0, 0.0],
        y: [0.0, 10.0, 0.0],
        v: [0.0; 3],
        zoom: -10.0,
        speed: 0.0,
        vspeed: 0.0,
        vturn: 0.0,
        startx: w / 2,
        starty: h / 2,
        mag,
        minutes,
        cycl: d.res.bool("colcycle"),
        movef: d.res.float("move"),
        wobber: d.res.float("wobble"),
        cycle,
        gc: Gc::new(BLACK, BLACK),
        scrn_width: w,
        scrn_height: h,
        buffer: Pixmap::new(w, h),
        colors,
        px: 0,
        py: 0,
        buttons: 0,
    };

    st.gc.set_foreground(BLACK);
    st.buffer.fill_rectangle(&st.gc, 0, 0, w, h);
    st.v = vektorprodukt(st.x, st.y);
    d.win().fill_rectangle(&st.gc, 0, 0, w, h);
    Box::new(st)
}

impl T3d {
    /// Place the three bubbles of one hand, each further out and smaller than
    /// the last by the golden ratio.
    fn zeiger(&mut self, mut dist: f64, mut rad: f64, z: f64, sec: f64, n: &mut usize) {
        let gratio = (2.0 / (1.0 + 5.0f64.sqrt())).sqrt();
        for _ in 0..3 {
            self.kugeln[*n].x = dist * sec.cos();
            self.kugeln[*n].y = -dist * sec.sin();
            self.kugeln[*n].z = z;
            self.kugeln[*n].r = rad;
            *n += 1;

            dist += rad;
            rad *= gratio;
        }
    }

    /// Put every bubble where the clock says it should be, for the time `k` in
    /// seconds since midnight.
    fn manipulate(&mut self, k: f64) {
        let tau = std::f64::consts::TAU;
        let sec = tau * (k / 60.0).fract();
        let min = tau * (k / 3600.0).fract();
        let hour = tau * (k / 43200.0).fract();
        let l = tau * (k / 300.0).fract();

        let km = kmax(self.minutes);
        let mut i = 0.0f64;
        for n in 0..km {
            self.kugeln[n].x = 4.0 * i.sin();
            self.kugeln[n].y = 4.0 * i.cos();
            // A standing wave round the ring, keyed to the second hand, whose
            // number of lobes steps up over each five-minute cycle.
            self.kugeln[n].z = self.wobber
                * ((i - sec) * (2.0 + 5.0 * l / std::f64::consts::PI).floor()).cos()
                * (5.0 * l).sin();
            self.kugeln[n].r = if self.minutes {
                (if !n.is_multiple_of(5) { 0.3 } else { 0.6 })
                    * (if n.is_multiple_of(15) { 1.25 } else { 0.75 })
            } else {
                (if n & 1 != 0 { 0.5 } else { 1.0 })
                    * (if n.is_multiple_of(6) { 1.25 } else { 0.75 })
            };
            i += tau / km as f64;
        }

        let mut n = km;
        self.kugeln[n].x = 0.0;
        self.kugeln[n].y = 0.0;
        self.kugeln[n].z = 0.0;
        self.kugeln[n].r = 2.0 + (tau * k.fract()).cos() / 2.0;
        n += 1;

        self.zeiger(2.0, 0.75, -2.0, sec, &mut n);
        self.zeiger(1.0, 1.0, -1.5, min, &mut n);
        self.zeiger(0.0, 1.5, -1.0, hour, &mut n);

        // Tilt the whole clock side to side.
        let tilt = self.movef * (self.cycle * sec).sin();
        let (s, c) = (tilt.sin(), tilt.cos());
        for n in 0..self.maxk {
            let ys = self.kugeln[n].y * c + self.kugeln[n].z * s;
            let zs = -self.kugeln[n].y * s + self.kugeln[n].z * c;
            self.kugeln[n].y = ys;
            self.kugeln[n].z = zs;
        }
    }

    /// Upstream's own quicksort, kept rather than swapped for a library one so
    /// that bubbles at the same distance keep the same painting order.
    fn sort(&mut self, l: i32, r: i32) {
        let (mut i, mut j) = (l, r);
        let x = self.kugeln[((l + r) / 2) as usize].d;
        loop {
            while self.kugeln[i as usize].d > x {
                i += 1;
            }
            while x > self.kugeln[j as usize].d {
                j -= 1;
            }
            if i <= j {
                self.kugeln.swap(i as usize, j as usize);
                i += 1;
                j -= 1;
            }
            if i > j {
                break;
            }
        }
        if l < j {
            self.sort(l, j);
        }
        if i < r {
            self.sort(i, r);
        }
    }

    /// Draw one bubble: a stack of filled circles, each smaller than the last
    /// and a step brighter, offset along the diagonal so the highlight lands
    /// off-centre.
    fn fill_kugel(&mut self, i: usize) {
        let k = self.kugeln[i];
        let inr = if k.r1.abs() < 6.0 { 9 } else { 3 };

        let mut m = 0;
        while m <= 28 {
            let ra = k.r1 * (1.0 - (m * m) as f64 / (28.0 * 28.0)).sqrt();
            let mut col = if m == 27 { 33 } else { m };
            if col > 33 {
                col = 33;
            }
            col /= 3;
            self.gc.set_foreground(self.colors[col as usize].pixel);

            self.buffer.fill_arc(
                &self.gc,
                (k.x1 as f64 + (k.r1 + ra) / 2.0) as i32,
                (k.y1 as f64 + (k.r1 + ra) / 2.0) as i32,
                -(2.0 * ra + 1.0) as i32,
                -(2.0 * ra + 1.0) as i32,
                0,
                360 * 64,
            );
            m += inr;
        }
    }

    /// Project every bubble onto the screen, and record how far away it is.
    fn projektion(&mut self) {
        for i in 0..self.maxk {
            let c1 = [
                self.kugeln[i].x - self.a[0],
                self.kugeln[i].y - self.a[1],
                self.kugeln[i].z - self.a[2],
            ];
            let cnorm = (c1[0] * c1[0] + c1[1] * c1[1] + c1[2] * c1[2]).sqrt();
            let cno = c1[0] * self.v[0] + c1[1] * self.v[1] + c1[2] * self.v[2];

            self.kugeln[i].d = cnorm;
            if cno < 0.0 {
                self.kugeln[i].d = -20.0; // Behind the eye.
            }

            self.kugeln[i].r1 = self.mag * self.zoom * self.kugeln[i].r / cnorm;

            let c2 = [self.v[0] / cno, self.v[1] / cno, self.v[2] / cno];
            let k = vektorprodukt(c2, c1);

            let x1 = self.startx as f64
                + (self.x[0] * k[0] + self.x[1] * k[1] + self.x[2] * k[2]) * self.mag;
            let y1 = self.starty as f64
                - (self.y[0] * k[0] + self.y[1] * k[1] + self.y[2] * k[2]) * self.mag;
            if x1 > -2000.0
                && x1 < self.scrn_width as f64 + 2000.0
                && y1 > -2000.0
                && y1 < self.scrn_height as f64 + 2000.0
            {
                self.kugeln[i].x1 = x1 as i32;
                self.kugeln[i].y1 = y1 as i32;
            } else {
                self.kugeln[i].x1 = 0;
                self.kugeln[i].y1 = 0;
                self.kugeln[i].d = -20.0;
            }
        }
    }
}

impl Screenhack for T3d {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.v = vektorprodukt(self.x, self.y);

        // Renormalise the basis, which the turning slowly stretches.
        for f in [&mut self.v, &mut self.x, &mut self.y] {
            let n = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
            f[0] = f[0] * NORM / n;
            f[1] = f[1] * NORM / n;
            f[2] = f[2] * NORM / n;
        }

        self.projektion();
        self.sort(0, self.maxk as i32 - 1);

        let dtime = d.wall_clock();

        if self.cycl {
            let mut draw_color = (64.0 * (dtime / 60.0).fract()) as i32 - 32;
            if draw_color < 0 {
                draw_color = -draw_color;
            }
            let p = self.colors[(draw_color / 3) as usize].pixel;
            self.gc.set_foreground(p);
        } else {
            self.gc.set_foreground(BLACK);
        }
        let (w, h) = (self.scrn_width, self.scrn_height);
        self.buffer.fill_rectangle(&self.gc, 0, 0, w, h);

        self.manipulate(dtime);
        for i in 0..self.maxk {
            if self.kugeln[i].d > 0.0 {
                self.fill_kugel(i);
            }
        }

        self.gc.set_foreground(BLACK);
        d.win().copy_area(&self.gc, &self.buffer, 0, 0, w, h, 0, 0);

        // Upstream polls the pointer every frame; the same state is tracked
        // from events here.
        if self.px > 0 && self.px < self.scrn_width && self.py > 0 && self.py < self.scrn_height {
            if self.px != self.startx && self.buttons & 2 != 0 {
                let (mut y, x) = (self.y, self.x);
                turn(
                    &mut y,
                    x,
                    (self.px - self.startx) as f64 / (8000.0 * self.mag),
                );
                self.y = y;
            }
            if self.py != self.starty && self.buttons & 2 != 0 {
                let (mut x, y) = (self.x, self.y);
                turn(
                    &mut x,
                    y,
                    (self.py - self.starty) as f64 / (-8000.0 * self.mag),
                );
                self.x = x;
            }
            if self.buttons & 1 != 0 {
                self.bump_turn(1.0);
            }
            if self.buttons & 4 != 0 {
                self.bump_turn(-1.0);
            }
        }
        if self.buttons & 1 == 0 && self.buttons & 4 == 0 {
            self.vturn = 0.0;
        }

        self.speed += self.speed * self.vspeed;
        if self.speed < 0.0000001 && self.vspeed > 0.000001 {
            self.speed = 0.000001;
        }
        self.vspeed *= 0.1;
        if self.speed > 0.01 {
            self.speed = 0.01;
        }
        for i in 0..3 {
            self.a[i] += self.speed * self.v[i];
        }

        self.timewait
    }

    fn reshape(&mut self, d: &mut Dpy, w: i32, h: i32) {
        if w != self.scrn_width || h != self.scrn_height {
            self.scrn_width = w;
            self.scrn_height = h;
            self.buffer = d.new_pixmap(w, h);
            self.startx = w / 2;
            self.starty = h / 2;
        }
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match *event {
            XEvent::KeyPress { key } => match key.to_ascii_lowercase() {
                's' => {
                    self.vspeed = 0.5;
                    true
                }
                'a' => {
                    self.vspeed = -0.3;
                    true
                }
                '0' => {
                    self.speed = 0.0;
                    self.vspeed = 0.0;
                    true
                }
                'z' => {
                    self.mag *= 1.02;
                    true
                }
                'x' => {
                    self.mag /= 1.02;
                    true
                }
                _ => false,
            },
            XEvent::MotionNotify { x, y } => {
                self.px = x;
                self.py = y;
                false
            }
            XEvent::ButtonPress { x, y, button } => {
                self.px = x;
                self.py = y;
                self.buttons |= 1 << button.min(31);
                false
            }
            XEvent::ButtonRelease { x, y, button } => {
                self.px = x;
                self.py = y;
                self.buttons &= !(1 << button.min(31));
                false
            }
        }
    }
}

impl T3d {
    /// Spin the view about the direction it is looking, accelerating while the
    /// button is held.
    fn bump_turn(&mut self, dir: f64) {
        if self.vturn == 0.0 {
            self.vturn = 0.005;
        } else if self.vturn < 2.0 {
            self.vturn += 0.01;
        }
        let w = dir * 0.002 * self.vturn;
        let (mut x, mut y, v) = (self.x, self.y, self.v);
        turn(&mut x, v, w);
        turn(&mut y, v, w);
        self.x = x;
        self.y = y;
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*move: 0.5",
    "*wobble: 2.0",
    "*cycle: 10.0",
    "*mag: 1.0",
    "*minutes: False",
    "*delay: 40000",
    "*colcycle: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "40000").inverted(),
    Opt::slider("move", "Turn side-to-side", 0.0, 3.0, 0.1, 1, "0.5"),
    Opt::slider("wobble", "Wobbliness", 0.0, 3.0, 0.1, 1, "2.0"),
    Opt::slider("cycle", "Cycle seconds", 0.0, 60.0, 1.0, 0, "10.0"),
    Opt::slider("mag", "Magnification", 0.1, 4.0, 0.1, 1, "1.0"),
    Opt::boolean("minutes", "Minute tick marks", "False"),
    Opt::boolean("colcycle", "Cycle the background", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "t3d",
    label: "T3D",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Bernd Paysan",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=5UohH7U2CAI"),
        blurb: "Draws a working analog clock composed of floating, throbbing bubbles.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
