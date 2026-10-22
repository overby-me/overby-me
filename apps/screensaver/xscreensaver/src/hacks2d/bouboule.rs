//! Port of `hacks/bouboule.c`.
//!
//! ```text
//! Ported from xlockmore 4.03a12 to be a standalone program and thus usable
//! with xscreensaver by Jamie Zawinski <jwz@jwz.org> on 15-May-97.
//!
//! Original copyright notice from xlock.c:
//!
//!  * Copyright (c) 1988-91 by Patrick J. Naughton.
//!  *
//!  * Permission to use, copy, modify, and distribute this software and its
//!  * documentation for any purpose and without fee is hereby granted,
//!  * provided that the above copyright notice appear in all copies and that
//!  * both that copyright notice and this permission notice appear in
//!  * supporting documentation.
//!  *
//!  * This file is provided AS IS with no warranties of any kind.  The author
//!  * shall have no liability with respect to the infringement of copyrights,
//!  * trade secrets or any patents by this file or any part thereof.  In no
//!  * event will the author be liable for any lost revenue or profits or
//!  * other special, indirect and consequential damages.
//!
//! bouboule.c (bouboule mode for xlockmore)
//!
//! Sort of starfield for xlockmore. I found that making a starfield for
//! a 3D engine and thought it could be a nice lock mode. For a real starfield,
//! I only scale the sort of sphere you see to the whole sky and clip the stars
//! to the camera screen.
//!
//!   Code Copyright 1996 by Jeremie PETIT (jeremie_petit@geocities.com)
//! ```
//!
//! Spots scattered over a sphere that is not drawn. Each spot is a unit
//! vector, the whole set is turned about all three axes at once, and the
//! result is squashed onto the screen by two independent radii, so what you
//! see is a balloon being pushed about by something you cannot see.
//!
//! Nothing in it moves at a constant rate. Every quantity that could vary,
//! the centre of the balloon, its two radii, the three rotation angles, is a
//! sine wave between a minimum and a maximum, and the *speed* of that sine
//! wave is itself a sine wave, applied four times out of five. That is why it
//! never settles into a rhythm.
//!
//! Two limits are re-imposed every frame rather than set once: the balloon may
//! not be wider than the room it has to the nearer edge of the screen, and
//! neither radius may be more than twice the other, so it can be squashed but
//! not to a line.
//!
//! Upstream times two ways of erasing the previous frame against each other
//! for the first hundred frames and keeps the faster: painting out the old
//! spots one by one, or one rectangle over their bounding box. They leave the
//! same picture behind, so this uses the first, which is the one upstream
//! prefers when the two are close.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::BLACK;
use crate::runtime::xlockmore::{ColorScheme, ModeInfo, nrand};
use crate::runtime::{
    About, Dpy, Opt, Pixel, Runner, SaverDef, Screenhack, StartArgs, XArc, XEvent,
};

const MINSTARS: i32 = 1;
const MINSIZE: i32 = 1;
/// How often we change colors. This value should be tuned to the number of
/// stars; jwz thinks slower colour changes look better.
const COLOR_CHANGES: i16 = 50;
/// Whether the sphere can be very large and have a small height, or the
/// opposite.
const MAX_SIZEX_SIZEY: f64 = 2.0;

/// Percentage of changes for the speed of change of the three theta values.
const THETACANRAND: i32 = 80;
/// The same, for sizex and sizey.
const SIZECANRAND: i32 = 80;
/// The same, for x and y.
const POSCANRAND: i32 = 80;

const VARRANDMIN: f64 = -70.0;
const VARRANDMAX: f64 = 70.0;

/// Stars can come this close.
const MINZVAL: f64 = 100.0;
/// This is where the screen is.
const SCREENZ: f64 = 2000.0;
/// Stars can go this far away.
const MAXZVAL: f64 = 10000.0;

fn varrand_alpha() -> f64 {
    nrand((std::f64::consts::PI * 1000.0) as i32) as f64 / 1000.0
}

fn varrand_step() -> f64 {
    std::f64::consts::PI / (nrand(100) as f64 + 100.0)
}

/// A value that swings between two bounds, at a speed that swings too.
#[derive(Default, Clone)]
struct SinVariable {
    /// The current state, between zero and two pi.
    alpha: f64,
    /// Speed of evolution of alpha: a reasonable fraction of two pi.
    step: f64,
    minimum: f64,
    maximum: f64,
    value: f64,
    /// Percentage of frames on which the speed itself is varied. Zero means
    /// the speed is fixed, which is what the inner variable uses.
    mayrand: i32,
    /// The variation of alpha, one level deep.
    varrand: Option<Box<SinVariable>>,
}

impl SinVariable {
    fn vary(&mut self) {
        self.value = self.minimum + (self.maximum - self.minimum) * (self.alpha.sin() + 1.0) / 2.0;

        if self.mayrand == 0 {
            self.alpha += self.step;
        } else {
            let (mayrand, step) = (self.mayrand, self.step);
            let vaval = nrand(100);
            if let Some(vr) = self.varrand.as_deref_mut() {
                if vaval <= mayrand {
                    vr.vary();
                }
                let v = vr.value;
                self.alpha += (100.0 + v) * step / 100.0;
            }
        }

        if self.alpha > std::f64::consts::TAU {
            self.alpha -= std::f64::consts::TAU;
        }
    }

    fn init(&mut self, alpha: f64, step: f64, minimum: f64, maximum: f64, mayrand: i32) {
        self.alpha = alpha;
        self.step = step;
        self.minimum = minimum;
        self.maximum = maximum;
        self.mayrand = mayrand;
        if mayrand != 0 {
            let mut vr = self.varrand.take().unwrap_or_default();
            // Upstream passes these as arguments to one call, so which of the
            // two draws its random number first is up to the compiler.
            let (a, s) = (varrand_alpha(), varrand_step());
            vr.init(a, s, VARRANDMIN, VARRANDMAX, 0);
            vr.vary();
            self.varrand = Some(vr);
        }
        // We calculate the values at least once for initialization.
        self.vary();
    }
}

#[derive(Default, Clone, Copy)]
struct Star {
    /// A unit vector: where the spot sits on the invisible sphere.
    x: f64,
    y: f64,
    z: f64,
    size: i32,
}

struct State {
    mi: ModeInfo,
    width: i32,
    height: i32,
    /// Centre of the field on the screen, and its depth.
    x: SinVariable,
    y: SinVariable,
    z: SinVariable,
    /// Half width and half height of the field.
    sizex: SinVariable,
    sizey: SinVariable,
    /// Rotation angles of the starfield about its own axes.
    thetax: SinVariable,
    thetay: SinVariable,
    thetaz: SinVariable,
    star: Vec<Star>,
    xarc: Vec<XArc>,
    xarcleft: Vec<XArc>,
    oldxarc: Vec<XArc>,
    oldxarcleft: Vec<XArc>,
    color: Pixel,
    colorp: i32,
    colorchange: i16,
    use3d: bool,
    delta3d: f64,
    right3d: Pixel,
    left3d: Pixel,
    delay: u32,
}

impl State {
    /// How far apart the two eyes see a spot at this depth.
    fn zdiff(&self, z: f64) -> f64 {
        self.delta3d * 20.0 * (1.0 - SCREENZ / (z + 1000.0))
    }

    /// Lay the evolving variables out for a window of this size.
    fn init_vars(&mut self) {
        let (w, h) = (self.width as f64, self.height as f64);

        self.x.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(100) as f64 + 100.0),
            w / 4.0,
            3.0 * w / 4.0,
            POSCANRAND,
        );
        self.y.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(100) as f64 + 100.0),
            h / 4.0,
            3.0 * h / 4.0,
            POSCANRAND,
        );

        // For z we have to ensure that the bouboule does not get behind the
        // eyes of the viewer, which are at zero. Because the bouboule uses
        // the x-radius for the z-radius too, we use the x-values.
        self.z.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(100) as f64 + 100.0),
            w / 2.0 + MINZVAL,
            w / 2.0 + MAXZVAL,
            POSCANRAND,
        );

        self.sizex.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(100) as f64 + 100.0),
            (w - self.x.value).min(self.x.value) / 5.0,
            (w - self.x.value).min(self.x.value),
            SIZECANRAND,
        );

        // Upstream reads sizey's own maximum here before it has been set, so
        // on a fresh field that term is zero.
        let sizey_max_before = self.sizey.maximum;
        self.sizey.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(100) as f64 + 100.0),
            (self.sizex.value / MAX_SIZEX_SIZEY).max(sizey_max_before / 5.0),
            (self.sizex.value * MAX_SIZEX_SIZEY).min((h - self.y.value).min(self.y.value)),
            SIZECANRAND,
        );

        self.thetax.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(200) as f64 + 200.0),
            -std::f64::consts::PI,
            std::f64::consts::PI,
            THETACANRAND,
        );
        self.thetay.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(200) as f64 + 200.0),
            -std::f64::consts::PI,
            std::f64::consts::PI,
            THETACANRAND,
        );
        self.thetaz.init(
            nrand(3142) as f64 / 1000.0,
            std::f64::consts::PI / (nrand(400) as f64 + 400.0),
            -std::f64::consts::PI,
            std::f64::consts::PI,
            THETACANRAND,
        );
    }
}

/// Degrees to radians.
fn dtor(x: f64) -> f64 {
    x * std::f64::consts::PI / 180.0
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    // SMOOTH_COLORS, from the #define above the xlockmore.h include.
    let mi = ModeInfo::new(d, ColorScheme::Smooth);
    let (width, height) = (d.width(), d.height());

    let mut size = mi.size;
    if width > 2560 || height > 2560 {
        size *= 2; // Retina displays.
    }
    let max_star_size = if size < -MINSIZE {
        nrand(-size - MINSIZE + 1) + MINSIZE
    } else if size < MINSIZE {
        MINSIZE
    } else {
        size
    };

    let mut nb_stars = mi.count;
    if nb_stars < -MINSTARS {
        nb_stars = nrand(-nb_stars - MINSTARS + 1) + MINSTARS;
    } else if nb_stars < MINSTARS {
        nb_stars = MINSTARS;
    }
    let n = nb_stars as usize;

    let use3d = d.res.bool("use3d");
    let mut st = State {
        mi,
        width,
        height,
        x: SinVariable::default(),
        y: SinVariable::default(),
        z: SinVariable::default(),
        sizex: SinVariable::default(),
        sizey: SinVariable::default(),
        thetax: SinVariable::default(),
        thetay: SinVariable::default(),
        thetaz: SinVariable::default(),
        star: vec![Star::default(); n],
        xarc: vec![XArc::default(); n],
        xarcleft: vec![XArc::default(); n],
        oldxarc: vec![XArc::default(); n],
        oldxarcleft: vec![XArc::default(); n],
        color: BLACK,
        colorp: 0,
        colorchange: 0,
        use3d,
        delta3d: d.res.float("delta3d"),
        right3d: d.res.pixel("right3d"),
        left3d: d.res.pixel("left3d"),
        delay: d.res.int("delay").max(0) as u32,
    };
    d.clear_window();
    st.init_vars();

    // The stars are distributed over a sphere by elevation and bearing, which
    // is the one idea kept from the net3d starfield this grew out of.
    for i in 0..n {
        let theta = dtor(nrand(1800) as f64 / 10.0 - 90.0);
        let omega = dtor(nrand(3600) as f64 / 10.0 - 180.0);
        let star = &mut st.star[i];
        star.x = theta.cos() * omega.sin();
        star.y = omega.sin() * theta.sin();
        star.z = omega.cos();

        // Half the spots come out as the smallest size.
        star.size = nrand(2 * max_star_size);
        if star.size < max_star_size {
            star.size = 0;
        } else {
            star.size -= max_star_size;
        }

        let wh = 2 + star.size;
        for list in [
            &mut st.xarc,
            &mut st.xarcleft,
            &mut st.oldxarc,
            &mut st.oldxarcleft,
        ] {
            list[i] = XArc {
                x: 0,
                y: 0,
                width: wh,
                height: wh,
                angle1: 0,
                // We draw whole disks: from zero to three hundred and sixty
                // degrees.
                angle2: 360 * 64,
            };
        }
    }

    if st.mi.npixels() > 2 {
        st.colorp = nrand(st.mi.npixels());
    }
    st.color = if !use3d && st.mi.npixels() > 2 {
        st.mi.pixel(st.colorp as usize)
    } else {
        st.mi.white
    };

    Box::new(st)
}

impl Screenhack for State {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // We make variables vary.
        self.thetax.vary();
        self.thetay.vary();
        self.thetaz.vary();
        self.x.vary();
        self.y.vary();
        if self.use3d {
            self.z.vary();
        }

        // A little trick to prevent the bouboule from being bigger than the
        // screen.
        let (w, h) = (self.width as f64, self.height as f64);
        self.sizex.maximum = (w - self.x.value).min(self.x.value);
        self.sizex.minimum = self.sizex.maximum / 3.0;

        // Another trick to make the ball not too flat.
        self.sizey.minimum = (self.sizex.value / MAX_SIZEX_SIZEY).max(self.sizey.maximum / 3.0);
        self.sizey.maximum =
            (self.sizex.value * MAX_SIZEX_SIZEY).min((h - self.y.value).min(self.y.value));

        self.sizex.vary();
        self.sizey.vary();

        // The rotation is done on the fly rather than through a matrix: the
        // stars are unit vectors and all of them just turn.
        let (sx, cx) = self.thetax.value.sin_cos();
        let (sy, cy) = self.thetay.value.sin_cos();
        let (sz, cz) = self.thetaz.value.sin_cos();

        let (sizex, sizey) = (self.sizex.value, self.sizey.value);
        let (px, py, pz) = (self.x.value, self.y.value, self.z.value);

        for i in 0..self.star.len() {
            let star = self.star[i];
            let mut diff = 0;
            if self.use3d {
                // To help the eyes, the starfield is always as wide as it is
                // deep, so the x radius can be used for both.
                diff = self
                    .zdiff(sizex * ((sy * cx) * star.x + sx * star.y + (cx * cy) * star.z) + pz)
                    as i32;
            }

            let ax = (sizex
                * ((cy * cz - sx * sy * sz) * star.x
                    + (-cx * sz) * star.y
                    + (sy * cz + sz * sx * cy) * star.z)
                + px) as i32;
            let ay = (sizey
                * ((cy * sz + sx * sy * cz) * star.x
                    + (cx * cz) * star.y
                    + (sy * sz - sx * cy * cz) * star.z)
                + py) as i32;

            self.xarc[i].x = ax;
            self.xarc[i].y = ay;
            if self.use3d {
                self.xarcleft[i].x = ax;
                self.xarcleft[i].y = ay;
                self.xarc[i].x += diff;
                self.xarcleft[i].x -= diff;
            }
            if star.size != 0 {
                self.xarc[i].x -= star.size;
                self.xarc[i].y -= star.size;
                if self.use3d {
                    self.xarcleft[i].x -= star.size;
                    self.xarcleft[i].y -= star.size;
                }
            }
        }

        // First, we erase the previous starfield.
        self.mi.gc.set_foreground(self.mi.black);
        d.win().fill_arcs(&self.mi.gc, &self.oldxarc);
        if self.use3d {
            d.win().fill_arcs(&self.mi.gc, &self.oldxarcleft);
        }

        // Then we draw the new one.
        if self.use3d {
            self.mi.gc.set_foreground(self.right3d);
            d.win().fill_arcs(&self.mi.gc, &self.xarc);
            self.mi.gc.set_foreground(self.left3d);
            d.win().fill_arcs(&self.mi.gc, &self.xarcleft);
        } else {
            self.mi.gc.set_foreground(self.color);
            d.win().fill_arcs(&self.mi.gc, &self.xarc);
        }

        std::mem::swap(&mut self.xarc, &mut self.oldxarc);
        if self.use3d {
            std::mem::swap(&mut self.xarcleft, &mut self.oldxarcleft);
        }

        // We set up the color for the next drawing.
        if !self.use3d && self.mi.npixels() > 2 {
            self.colorchange += 1;
            if self.colorchange >= COLOR_CHANGES {
                self.colorchange = 0;
                self.colorp += 1;
                if self.colorp >= self.mi.npixels() {
                    self.colorp = 0;
                }
                self.color = self.mi.pixel(self.colorp as usize);
            }
        }

        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.mi.reshape(width, height);
        self.width = width;
        self.height = height;
        let (a, s) = (self.x.alpha, self.x.step);
        self.x.init(
            a,
            s,
            width as f64 / 4.0,
            3.0 * width as f64 / 4.0,
            POSCANRAND,
        );
        let (a, s) = (self.y.alpha, self.y.step);
        self.y.init(
            a,
            s,
            height as f64 / 4.0,
            3.0 * height as f64 / 4.0,
            POSCANRAND,
        );
    }

    fn event(&mut self, _d: &mut Dpy, _event: &XEvent) -> bool {
        false
    }
}

const DEFAULTS: &[&str] = &[
    "*count: 100",
    "*size: 15",
    "*delay: 20000",
    "*ncolors: 64",
    "*use3d: True",
    "*delta3d: 1.5",
    "*right3d: red",
    "*left3d: blue",
    "*both3d: magenta",
    "*none3d: black",
    "*fpsSolid: true",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("count", "Number of spots", 1.0, 400.0, 1.0, 0, "100"),
    Opt::slider("ncolors", "Number of colors", 1.0, 255.0, 1.0, 0, "64"),
    Opt::boolean("use3d", "Do Red/Blue 3D separation", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "bouboule",
    label: "Bouboule",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jeremie Petit",
        year: "1997",
        video: Some("https://www.youtube.com/watch?v=MdmIBmlkyFw"),
        blurb: "A deforming balloon with varying-sized spots painted on its invisible surface.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
