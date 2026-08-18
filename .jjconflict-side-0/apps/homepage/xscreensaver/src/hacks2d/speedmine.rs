//! Port of `hacks/speedmine.c`.
//!
//! ```text
//! speedmine, Copyright (C) 2001 Conrad Parker <conrad@deephackmode.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Written mostly over the Easter holiday, 2001. Psychedelic option due to
//! a night at Home nightclub, Sydney. Three all-nighters of solid partying
//! were involved in the week this hack was written.
//!
//! Hacking notes
//!
//! This program generates a rectangular terrain grid and maps this onto
//! a semi-circular tunnel. The terrain has length TERRAIN_LENGTH, which
//! corresponds to length along the tunnel, and breadth TERRAIN_BREADTH,
//! which corresponds to circumference around the tunnel. For each frame,
//! the tunnel is perspective mapped onto a set of X and Y screen values.
//! ```
//!
//! A mine shaft flown down at speed. The shaft is a rectangular height field
//! wrapped into a tube: one axis of the grid runs along the tunnel and the
//! other around it, and the height at each point becomes distance from the
//! axis, so rough terrain becomes a rocky wall. A quarter of the way around,
//! the wall is pushed out further to make the floor you appear to be flying
//! over.
//!
//! The terrain is endless because it is a ring. Only a quarter of it is ever
//! regenerated at once, and only when the viewpoint has just crossed out of
//! that quarter, so the shaft ahead is always new and the part behind is being
//! quietly rewritten. Each quarter is built by repeated subdivision: two corner
//! heights are rolled, then every midpoint between known heights is set near
//! their average with a perturbation that shrinks as the rectangles do. The
//! same subdivision, on one axis, gives the curvature of the tunnel and how
//! wide it opens.
//!
//! Drawing is one polygon per grid square, shaded by depth, by how steeply the
//! square is tilted, and by whether it belongs to the floor or the wall, out of
//! three ramps of thirty-two colours. Far squares are drawn coarser than near
//! ones: three depth bands step the grid by four, then two, then one, and the
//! seams between bands are stitched with pentagons.
//!
//! Flying is not steering. Gravity pulls along the tunnel's own vertical
//! curvature, so the speed rises going down a slope and falls going up one, and
//! thrust is added every frame up to a ceiling. The view leans into the bends
//! and, if bumps are on, is jolted by the terrain immediately under it.
//!
//! Scattered along the shaft are bonuses, drawn as a shower of sparks across
//! the tunnel mouth, each preceded three units earlier by a marker. Flying
//! through one picks one of seven effects at random: switch to and from
//! wireframe, a burst of speed, a full spin either way, a change of palette,
//! reverse the view so the shaft is flown backwards for a moment, or jam
//! against the same bonus three times so it happens three times over.
//!
//! The worm is the same code with the numbers changed: faster, ten times the
//! thrust, three times the gravity, twice the curvature and twist, always
//! psychedelic, and drawn only in the coarse band so the tube is chunky and
//! never fills the screen.
//!
//! Two things here are not upstream's. The sparks are clipped to the tunnel
//! mouth by testing each spark's centre against the polygon rather than by
//! painting the polygon through a bitmap mask, which this runtime has no
//! support for; at a spark two pixels across the difference is whether one
//! straddling the rim is cut or dropped. And the frame rate the tunnel's
//! advance is scaled by comes from the saver clock rather than the system one,
//! which under test is the requested frame interval exactly.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{Pixel, make_color_ramp, parse_color, rgb_to_hsv};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, SelectItem, StartArgs, XPoint, random,
};

/// Shades in each of the three ramps.
const MAX_COLORS: usize = 32;
/// Levels of interpolation between grid steps, for the perspective.
const INTERP: i32 = 32;
/// Both must be powers of two.
const TERRAIN_LENGTH: usize = 256;
const TERRAIN_BREADTH: usize = 32;
/// The total perspective distance of the terrain.
const TERRAIN_PDIST: i32 = INTERP * TERRAIN_LENGTH as i32;
const ROTS: usize = 1024;
const TB_MUL: usize = ROTS / TERRAIN_BREADTH;
const STEEL_ELEVATION: i32 = 300;
const FORWARDS: i32 = 1;

fn rand_range(r: i32) -> i32 {
    if r > 0 {
        (random() % r as u32) as i32
    } else if r < 0 {
        -((random() % (-r) as u32) as i32)
    } else {
        0
    }
}

fn sign3(a: f64) -> f64 {
    if a > 0.0 {
        1.0
    } else if a < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn modulo(a: i32, b: i32) -> i32 {
    a.rem_euclid(b)
}

struct SpeedMine {
    gc: Gc,
    background: Pixel,
    tunnelend: Pixel,
    ground_colors: Vec<Pixel>,
    wall_colors: Vec<Pixel>,
    bonus_colors: Vec<Pixel>,
    ncolors: usize,

    be_wormy: bool,
    width: i32,
    height: i32,
    delay: u32,

    smoothness: i32,
    wire_flag: bool,
    terrain_flag: bool,
    widening_flag: bool,
    bumps_flag: bool,
    bonuses_flag: bool,
    crosshair_flag: bool,
    psychedelic_flag: bool,

    maxspeed: f64,
    thrust: f64,
    gravity: f64,
    vertigo: f64,
    curviness: f64,
    twistiness: f64,

    pos: f64,
    speed: f64,
    accel: f64,
    step: f64,
    direction: i32,

    pindex: i32,
    nearest: i32,
    flipped_at: i32,
    xoffset: i32,
    yoffset: i32,

    bonus_bright: i32,
    wire_bonus: i32,
    speed_bonus: f64,
    spin_bonus: i32,
    backwards_bonus: i32,

    sintab: [f64; ROTS],
    costab: [f64; ROTS],
    orientation: i32,

    terrain: Vec<[i32; TERRAIN_BREADTH]>,
    xcurvature: [f64; TERRAIN_LENGTH],
    ycurvature: [f64; TERRAIN_LENGTH],
    zcurvature: [f64; TERRAIN_LENGTH],
    wideness: [i32; TERRAIN_LENGTH],
    bonuses: [i32; TERRAIN_LENGTH],
    xvals: Vec<[i32; TERRAIN_BREADTH]>,
    yvals: Vec<[i32; TERRAIN_BREADTH]>,
    worldx: Vec<[f64; TERRAIN_BREADTH]>,
    worldy: Vec<[f64; TERRAIN_BREADTH]>,
    minx: [i32; TERRAIN_LENGTH],
    maxx: [i32; TERRAIN_LENGTH],
    miny: [i32; TERRAIN_LENGTH],
    maxy: [i32; TERRAIN_LENGTH],

    fps_start: f64,
    rotation_offset: i32,
    jamming: i32,
}

impl SpeedMine {
    fn new(d: &mut Dpy) -> Self {
        let width = d.width();
        let height = d.height();
        let be_wormy = d.res.bool("worm");

        let mut st = Self {
            gc: Gc::new(d.res.pixel("foreground"), d.res.pixel("background")),
            background: d.res.pixel("background"),
            tunnelend: parse_color(d.res.string("tunnelend")).unwrap_or(0xFF00_0000),
            ground_colors: Vec::new(),
            wall_colors: Vec::new(),
            bonus_colors: Vec::new(),
            ncolors: MAX_COLORS,

            be_wormy,
            width,
            height,
            delay: d.res.int("delay").max(0) as u32,

            smoothness: d.res.int("smoothness").max(1),
            wire_flag: d.res.bool("wire"),
            terrain_flag: d.res.bool("terrain"),
            widening_flag: d.res.bool("widening"),
            bumps_flag: d.res.bool("bumps"),
            bonuses_flag: d.res.bool("bonuses"),
            crosshair_flag: d.res.bool("crosshair"),
            psychedelic_flag: d.res.bool("psychedelic"),

            maxspeed: (d.res.float("maxspeed") * 0.01).abs(),
            thrust: d.res.float("thrust") * 0.2,
            gravity: d.res.float("gravity") * 0.002 / 9.8,
            vertigo: d.res.float("vertigo") * 0.2,
            curviness: d.res.float("curviness") * 0.25,
            twistiness: d.res.float("twistiness") * 0.125,

            pos: 0.0,
            speed: 1.1,
            accel: 0.00000001,
            step: 0.0,
            direction: FORWARDS,

            pindex: 0,
            nearest: 0,
            flipped_at: 0,
            xoffset: 0,
            yoffset: 0,

            bonus_bright: 0,
            wire_bonus: 0,
            speed_bonus: 0.0,
            spin_bonus: 0,
            backwards_bonus: 0,

            sintab: [0.0; ROTS],
            costab: [0.0; ROTS],
            orientation: (17 * ROTS as i32) / 22,

            terrain: vec![[0; TERRAIN_BREADTH]; TERRAIN_LENGTH],
            xcurvature: [0.0; TERRAIN_LENGTH],
            ycurvature: [0.0; TERRAIN_LENGTH],
            zcurvature: [0.0; TERRAIN_LENGTH],
            wideness: [0; TERRAIN_LENGTH],
            bonuses: [0; TERRAIN_LENGTH],
            xvals: vec![[0; TERRAIN_BREADTH]; TERRAIN_LENGTH],
            yvals: vec![[0; TERRAIN_BREADTH]; TERRAIN_LENGTH],
            worldx: vec![[0.0; TERRAIN_BREADTH]; TERRAIN_LENGTH],
            worldy: vec![[0.0; TERRAIN_BREADTH]; TERRAIN_LENGTH],
            minx: [0; TERRAIN_LENGTH],
            maxx: [0; TERRAIN_LENGTH],
            miny: [0; TERRAIN_LENGTH],
            maxy: [0; TERRAIN_LENGTH],

            fps_start: 0.0,
            rotation_offset: 0,
            jamming: 0,
        };

        if st.be_wormy {
            st.maxspeed *= 1.43;
            st.thrust *= 10.0;
            st.gravity *= 3.0;
            st.vertigo *= 0.5;
            st.smoothness *= 2;
            st.curviness *= 2.0;
            st.twistiness *= 2.0;
            st.psychedelic_flag = true;
            st.crosshair_flag = false;
        }

        if st.psychedelic_flag {
            st.init_psychedelic_colors();
        } else {
            st.init_colors(d);
        }

        for i in 0..ROTS {
            let th = std::f64::consts::PI * 2.0 * i as f64 / ROTS as f64;
            st.costab[i] = th.cos();
            st.sintab[i] = th.sin();
        }

        let wide = st.random_wideness();
        for i in 0..TERRAIN_LENGTH {
            st.wideness[i] = wide;
            st.bonuses[i] = 0;
        }

        st.init_terrain();
        st.init_curves();
        st.wrap_tunnel(0, TERRAIN_LENGTH as i32 - 1);
        st.step = st.effective_speed();
        st
    }

    fn wireframe(&self) -> bool {
        self.wire_flag || self.wire_bonus > 8 || self.wire_bonus % 2 == 1
    }

    fn effective_speed(&self) -> f64 {
        self.direction as f64 * (self.speed + self.speed_bonus)
    }

    fn random_elevation(&self) -> i32 {
        if self.terrain_flag {
            (random() % 200) as i32
        } else {
            0
        }
    }

    fn random_curvature(&self) -> f64 {
        if self.curviness > 0.0 {
            ((random() % 40) as f64 - 20.0) * self.curviness
        } else {
            0.0
        }
    }

    fn random_twist(&self) -> f64 {
        if self.twistiness > 0.0 {
            ((random() % 40) as f64 - 20.0) * self.twistiness
        } else {
            0.0
        }
    }

    fn random_wideness(&self) -> i32 {
        if self.widening_flag {
            (random() % 1200) as i32
        } else {
            0
        }
    }

    // ---- colours ----------------------------------------------------------

    /// A ramp of up to thirty-two shades between two randomly chosen hues,
    /// which are opposite each other when the colours are meant to clash.
    fn color_ramp(&self, s1: f64, s2: f64, v1: f64, v2: f64) -> Vec<Pixel> {
        let (h1, h2) = if self.psychedelic_flag {
            let h1 = rand_range(360);
            (h1, (h1 + 180) % 360)
        } else {
            let h = rand_range(360);
            (h, h)
        };
        make_color_ramp(h1, s1, v1, h2, s2, v2, MAX_COLORS, false)
            .iter()
            .map(|c| c.pixel)
            .collect()
    }

    fn init_psychedelic_colors(&mut self) {
        self.ground_colors = self.color_ramp(0.0, 0.8, 0.0, 0.9);
        self.wall_colors = self.color_ramp(0.0, 0.6, 0.0, 0.9);
        self.bonus_colors = self.color_ramp(0.6, 0.9, 0.4, 1.0);
    }

    fn init_colors(&mut self, d: &mut Dpy) {
        let dark = parse_color(d.res.string("darkground")).unwrap_or(0xFF10_1010);
        let light = parse_color(d.res.string("lightground")).unwrap_or(0xFFA0_A0A0);
        let to16 = |p: Pixel| {
            let (r, g, b) = crate::runtime::color::unrgb(p);
            (
                ((r as u16) << 8) | r as u16,
                ((g as u16) << 8) | g as u16,
                ((b as u16) << 8) | b as u16,
            )
        };
        let (dr, dg, db) = to16(dark);
        let (lr, lg, lb) = to16(light);
        let (h1, s1, v1) = rgb_to_hsv(dr, dg, db);
        let (h2, s2, v2) = rgb_to_hsv(lr, lg, lb);
        self.ground_colors = make_color_ramp(h1, s1, v1, h2, s2, v2, MAX_COLORS, false)
            .iter()
            .map(|c| c.pixel)
            .collect();
        self.wall_colors = self.color_ramp(0.0, 0.6, 0.0, 0.9);
        self.bonus_colors = self.color_ramp(0.6, 0.9, 0.4, 1.0);
    }

    /// The colour-change bonus. In psychedelic mode the ground changes too.
    fn change_colors(&mut self) {
        let (s1, s2) = if self.psychedelic_flag {
            self.ground_colors = self.color_ramp(0.0, 0.8, 0.0, 0.9);
            (0.4, 0.9)
        } else {
            (0.0, 0.6)
        };
        self.wall_colors = self.color_ramp(s1, s2, 0.0, 0.9);
        self.bonus_colors = self.color_ramp(0.6, 0.9, 0.4, 1.0);
    }

    // ---- the terrain ------------------------------------------------------

    /// Wrap the height field around the semi-circular tunnel.
    fn wrap_tunnel(&mut self, start: i32, end: i32) {
        for i in start..=end {
            let i = i as usize;
            for j in 0..TERRAIN_BREADTH {
                let x = j as f64 * (1.0 / TERRAIN_BREADTH as f64);
                let v = self.terrain[i][j];
                let mut y = (if v == STEEL_ELEVATION { 200 } else { v }) as f64
                    - self.wideness[i] as f64
                    - 1200.0;
                // The floor of the tunnel is pushed further out.
                if j > TERRAIN_BREADTH / 8 && j < 3 * TERRAIN_BREADTH / 8 {
                    y -= 300.0;
                }
                self.worldx[i][j] = x / 2.0 * self.costab[j * TB_MUL]
                    - (y - self.height as f64 / 4.0) * x * self.sintab[j * TB_MUL];
                self.worldy[i][j] = x / 4.0 * self.sintab[j * TB_MUL] + y * self.costab[j * TB_MUL];
            }
        }
    }

    fn generate_smooth(&mut self, start: i32, end: i32) {
        for i in start..=end {
            for j in 0..TERRAIN_BREADTH {
                self.terrain[i as usize][j] = STEEL_ELEVATION;
            }
        }
    }

    fn generate_straight(&mut self, start: i32, end: i32) {
        for i in start..=end {
            let ii = modulo(i, TERRAIN_LENGTH as i32) as usize;
            self.xcurvature[ii] = 0.0;
            self.ycurvature[ii] = 0.0;
            self.zcurvature[ii] = 0.0;
            self.wideness[ii] = 0;
        }
    }

    /// A height near the average of its four neighbours, perturbed by an
    /// amount that shrinks with the rectangle.
    fn terrain_value(&self, v1: i32, v2: i32, v3: i32, v4: i32, w: i32) -> i32 {
        if !self.terrain_flag {
            return 0;
        }
        let sum = v1 + v2 + v3 + v4;
        let mut rval = w * sum / self.smoothness;
        if rval == 0 {
            rval = 2;
        }
        let ret = sum / 4 - (rval / 2) + rand_range(rval);
        if !(-400..=400).contains(&ret) {
            sum / 4
        } else {
            ret
        }
    }

    /// Successive subdivision of the height field, down to rectangles of
    /// minimum dimension `final_`.
    fn generate_terrain(&mut self, start: i32, end: i32, final_: i32) {
        let tl = TERRAIN_LENGTH as i32;
        let tb = TERRAIN_BREADTH as i32;
        let diff = end - start + 1;

        self.terrain[end as usize][0] = self.random_elevation();
        self.terrain[end as usize][TERRAIN_BREADTH / 2] = self.random_elevation();

        let mut w = diff / 2;
        let mut l = tb / 4;
        while w >= final_ || l >= final_ {
            if w < 1 {
                w = 1;
            }
            if l < 1 {
                l = 1;
            }

            let mut i = start + w - 1;
            while i < end {
                let ip = modulo(i - w, tl) as usize;
                let in_ = modulo(i + w, tl) as usize;
                let mut j = l - 1;
                while j < tb {
                    let jp = modulo(j - 1, tb) as usize;
                    let jn = modulo(j + 1, tb) as usize;
                    self.terrain[i as usize][j as usize] = self.terrain_value(
                        self.terrain[ip][jp],
                        self.terrain[in_][jp],
                        self.terrain[ip][jn],
                        self.terrain[in_][jn],
                        w,
                    );
                    j += l * 2;
                }
                i += w * 2;
            }

            for start_off in [w * 2, w] {
                let jstart = if start_off == w * 2 { l - 1 } else { 2 * l - 1 };
                let mut i = start + start_off - 1;
                while i < end {
                    let ip = modulo(i - w, tl) as usize;
                    let in_ = modulo(i + w, tl) as usize;
                    let mut j = jstart;
                    while j < tb {
                        let jp = modulo(j - 1, tb) as usize;
                        let jn = modulo(j + 1, tb) as usize;
                        self.terrain[i as usize][j as usize] = self.terrain_value(
                            self.terrain[ip][j as usize],
                            self.terrain[in_][j as usize],
                            self.terrain[i as usize][jp],
                            self.terrain[i as usize][jn],
                            w,
                        );
                        j += l * 2;
                    }
                    i += w * 2;
                }
            }

            w /= 2;
            l /= 2;
            if w == 0 && l == 0 {
                break;
            }
        }
    }

    fn curvature_value(v1: f64, v2: f64, w: i32) -> f64 {
        let sum = v1 + v2;
        let avg = sum / 2.0;
        let diff = (v1 - avg).min(v2 - avg);
        let rval = diff as i32 * w;
        if rval == 0 {
            return avg;
        }
        avg - (rval as f64) / 500.0 + (rand_range(rval) as f64) / 1000.0
    }

    fn generate_curves(&mut self, start: i32, end: i32) {
        let tl = TERRAIN_LENGTH as i32;
        let diff = modulo(end - start + 1, tl);
        let e = end as usize;

        self.xcurvature[e] = if random().is_multiple_of(100) {
            30.0 * self.random_curvature()
        } else if random().is_multiple_of(10) {
            20.0 * self.random_curvature()
        } else {
            10.0 * self.random_curvature()
        };
        self.ycurvature[e] = if random().is_multiple_of(50) {
            20.0 * self.random_curvature()
        } else if random().is_multiple_of(25) {
            30.0 * self.random_curvature()
        } else {
            10.0 * self.random_curvature()
        };
        self.zcurvature[e] = if random().is_multiple_of(3) {
            self.random_twist()
        } else {
            Self::curvature_value(self.zcurvature[e], self.random_twist(), 1)
        };
        self.wideness[e] = if self.be_wormy {
            self.random_wideness()
        } else {
            Self::curvature_value(self.wideness[e] as f64, self.random_wideness() as f64, 1) as i32
        };

        let mut w = diff / 2;
        while w >= 1 {
            let mut i = start + w - 1;
            while i < end {
                let ii = modulo(i, tl) as usize;
                let ip = modulo(i - w, tl) as usize;
                let in_ = modulo(i + w, tl) as usize;
                self.xcurvature[ii] =
                    Self::curvature_value(self.xcurvature[ip], self.xcurvature[in_], w);
                self.ycurvature[ii] =
                    Self::curvature_value(self.ycurvature[ip], self.ycurvature[in_], w);
                self.zcurvature[ii] =
                    Self::curvature_value(self.zcurvature[ip], self.zcurvature[in_], w);
                self.wideness[ii] =
                    Self::curvature_value(self.wideness[ip] as f64, self.wideness[in_] as f64, w)
                        as i32;
                i += w * 2;
            }
            w /= 2;
        }
    }

    fn init_terrain(&mut self) {
        let tl = TERRAIN_LENGTH as i32;
        for i in 0..TERRAIN_LENGTH {
            for j in 0..TERRAIN_BREADTH {
                self.terrain[i][j] = 0;
            }
        }
        self.terrain[TERRAIN_LENGTH - 1][0] = -((random() % 300) as i32);
        self.terrain[TERRAIN_LENGTH - 1][TERRAIN_BREADTH / 2] = -((random() % 300) as i32);

        self.generate_smooth(0, tl - 1);
        self.generate_terrain(0, tl / 4 - 1, 4);
        self.generate_terrain(tl / 4, tl / 2 - 1, 2);
        self.generate_terrain(tl / 2, 3 * tl / 4 - 1, 1);
        self.generate_smooth(3 * tl / 4, tl - 1);
    }

    fn init_curves(&mut self) {
        let tl = TERRAIN_LENGTH as i32;
        for i in 0..TERRAIN_LENGTH - 1 {
            self.xcurvature[i] = 0.0;
            self.ycurvature[i] = 0.0;
            self.zcurvature[i] = 0.0;
        }
        self.xcurvature[TERRAIN_LENGTH - 1] = self.random_curvature();
        self.ycurvature[TERRAIN_LENGTH - 1] = self.random_curvature();
        self.zcurvature[TERRAIN_LENGTH - 1] = self.random_twist();

        self.generate_straight(0, tl / 4 - 1);
        self.generate_curves(tl / 4, tl / 2 - 1);
        self.generate_curves(tl / 2, 3 * tl / 4 - 1);
        self.generate_straight(3 * tl / 4, tl - 1);
    }

    /// Regenerate the quarter of the shaft that has just been left behind.
    fn regenerate_terrain(&mut self) {
        let tl = TERRAIN_LENGTH as i32;
        let passed = self.nearest % (tl / 4);
        if self.speed == 0.0
            || (self.speed > 0.0 && passed > self.step as i32)
            || (self.speed < 0.0 && (tl / 4) - passed > self.step.abs() as i32)
        {
            return;
        }

        let end = modulo(self.nearest - passed - 1, tl);
        let start = modulo(end - tl / 4 + 1, tl);
        if start >= end {
            return;
        }

        self.set_bonuses(start, end);

        match random() % 64 {
            0 | 1 => {
                self.generate_terrain(start, end, 1);
                let to = start + tl / 8 + (random() % (TERRAIN_LENGTH as u32 / 8)) as i32;
                self.generate_smooth(start, to.min(tl - 1));
            }
            2 => {
                self.generate_smooth(start, end);
                self.generate_terrain(start, end, 4);
            }
            3 => {
                self.generate_smooth(start, end);
                self.generate_terrain(start, end, 2);
            }
            _ => self.generate_terrain(start, end, 1),
        }

        if random().is_multiple_of(16) {
            self.generate_straight(start, end);
        } else {
            self.generate_curves(start, end);
        }
        self.wrap_tunnel(start, end);
    }

    // ---- bonuses ----------------------------------------------------------

    fn set_bonuses(&mut self, start: i32, end: i32) {
        if !self.bonuses_flag {
            return;
        }
        let tl = TERRAIN_LENGTH as i32;
        let diff = end - start;
        for i in start..=end {
            self.bonuses[modulo(i, tl) as usize] = 0;
        }
        if random().is_multiple_of(4) {
            let i = start + rand_range(diff - 3);
            // The marker, then the real thing three units later.
            self.bonuses[modulo(i, tl) as usize] = 2;
            self.bonuses[modulo(i + 3, tl) as usize] = 1;
        }
    }

    /// Swap the terrain end for end, so the shaft is flown backwards.
    fn flip_direction(&mut self) {
        let tb = TERRAIN_BREADTH as i32;
        self.direction = -self.direction;
        self.bonus_bright = 20;
        for i in 0..TERRAIN_LENGTH as i32 {
            // Upstream wraps these against the breadth rather than the length,
            // so only the first thirty-two rows are ever swapped.
            let in_ = modulo(self.nearest + i, tb) as usize;
            let ip = modulo(self.nearest - i, tb) as usize;
            self.terrain.swap(ip, in_);
        }
    }

    fn do_bonus(&mut self) {
        let tl = TERRAIN_LENGTH as i32;
        self.bonus_bright = 20;

        if self.jamming > 0 {
            self.jamming -= 1;
            self.nearest = modulo(self.nearest - 2, tl);
            return;
        }
        if self.psychedelic_flag {
            self.change_colors();
        }
        match random() % 7 {
            0 => self.wire_bonus = if self.wire_bonus != 0 { 0 } else { 300 },
            1 => self.speed_bonus = 40.0,
            2 => self.spin_bonus += ROTS as i32,
            3 => self.spin_bonus -= ROTS as i32,
            4 => {
                self.flipped_at = self.nearest;
                self.flip_direction();
                self.backwards_bonus = if self.backwards_bonus != 0 { 0 } else { 10 };
            }
            5 => self.change_colors(),
            _ => {
                // Jam against the bonus a few times; deja vu!
                self.nearest = modulo(self.nearest - 2, tl);
                self.jamming = 3;
            }
        }
    }

    fn check_bonuses(&mut self) {
        if !self.bonuses_flag {
            return;
        }
        let tl = TERRAIN_LENGTH as i32;
        let (mut start, mut end) = if self.step >= 0.0 {
            (self.nearest, self.nearest + self.step.floor() as i32)
        } else {
            (self.nearest + self.step.floor() as i32, self.nearest)
        };
        if self.be_wormy {
            start += tl / 4;
            end += tl / 4;
        }
        for i in start..end {
            if self.bonuses[modulo(i, tl) as usize] == 1 {
                self.do_bonus();
            }
        }
    }

    fn decrement_bonuses(&mut self) {
        if !self.bonuses_flag {
            return;
        }
        let tl = TERRAIN_LENGTH as i32;
        if self.bonus_bright > 0 {
            self.bonus_bright -= 4;
        }
        if self.wire_bonus > 0 {
            self.wire_bonus -= 1;
        }
        if self.speed_bonus > 0.0 {
            self.speed_bonus -= 2.0;
        }
        if self.spin_bonus > 10 {
            self.spin_bonus -= (self.step * 13.7) as i32;
        } else if self.spin_bonus < -10 {
            self.spin_bonus += (self.step * 11.3) as i32;
        }
        if self.backwards_bonus > 1 {
            self.backwards_bonus -= 1;
        } else if self.backwards_bonus == 1 {
            self.nearest +=
                2 * (self.flipped_at.max(self.nearest) - self.flipped_at.min(self.nearest));
            self.nearest = modulo(self.nearest, tl);
            self.flip_direction();
            self.backwards_bonus = 0;
        }
    }

    // ---- the view ---------------------------------------------------------

    /// Map the world coordinates onto the screen.
    fn perspective(&mut self) {
        let tl = TERRAIN_LENGTH as i32;
        let tb = TERRAIN_BREADTH as i32;
        let mut zf = 8.0 * 28.0 / (self.width as f64 * TERRAIN_LENGTH as f64);
        if self.be_wormy {
            zf *= 3.0;
        }
        let mut depth = TERRAIN_PDIST - INTERP + self.pindex;
        let view_pos = modulo(self.nearest + 3 * tl / 4, tl) as usize;

        self.xoffset += (-self.xcurvature[view_pos] * self.curviness / 8.0) as i32;
        self.xoffset /= 2;
        self.yoffset += (-self.ycurvature[view_pos] * self.curviness / 4.0) as i32;
        self.yoffset /= 2;
        self.rotation_offset +=
            ((self.zcurvature[view_pos] - self.zcurvature[self.nearest as usize]) * ROTS as f64
                / 8.0) as i32;
        self.rotation_offset /= 2;
        let mut rotation_bias = self.orientation + self.spin_bonus - self.rotation_offset;

        if self.bumps_flag {
            let t = &self.terrain[view_pos];
            if self.be_wormy {
                self.yoffset -= t[TERRAIN_BREADTH / 4] * self.width / (8 * 1600);
                rotation_bias += (t[TERRAIN_BREADTH / 4 + 2] - t[TERRAIN_BREADTH / 4 - 2]) / 8;
            } else {
                self.yoffset -= t[TERRAIN_BREADTH / 4] * self.width / (2 * 1600);
                rotation_bias += (t[TERRAIN_BREADTH / 4 + 2] - t[TERRAIN_BREADTH / 4 - 2]) / 16;
            }
        }
        rotation_bias = modulo(rotation_bias, ROTS as i32);

        let (mut xc, mut yc, mut zc) = (0.0, 0.0, 0.0);
        let (mut xcc, mut ycc, mut zcc) = (0.0, 0.0, 0.0);
        for t in 0..tl {
            let i = modulo(self.nearest + t, tl) as usize;
            xc += self.xcurvature[i];
            yc += self.ycurvature[i];
            zc += self.zcurvature[i];
            xcc += xc;
            ycc += yc;
            zcc += zc;
            self.maxx[i] = 0;
            self.maxy[i] = 0;
            self.minx[i] = self.width;
            self.miny[i] = self.height;
        }

        for t in 0..tl as usize {
            let i = modulo(self.nearest - 1 - t as i32, tl) as usize;
            let zfactor = depth as f64 * (12.0 - TERRAIN_LENGTH as f64 / 8.0) * zf;
            for j in 0..TERRAIN_BREADTH {
                let jj = modulo(self.direction * j as i32, tb) as usize;
                // Avoiding a division by zero.
                let (xx, yy) = if zfactor != 0.0 {
                    (
                        (self.worldx[i][jj] - (self.vertigo * xcc)) / zfactor,
                        (self.worldy[i][j] - (self.vertigo * ycc)) / zfactor,
                    )
                } else {
                    (0.0, 0.0)
                };
                let r = modulo(rotation_bias + (self.vertigo * zcc) as i32, ROTS as i32) as usize;

                self.xvals[t][j] = self.xoffset
                    + (self.width >> 1)
                    + (xx * self.costab[r] - yy * self.sintab[r]) as i32;
                self.maxx[t] = self.maxx[t].max(self.xvals[t][j]);
                self.minx[t] = self.minx[t].min(self.xvals[t][j]);

                self.yvals[t][j] = self.yoffset
                    + self.height / 2
                    + (xx * self.sintab[r] + yy * self.costab[r]) as i32;
                self.maxy[t] = self.maxy[t].max(self.yvals[t][j]);
                self.miny[t] = self.miny[t].min(self.yvals[t][j]);
            }
            xcc -= xc;
            ycc -= yc;
            zcc -= zc;
            xc -= self.xcurvature[i];
            yc -= self.ycurvature[i];
            zc -= self.zcurvature[i];
            depth -= INTERP;
        }
    }

    /// The shade for one face: depth, then tilt, then which way it faces.
    fn shade(&self, t: i32, i: usize, in_: usize, j: usize, dy: i32) -> usize {
        let mut index = self.bonus_bright
            + self.ncolors as i32 / 3
            + t * (t * INTERP + self.pindex) * self.ncolors as i32
                / (3 * TERRAIN_LENGTH as i32 * TERRAIN_PDIST);
        if !self.wireframe() {
            index += dy / 8;
            index += ((self.worldx[i][j] - self.worldx[in_][j]) / 40.0) as i32;
            index += (self.terrain[in_][j] - self.terrain[i][j]) / 100;
        }
        if self.be_wormy && self.psychedelic_flag {
            index += self.ncolors as i32 / 4;
        }
        index.clamp(0, self.ncolors as i32 - 1) as usize
    }

    /// Which ramp a face comes out of: the bonus, the floor, or the wall.
    fn face_color(&self, i: usize, in_: usize, j: usize, index: usize, pentagon: bool) -> Pixel {
        let tb = TERRAIN_BREADTH;
        if self.bonuses[i] != 0 {
            return self.bonus_colors[index];
        }
        let ground = if pentagon {
            j < tb / 8
                || (j > tb / 8 && j < 3 * tb / 8 - 1)
                || self.terrain[i][j] == STEEL_ELEVATION
                || self.wideness[in_] - self.wideness[i] > 200
        } else {
            (self.direction > 0 && j < tb / 8)
                || (j > tb / 8 && j < 3 * tb / 8 - 1)
                || (self.direction < 0 && j > 3 * tb / 8 - 1 && j < tb / 2)
                || self.terrain[i][j] == STEEL_ELEVATION
                || self.wideness[in_] - self.wideness[i] > 200
        };
        if ground {
            self.ground_colors[index]
        } else {
            self.wall_colors[index]
        }
    }

    fn render_quads(&mut self, d: &mut Dpy, t: usize, dt: usize, i: usize) {
        let in_ = modulo(i as i32 + 1, TERRAIN_LENGTH as i32) as usize;
        let mut j = 0;
        while j < TERRAIN_BREADTH {
            let t2 = (t + dt) % TERRAIN_LENGTH;
            let j2 = (j + dt) % TERRAIN_BREADTH;
            let pts = [
                XPoint {
                    x: self.xvals[t][j],
                    y: self.yvals[t][j],
                },
                XPoint {
                    x: self.xvals[t2][j],
                    y: self.yvals[t2][j],
                },
                XPoint {
                    x: self.xvals[t2][j2],
                    y: self.yvals[t2][j2],
                },
                XPoint {
                    x: self.xvals[t][j2],
                    y: self.yvals[t][j2],
                },
            ];
            let index = self.shade(t as i32, i, in_, j, pts[0].y - pts[3].y);
            if self.wireframe() {
                let c = if self.bonuses[i] != 0 {
                    self.bonus_colors[index]
                } else {
                    self.ground_colors[index]
                };
                self.gc.set_foreground(c);
                d.win().draw_lines(&self.gc, &pts);
            } else {
                let c = self.face_color(i, in_, j, index, false);
                self.gc.set_foreground(c);
                d.win().fill_polygon(&self.gc, &pts);
            }
            j += dt;
        }
    }

    /// The seam between two depth bands, where the grid step changes.
    fn render_pentagons(&mut self, d: &mut Dpy, t: usize, dt: usize, i: usize) {
        let in_ = modulo(i as i32 + 1, TERRAIN_LENGTH as i32) as usize;
        let mut j = 0;
        while j < TERRAIN_BREADTH {
            let t2 = (t + dt * 2) % TERRAIN_LENGTH;
            let j2 = (j + dt) % TERRAIN_BREADTH;
            let j3 = (j + dt + dt) % TERRAIN_BREADTH;
            let pts = [
                XPoint {
                    x: self.xvals[t][j],
                    y: self.yvals[t][j],
                },
                XPoint {
                    x: self.xvals[t2][j],
                    y: self.yvals[t2][j],
                },
                XPoint {
                    x: self.xvals[t2][j2],
                    y: self.yvals[t2][j2],
                },
                XPoint {
                    x: self.xvals[t2][j3],
                    y: self.yvals[t2][j3],
                },
                XPoint {
                    x: self.xvals[t][j3],
                    y: self.yvals[t][j3],
                },
            ];
            let index = self.shade(t as i32, i, in_, j, pts[0].y - pts[3].y);
            if self.wireframe() {
                let c = if self.bonuses[i] != 0 {
                    self.bonus_colors[index]
                } else {
                    self.ground_colors[index]
                };
                self.gc.set_foreground(c);
                d.win().draw_lines(&self.gc, &pts);
            } else {
                let c = self.face_color(i, in_, j, index, true);
                self.gc.set_foreground(c);
                d.win().fill_polygon(&self.gc, &pts);
            }
            j += dt * 2;
        }
    }

    /// The tunnel cross-section at one depth.
    fn block_points(&self, t: usize) -> Vec<XPoint> {
        (0..TERRAIN_BREADTH / 2)
            .map(|i| XPoint {
                x: self.xvals[t][i * 2],
                y: self.yvals[t][i * 2],
            })
            .collect()
    }

    fn render_block(&mut self, d: &mut Dpy, color: Pixel, t: usize) {
        let pts = self.block_points(t);
        self.gc.set_foreground(color);
        d.win().fill_polygon(&self.gc, &pts);
    }

    /// The shower of sparks that marks a bonus.
    ///
    /// Upstream paints the whole cross-section through a one-bit mask of
    /// random crosses; here each cross is tested against the cross-section and
    /// drawn whole or not at all, which this runtime can do and a bitmap clip
    /// cannot.
    fn render_bonus_block(&mut self, d: &mut Dpy, t: usize, i: usize) {
        if self.bonuses[i] == 0 || self.wireframe() {
            return;
        }
        let tl = TERRAIN_LENGTH;
        let w = self.maxx[t] - self.minx[t];
        let h = self.maxy[t] - self.miny[t];
        if w < 6 || h < 6 {
            return;
        }
        let lim = self.width as usize * tl / (300 * (tl - t).max(1));
        let bt = t * self.bonus_colors.len() / (2 * tl);
        let color = self.bonus_colors[bt.min(self.bonus_colors.len() - 1)];
        let pts = self.block_points(t);
        self.gc.set_foreground(color);

        let l1 = if t > 3 * tl / 4 { 2 } else { 1 };
        let l2 = if t > 7 * tl / 8 { 2 } else { 1 };
        for l in [l1, l2] {
            for _ in 0..lim {
                let a = rand_range(w);
                let b = rand_range(h);
                let (x, y) = (self.minx[t] + a, self.miny[t] + b);
                if !inside(&pts, x, y) {
                    continue;
                }
                d.win().draw_line(&self.gc, x - l, y, x + l, y);
                d.win().draw_line(&self.gc, x, y - l, x, y + l);
            }
        }
    }

    /// The furthest depth at which the tunnel still covers the whole view.
    fn begin_at(&self) -> i32 {
        let (mut max_minx, mut min_maxx) = (0, self.width);
        let (mut max_miny, mut min_maxy) = (0, self.height);
        let mut t = TERRAIN_LENGTH as i32 - 1;
        while t > 0 {
            max_minx = max_minx.max(self.minx[t as usize]);
            min_maxx = min_maxx.min(self.maxx[t as usize]);
            max_miny = max_miny.max(self.miny[t as usize]);
            min_maxy = min_maxy.min(self.maxy[t as usize]);
            if max_miny >= min_maxy || max_minx >= min_maxx {
                break;
            }
            t -= 1;
        }
        t
    }

    fn render(&mut self, d: &mut Dpy) {
        let tl = TERRAIN_LENGTH as i32;
        let mut i = self.nearest as usize;
        let mut t;

        if self.be_wormy || self.wireframe() {
            let (w, h) = (self.width, self.height);
            self.gc.set_foreground(self.background);
            d.win().fill_rectangle(&self.gc, 0, 0, w, h);
            let dt = 4;
            t = 0;
            while t < TERRAIN_LENGTH / 4 {
                self.render_bonus_block(d, t, i);
                i = modulo(i as i32 - dt as i32, tl) as usize;
                self.render_quads(d, t, dt, i);
                t += dt;
            }
        } else {
            t = self.begin_at().max(tl / 4) as usize;
            i = modulo(self.nearest - t as i32, tl) as usize;
            let end = self.tunnelend;
            self.render_block(d, end, t);
        }

        let mut dt = 2;
        if t == TERRAIN_LENGTH / 4 {
            self.render_pentagons(d, t, dt, i);
        }
        while t < 3 * TERRAIN_LENGTH / 4 {
            self.render_bonus_block(d, t, i);
            i = modulo(i as i32 - dt as i32, tl) as usize;
            self.render_quads(d, t, dt, i);
            t += dt;
        }

        dt = 1;
        let last = TERRAIN_LENGTH - (1 + usize::from(self.pindex < INTERP / 2));
        if self.be_wormy {
            while t < last {
                self.render_bonus_block(d, t, i);
                i = modulo(i as i32 - dt as i32, tl) as usize;
                t += dt;
            }
        } else {
            if t == 3 * TERRAIN_LENGTH / 4 {
                self.render_pentagons(d, t, dt, i);
            }
            while t < last {
                self.render_bonus_block(d, t, i);
                i = modulo(i as i32 - dt as i32, tl) as usize;
                self.render_quads(d, t, dt, i);
                t += dt;
            }
        }

        if self.crosshair_flag {
            let c = if self.wireframe() {
                self.bonus_colors[self.bonus_colors.len() / 2]
            } else {
                self.background
            };
            self.gc.set_foreground(c);
            let (w2, h2) = (
                self.width / 2 + self.xoffset,
                self.height / 2 + self.yoffset * 2,
            );
            d.win().fill_rectangle(&self.gc, w2 - 8, h2 - 1, 16, 3);
            d.win().fill_rectangle(&self.gc, w2 - 1, h2 - 8, 3, 16);
        }
    }

    /// Advance to the position for the next frame.
    fn move_along(&mut self) {
        let tl = TERRAIN_LENGTH as i32;
        self.pos += self.step;
        let dpos = sign3(self.pos) * self.pos.abs().floor();

        self.pindex += sign3(self.effective_speed()) as i32 + INTERP;
        while self.pindex >= INTERP {
            self.nearest -= 1;
            self.pindex -= INTERP;
        }
        while self.pindex < 0 {
            self.nearest += 1;
            self.pindex += INTERP;
        }
        self.nearest = modulo(self.nearest + dpos as i32, tl);
        self.pos -= dpos;

        self.accel = self.thrust + self.ycurvature[self.nearest as usize] * self.gravity;
        self.speed += self.accel;
        self.speed = self.speed.clamp(-self.maxspeed, self.maxspeed);
    }
}

/// Whether a point is inside a polygon, by the even-odd rule.
fn inside(pts: &[XPoint], x: i32, y: i32) -> bool {
    let mut c = false;
    let n = pts.len();
    for k in 0..n {
        let (a, b) = (pts[k], pts[(k + 1) % n]);
        if (a.y > y) != (b.y > y) {
            let t = (y - a.y) as f64 / (b.y - a.y) as f64;
            if (x as f64) < a.x as f64 + t * (b.x - a.x) as f64 {
                c = !c;
            }
        }
    }
    c
}

impl Screenhack for SpeedMine {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        self.regenerate_terrain();
        self.perspective();
        self.render(d);

        // How far the shaft advances per frame is scaled by the measured frame
        // rate, so the speed is the same however fast the frames come.
        let now = d.time;
        if now > self.fps_start + 0.5 {
            let elapsed = now - self.fps_start;
            self.fps_start = now;
            self.step = self.effective_speed() * elapsed;
        }

        self.move_along();
        self.decrement_bonuses();
        self.check_bonuses();
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    Box::new(SpeedMine::new(d))
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: white",
    "*worm: False",
    "*wire: False",
    "*darkground: #101010",
    "*lightground: #a0a0a0",
    "*tunnelend: #000000",
    "*delay: 30000",
    "*maxspeed: 700",
    "*thrust: 1.0",
    "*gravity: 9.8",
    "*vertigo: 1.0",
    "*terrain: True",
    "*smoothness: 6",
    "*curviness: 1.0",
    "*twistiness: 1.0",
    "*widening: True",
    "*bumps: True",
    "*bonuses: True",
    "*crosshair: True",
    "*psychedelic: False",
];

const MODES: &[SelectItem] = &[
    SelectItem {
        value: "false",
        label: "Tunnel",
    },
    SelectItem {
        value: "true",
        label: "Worm",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("maxspeed", "Max velocity", 1.0, 1000.0, 10.0, 0, "700"),
    Opt::slider("thrust", "Thrust", 0.0, 4.0, 0.1, 1, "1.0"),
    Opt::slider("gravity", "Gravity", 0.0, 25.0, 0.5, 1, "9.8"),
    Opt::select("worm", "Mode", MODES, "false"),
    Opt::boolean("terrain", "Rocky walls", "true"),
    Opt::boolean("bumps", "Allow wall collisions", "true"),
    Opt::boolean("bonuses", "Present bonuses", "true"),
    Opt::boolean("crosshair", "Display crosshair", "true"),
    Opt::boolean("wire", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "speedmine",
    label: "Speed Mine",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Conrad Parker",
        year: "2001",
        video: Some("https://www.youtube.com/watch?v=awOnhCxRD_c"),
        blurb: "Simulates speeding down a rocky mineshaft, or a funky dancing worm.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
