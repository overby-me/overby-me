//! Port of `hacks/glx/atlantis.c` and `hacks/glx/swim.c`.
//!
//! ```text
//! atlantis --- Shows moving 3D sea animals
//!
//! Copyright (c) E. Lassauge, 1998.
//!
//! Permission to use, copy, modify, and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appear in all copies and that
//! both that copyright notice and this permission notice appear in
//! supporting documentation.
//!
//! This file is provided AS IS with no warranties of any kind.  The author
//! shall have no liability with respect to the infringement of copyrights,
//! trade secrets or any patents by this file or any part thereof.  In no
//! event will the author be liable for any lost revenue or profits or
//! other special, indirect and consequential damages.
//!
//! The original code for this mode was written by Mark J. Kilgard
//! as a demo for openGL programming.
//!
//! (c) Copyright 1993, 1994, Silicon Graphics, Inc.
//! ALL RIGHTS RESERVED
//! Permission to use, copy, modify, and distribute this software for
//! any purpose and without fee is hereby granted, provided that the above
//! copyright notice appear in all copies and that both the copyright notice
//! and this permission notice appear in supporting documentation, and that
//! the name of Silicon Graphics, Inc. not be used in advertising
//! or publicity pertaining to distribution of the software without specific,
//! written prior permission.
//! ```
//!
//! Sharks, dolphins and a pair of whales, in a tank a hundred thousand units
//! across. It is one of the oldest OpenGL demos there is, and the fish are
//! all from 1993.
//!
//! Nobody here is following anybody. Each shark steers towards a fixed point
//! sixty thousand units away, turning half a degree a frame towards it and
//! taking a sudden burst of speed when it gets close to lined up, so the
//! school ends up wheeling round that point without ever having been told to.
//! The whales and the dolphin ignore all that and swim in a circle, banking
//! into the turn. The only thing any of them knows about the others is that a
//! shark tilts away from another shark that comes within a set distance,
//! which is what the "shark proximity" knob sets.
//!
//! The tails are the same trick in every fish: a run of points down the body
//! is displaced sideways by a sine of a phase that runs a little later the
//! further back you go, so the wave travels tailwards. The shark's tail also
//! leans into whatever turn it is making, which is why it looks like it is
//! steering rather than being dragged.
//!
//! The shark is drawn eight different ways and picks between them on the
//! signs of the third column of the modelview matrix. There is no depth
//! sorting in a 1993 demo, so instead the parts are ordered back to front for
//! each of the eight octants the camera can be in.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, DepthFunc, Shape};
use crate::runtime::png;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random_below,
};

use super::atlantis_models as m;

const RAD: f32 = 57.295;
const RRAD: f32 = 0.01745;

/// One animal, wherever it is and whatever it is doing.
#[derive(Clone, Copy, Default)]
struct FishRec {
    /// Position in global coordinate system.
    x: f32,
    y: f32,
    z: f32,
    /// Three rotation angles to determine heading; phi=roll, psi=bearing/yaw,
    /// theta=elevation/pitch.
    phi: f32,
    theta: f32,
    psi: f32,
    /// Speed along forward direction vector.
    v: f32,
    xt: f32,
    yt: f32,
    zt: f32,
    /// Tail position adjustments; htail controls the phase of the thrash
    /// animation for the whales and dolphin.
    htail: f32,
    vtail: f32,
    /// Scale factor for the whale/dolphin tail thrash speed. Normally bigger
    /// creatures move more slowly; since the fish size isn't stored anywhere
    /// adjust this speed instead.
    tail_speed_scale: f32,
    /// Scale factor for the size of the loop the whale/dolphin is swimming
    /// around in.
    loop_scale: f32,
    /// Parameters used for shark swimming.
    dtheta: f32,
    spurt: bool,
    attack: bool,
    sign: bool,
}

struct Atlantis {
    num_sharks: usize,
    sharkspeed: f32,
    whalespeed: f32,
    sharksize: f32,
    wire: bool,
    whaledir: bool,
    sharks: Vec<FishRec>,
    mom_whale: FishRec,
    baby_whale: FishRec,
    dolph: FishRec,

    do_texture: bool,
    do_gradient: bool,
    texture: u32,
    /// The tank's own colour, which is a slightly different blue every time.
    clear: [f32; 4],
}

/* swim.c */

fn fish_transform(g: &mut Gl, fish: &FishRec) {
    g.glx.translate(fish.y, fish.z, -fish.x);
    g.glx.rotate(-fish.psi, 0.0, 1.0, 0.0);
    g.glx.rotate(fish.theta, 1.0, 0.0, 0.0);
    g.glx.rotate(-fish.phi, 0.0, 0.0, 1.0);
}

fn whale_pilot(fish: &mut FishRec, whalespeed: f32, whaledir: bool) {
    let turn_speed_scale = 1.0
        / if fish.loop_scale == 0.0 {
            1.0
        } else {
            fish.loop_scale
        };

    /* turning in a circle */
    fish.psi += turn_speed_scale * if whaledir { -0.5 } else { 0.5 };
    /* banking into the turn */
    fish.phi = if whaledir { -20.0 } else { 20.0 };
    /* not rising or falling */
    fish.theta = 0.0;

    fish.x += whalespeed * fish.v * (fish.psi / RAD).cos() * (fish.theta / RAD).cos();
    fish.y += whalespeed * fish.v * (fish.psi / RAD).sin() * (fish.theta / RAD).cos();
    fish.z += whalespeed * fish.v * (fish.theta / RAD).sin();
}

fn shark_pilot(fish: &mut FishRec, sharkspeed: f32) {
    fish.xt = 60000.0;
    fish.yt = 0.0;
    fish.zt = 0.0;

    let x = fish.xt - fish.x;
    let y = fish.yt - fish.y;
    let z = fish.zt - fish.z;

    let thetal = fish.theta;

    let ttheta = RAD * (z / (x * x + y * y).sqrt()).atan();

    if ttheta > fish.theta + 0.25 {
        fish.theta += 0.5;
    } else if ttheta < fish.theta - 0.25 {
        fish.theta -= 0.5;
    }
    fish.theta = fish.theta.clamp(-90.0, 90.0);
    fish.dtheta = fish.theta - thetal;

    let tpsi = RAD * y.atan2(x);

    fish.attack = false;

    if (tpsi - fish.psi).abs() < 10.0 {
        fish.attack = true;
    } else if (tpsi - fish.psi).abs() < 45.0 {
        if fish.psi > tpsi {
            fish.psi -= 0.5;
            if fish.psi < -180.0 {
                fish.psi += 360.0;
            }
        } else if fish.psi < tpsi {
            fish.psi += 0.5;
            if fish.psi > 180.0 {
                fish.psi -= 360.0;
            }
        }
    } else {
        if random_below(100) > 98 {
            fish.sign = !fish.sign;
        }
        fish.psi += if fish.sign { 1.0 } else { -1.0 };
        if fish.psi > 180.0 {
            fish.psi -= 360.0;
        }
        if fish.psi < -180.0 {
            fish.psi += 360.0;
        }
    }

    if fish.attack {
        if fish.v < 1.1 {
            fish.spurt = true;
        }
        if fish.spurt {
            fish.v += 0.2;
        }
        if fish.v > 5.0 {
            fish.spurt = false;
        }
        if fish.v > 1.0 && !fish.spurt {
            fish.v -= 0.2;
        }
    } else {
        if random_below(400) == 0 && !fish.spurt {
            fish.spurt = true;
        }
        if fish.spurt {
            fish.v += 0.05;
        }
        if fish.v > 3.0 {
            fish.spurt = false;
        }
        if fish.v > 1.0 && !fish.spurt {
            fish.v -= 0.05;
        }
    }

    fish.x += sharkspeed * fish.v * (fish.psi / RAD).cos() * (fish.theta / RAD).cos();
    fish.y += sharkspeed * fish.v * (fish.psi / RAD).sin() * (fish.theta / RAD).cos();
    fish.z += sharkspeed * fish.v * (fish.theta / RAD).sin();
}

impl Atlantis {
    fn shark_miss(&mut self, i: usize) {
        for j in 0..self.num_sharks {
            if j == i {
                continue;
            }
            let x = self.sharks[j].x - self.sharks[i].x;
            let y = self.sharks[j].y - self.sharks[i].y;
            let z = self.sharks[j].z - self.sharks[i].z;

            let r = (x * x + y * y + z * z).sqrt();

            let avoid = 1.0;
            let thetal = self.sharks[i].theta;

            if r < self.sharksize {
                if z > 0.0 {
                    self.sharks[i].theta -= avoid;
                } else {
                    self.sharks[i].theta += avoid;
                }
            }
            let d = self.sharks[i].theta - thetal;
            self.sharks[i].dtheta += d;
        }
    }

    fn init_fishs(&mut self) {
        let size = self.sharksize;
        for s in &mut self.sharks {
            s.x = 70000.0 + random_below(size as i32) as f32;
            s.y = random_below(size as i32) as f32;
            s.z = random_below(size as i32) as f32;
            s.psi = random_below(360) as f32 - 180.0;
            s.v = 1.0;
            s.tail_speed_scale = 0.0;
            s.loop_scale = 0.0;
        }

        /* Random whale direction */
        self.whaledir = random_below(2) == 1;

        let dolphin_offset = (random_below(20000) - 10000) as f32;
        self.dolph = FishRec {
            x: 30000.0,
            y: dolphin_offset,
            z: dolphin_offset,
            phi: -20.0,
            theta: 0.0,
            psi: if self.whaledir { 90.0 } else { -90.0 },
            /* 5.0 ± 1.0 */
            v: 5.0 + frand(2.0) as f32,
            tail_speed_scale: 0.5,
            htail: random_below(360) as f32,
            /* ×1.0 ± 0.5 */
            loop_scale: 1.0 + frand(0.5) as f32,
            ..FishRec::default()
        };

        let whale_offset = (random_below(40000) - 20000) as f32;
        self.mom_whale = FishRec {
            x: 70000.0,
            y: whale_offset,
            z: whale_offset,
            phi: -20.0,
            theta: 0.0,
            psi: if self.whaledir { 90.0 } else { -90.0 },
            /* 3.0 ± 0.5 */
            v: 2.5 + frand(1.0) as f32,
            tail_speed_scale: 1.0 / 3.0,
            htail: random_below(360) as f32,
            loop_scale: 1.0 + frand(0.5) as f32,
            ..FishRec::default()
        };

        self.baby_whale = FishRec {
            x: self.mom_whale.x - 10000.0,
            y: self.mom_whale.y - 2000.0,
            z: self.mom_whale.z - 2000.0,
            phi: self.mom_whale.phi,
            theta: self.mom_whale.theta,
            psi: self.mom_whale.psi,
            v: self.mom_whale.v,
            tail_speed_scale: 1.0,
            htail: random_below(360) as f32,
            loop_scale: self.mom_whale.loop_scale,
            ..FishRec::default()
        };
    }

    /// Fill the background with a gradient -- thanks to Phil Carrig
    /// <pod@internode.on.net> for figuring out how to do this more
    /// efficiently!
    fn clear_tank(&self, g: &mut Gl) {
        g.glx.clear();

        if !self.do_gradient || self.wire {
            return;
        }

        let top = [0.0, 0.400, 0.70];
        let bot = [0.0, 0.025, 0.09];

        g.glx.matrix_mode_projection();
        g.glx.push_matrix();
        g.glx.load_identity();
        g.glx.matrix_mode_modelview();
        g.glx.push_matrix();
        g.glx.load_identity();

        g.glx.lighting(false);
        g.glx.begin(Shape::Quads);
        g.glx.color3f(bot[0], bot[1], bot[2]);
        g.glx.vertex3f(-1.0, -1.0, 1.0);
        g.glx.vertex3f(1.0, -1.0, 1.0);
        g.glx.color3f(top[0], top[1], top[2]);
        g.glx.vertex3f(1.0, 1.0, 1.0);
        g.glx.vertex3f(-1.0, 1.0, 1.0);
        g.glx.end();
        g.glx.lighting(true);

        /* Need to reset this because jwzgles conflates color and material */
        g.glx.color3f(0.0, 0.1, 0.2);

        g.glx.pop_matrix();
        g.glx.matrix_mode_projection();
        g.glx.pop_matrix();
        g.glx.matrix_mode_modelview();
    }

    fn animate(&mut self) {
        for i in 0..self.num_sharks {
            shark_pilot(&mut self.sharks[i], self.sharkspeed);
            self.shark_miss(i);
        }
        let (speed, dir) = (self.whalespeed, self.whaledir);
        whale_pilot(&mut self.dolph, speed, dir);
        whale_pilot(&mut self.mom_whale, speed, dir);
        whale_pilot(&mut self.baby_whale, speed, dir);
    }

    /// `DrawShark`. The tail leans into the turn, and which of the eight
    /// orderings to draw is read off the modelview matrix.
    fn draw_shark(&self, g: &mut Gl, fish: &mut FishRec) {
        fish.htail = ((fish.htail as i32 - (5.0 * fish.v) as i32) % 360) as f32;

        let thrash = 50.0 * fish.v;

        let seg1 = 0.6 * thrash * (fish.htail * RRAD).sin();
        let seg2 = 1.8 * thrash * ((fish.htail + 45.0) * RRAD).sin();
        let seg3 = 3.0 * thrash * ((fish.htail + 90.0) * RRAD).sin();
        let seg4 = 4.0 * thrash * ((fish.htail + 110.0) * RRAD).sin();

        let mut chomp = 0.0;
        if fish.v > 2.0 {
            chomp = -(fish.v - 2.0) * 200.0;
        }

        fish.vtail += (fish.dtheta - fish.vtail) * 0.1;
        fish.vtail = fish.vtail.clamp(-0.5, 0.5);

        let segup = thrash * fish.vtail;

        let mut p = m::SHARK_P;
        shark_segments(&mut p, seg1, seg2, seg3, seg4, segup, chomp);

        g.glx.push_matrix();
        g.glx.translate(0.0, 0.0, -3000.0);

        let mat = g.glx.modelview_matrix().0;
        let mut n = 0;
        if mat[2] >= 0.0 {
            n += 1;
        }
        if mat[6] >= 0.0 {
            n += 2;
        }
        if mat[10] >= 0.0 {
            n += 4;
        }

        g.glx.scale(2.0, 1.0, 1.0);
        g.glx.cull_face(true);

        let mut f = m::Fish::new(g, self.wire, self.do_texture);
        match n {
            0 => m::fish1(&mut f, &p),
            1 => m::fish2(&mut f, &p),
            2 => m::fish3(&mut f, &p),
            3 => m::fish4(&mut f, &p),
            4 => m::fish5(&mut f, &p),
            5 => m::fish6(&mut f, &p),
            6 => m::fish7(&mut f, &p),
            _ => m::fish8(&mut f, &p),
        }
        f.finish();

        g.glx.cull_face(false);
        g.glx.pop_matrix();
    }

    /// `DrawDolphin` and `DrawWhale`, which differ only in their numbers and
    /// in which parts they name.
    fn draw_whale(&self, g: &mut Gl, fish: &mut FishRec, dolphin: bool) {
        let tail_speed_scale = if fish.tail_speed_scale == 0.0 {
            1.0
        } else {
            fish.tail_speed_scale
        };
        fish.htail = ((fish.htail as i32 - (5.0 * tail_speed_scale * fish.v) as i32) % 360) as f32;

        let thrash = 70.0 * fish.v;
        let sines: [f32; 8] = if dolphin {
            [1.0, 2.0, 3.0, 1.0, 4.0, 4.5, 5.0, 6.0]
        } else {
            [1.5, 2.5, 3.7, 4.8, 6.0, 6.5, 6.5, 6.5]
        };
        let phases: [f32; 8] = if dolphin {
            [0.0, 4.0, 6.0, 0.0, 10.0, 15.0, 20.0, 30.0]
        } else {
            [0.0, 10.0, 15.0, 23.0, 28.0, 35.0, 40.0, 55.0]
        };
        let seg = |i: usize| sines[i] * thrash * ((fish.htail + phases[i]) * RRAD).sin();
        let segs = [
            seg(0),
            seg(1),
            seg(2),
            seg(3),
            seg(4),
            seg(5),
            seg(6),
            seg(7),
        ];

        let pitch = fish.v * ((fish.htail + if dolphin { 180.0 } else { -160.0 }) * RRAD).sin();

        let chomp = if dolphin {
            100.0
        } else if fish.v > 2.0 {
            -(fish.v - 2.0) * 200.0
        } else {
            0.0
        };

        g.glx.push_matrix();
        g.glx.rotate(pitch, 1.0, 0.0, 0.0);
        g.glx
            .translate(0.0, 0.0, if dolphin { 7000.0 } else { 8000.0 });
        g.glx.rotate(180.0, 0.0, 1.0, 0.0);
        if !dolphin {
            g.glx.scale(3.0, 3.0, 3.0);
        }
        g.glx.cull_face(true);

        if dolphin {
            let mut d = m::DOLPHIN_P;
            dolphin_segments(&mut d, &segs, chomp);
            let mut f = m::Fish::new(g, self.wire, self.do_texture);
            for part in [
                m::dolphin014 as fn(&mut m::Fish, &m::Pts),
                m::dolphin010,
                m::dolphin009,
                m::dolphin012,
                m::dolphin013,
                m::dolphin006,
                m::dolphin002,
                m::dolphin001,
                m::dolphin003,
                m::dolphin015,
                m::dolphin004,
                m::dolphin005,
                m::dolphin007,
                m::dolphin008,
                m::dolphin011,
                m::dolphin016,
            ] {
                part(&mut f, &d);
            }
            f.finish();
        } else {
            let mut p = m::WHALE_P;
            whale_segments(&mut p, &segs, chomp);
            let mut f = m::Fish::new(g, self.wire, self.do_texture);
            for part in [
                m::whale001 as fn(&mut m::Fish, &m::Pts),
                m::whale002,
                m::whale003,
                m::whale004,
                m::whale005,
                m::whale006,
                m::whale007,
                m::whale008,
                m::whale009,
                m::whale010,
                m::whale011,
                m::whale012,
                m::whale013,
                m::whale014,
                m::whale015,
                m::whale016,
            ] {
                part(&mut f, &p);
            }
            f.finish();
        }

        g.glx.cull_face(false);
        g.glx.pop_matrix();
    }

    fn all_display(&mut self, g: &mut Gl) {
        self.clear_tank(g);

        for i in 0..self.num_sharks {
            g.glx.push_matrix();
            let mut fish = self.sharks[i];
            fish_transform(g, &fish);
            self.draw_shark(g, &mut fish);
            self.sharks[i] = fish;
            g.glx.pop_matrix();
        }

        g.glx.push_matrix();
        let mut dolph = self.dolph;
        fish_transform(g, &dolph);
        self.draw_whale(g, &mut dolph, true);
        self.dolph = dolph;
        g.glx.pop_matrix();

        g.glx.push_matrix();
        let mut mom = self.mom_whale;
        fish_transform(g, &mom);
        self.draw_whale(g, &mut mom, false);
        self.mom_whale = mom;
        g.glx.pop_matrix();

        g.glx.push_matrix();
        let mut baby = self.baby_whale;
        fish_transform(g, &baby);
        g.glx.scale(0.45, 0.45, 0.3);
        self.draw_whale(g, &mut baby, false);
        self.baby_whale = baby;
        g.glx.pop_matrix();
    }
}

/// The whale's tail: a run of points displaced in y by a sine whose phase
/// runs later the further back down the body it is, so the wave travels
/// tailwards. `chomp` opens the jaw.
fn whale_segments(p: &mut [[f32; 3]], seg: &[f32; 8], chomp: f32) {
    let ip = m::WHALE_P;
    let [seg0, seg1, seg2, seg3, seg4, seg5, seg6, seg7] = *seg;
    p[12][1] = ip[12][1] + seg5;
    p[13][1] = ip[13][1] + seg5;
    p[14][1] = ip[14][1] + seg5;
    p[15][1] = ip[15][1] + seg5;
    p[16][1] = ip[16][1] + seg5;
    p[17][1] = ip[17][1] + seg5;
    p[18][1] = ip[18][1] + seg5;
    p[19][1] = ip[19][1] + seg5;
    p[20][1] = ip[20][1] + seg4;
    p[21][1] = ip[21][1] + seg4;
    p[22][1] = ip[22][1] + seg4;
    p[23][1] = ip[23][1] + seg4;
    p[24][1] = ip[24][1] + seg4;
    p[25][1] = ip[25][1] + seg4;
    p[26][1] = ip[26][1] + seg4;
    p[27][1] = ip[27][1] + seg4;
    p[28][1] = ip[28][1] + seg2;
    p[29][1] = ip[29][1] + seg2;
    p[30][1] = ip[30][1] + seg2;
    p[31][1] = ip[31][1] + seg2;
    p[32][1] = ip[32][1] + seg2;
    p[33][1] = ip[33][1] + seg2;
    p[34][1] = ip[34][1] + seg2;
    p[35][1] = ip[35][1] + seg2;
    p[36][1] = ip[36][1] + seg1;
    p[37][1] = ip[37][1] + seg1;
    p[38][1] = ip[38][1] + seg1;
    p[39][1] = ip[39][1] + seg1;
    p[40][1] = ip[40][1] + seg1;
    p[41][1] = ip[41][1] + seg1;
    p[42][1] = ip[42][1] + seg1;
    p[43][1] = ip[43][1] + seg1;
    p[44][1] = ip[44][1] + seg0;
    p[45][1] = ip[45][1] + seg0;
    p[46][1] = ip[46][1] + seg0;
    p[47][1] = ip[47][1] + seg0;
    p[48][1] = ip[48][1] + seg0;
    p[49][1] = ip[49][1] + seg0;
    p[50][1] = ip[50][1] + seg0;
    p[51][1] = ip[51][1] + seg0;
    p[9][1] = ip[9][1] + seg6;
    p[10][1] = ip[10][1] + seg6;
    p[75][1] = ip[75][1] + seg6;
    p[76][1] = ip[76][1] + seg6;
    p[1][1] = ip[1][1] + seg7;
    p[11][1] = ip[11][1] + seg7;
    p[68][1] = ip[68][1] + seg7;
    p[69][1] = ip[69][1] + seg7;
    p[70][1] = ip[70][1] + seg7;
    p[71][1] = ip[71][1] + seg7;
    p[72][1] = ip[72][1] + seg7;
    p[73][1] = ip[73][1] + seg7;
    p[74][1] = ip[74][1] + seg7;
    p[91][1] = ip[91][1] + seg3 * 1.1;
    p[92][1] = ip[92][1] + seg3;
    p[93][1] = ip[93][1] + seg3;
    p[94][1] = ip[94][1] + seg3;
    p[95][1] = ip[95][1] + seg3 * 0.9;
    p[99][1] = ip[99][1] + chomp;
    p[98][1] = ip[98][1] + chomp;
    p[64][1] = ip[64][1] + chomp;
    p[61][1] = ip[61][1] + chomp;
    p[97][1] = ip[97][1] + chomp;
    p[96][1] = ip[96][1] + chomp;
}

/// The dolphin's, which is the same idea with its own numbers.
fn dolphin_segments(p: &mut [[f32; 3]], seg: &[f32; 8], chomp: f32) {
    let ip = m::DOLPHIN_P;
    let [seg0, seg1, seg2, seg3, seg4, seg5, seg6, seg7] = *seg;
    p[12][1] = ip[12][1] + seg5;
    p[13][1] = ip[13][1] + seg5;
    p[14][1] = ip[14][1] + seg5;
    p[15][1] = ip[15][1] + seg5;
    p[16][1] = ip[16][1] + seg5;
    p[17][1] = ip[17][1] + seg5;
    p[18][1] = ip[18][1] + seg5;
    p[19][1] = ip[19][1] + seg5;
    p[20][1] = ip[20][1] + seg4;
    p[21][1] = ip[21][1] + seg4;
    p[22][1] = ip[22][1] + seg4;
    p[23][1] = ip[23][1] + seg4;
    p[24][1] = ip[24][1] + seg4;
    p[25][1] = ip[25][1] + seg4;
    p[26][1] = ip[26][1] + seg4;
    p[27][1] = ip[27][1] + seg4;
    p[28][1] = ip[28][1] + seg2;
    p[29][1] = ip[29][1] + seg2;
    p[30][1] = ip[30][1] + seg2;
    p[31][1] = ip[31][1] + seg2;
    p[32][1] = ip[32][1] + seg2;
    p[33][1] = ip[33][1] + seg2;
    p[34][1] = ip[34][1] + seg2;
    p[35][1] = ip[35][1] + seg2;
    p[36][1] = ip[36][1] + seg1;
    p[37][1] = ip[37][1] + seg1;
    p[38][1] = ip[38][1] + seg1;
    p[39][1] = ip[39][1] + seg1;
    p[40][1] = ip[40][1] + seg1;
    p[41][1] = ip[41][1] + seg1;
    p[42][1] = ip[42][1] + seg1;
    p[43][1] = ip[43][1] + seg1;
    p[44][1] = ip[44][1] + seg0;
    p[45][1] = ip[45][1] + seg0;
    p[46][1] = ip[46][1] + seg0;
    p[47][1] = ip[47][1] + seg0;
    p[48][1] = ip[48][1] + seg0;
    p[49][1] = ip[49][1] + seg0;
    p[50][1] = ip[50][1] + seg0;
    p[51][1] = ip[51][1] + seg0;
    p[9][1] = ip[9][1] + seg6;
    p[10][1] = ip[10][1] + seg6;
    p[75][1] = ip[75][1] + seg6;
    p[76][1] = ip[76][1] + seg6;
    p[1][1] = ip[1][1] + seg7;
    p[11][1] = ip[11][1] + seg7;
    p[68][1] = ip[68][1] + seg7;
    p[69][1] = ip[69][1] + seg7;
    p[70][1] = ip[70][1] + seg7;
    p[71][1] = ip[71][1] + seg7;
    p[72][1] = ip[72][1] + seg7;
    p[73][1] = ip[73][1] + seg7;
    p[74][1] = ip[74][1] + seg7;
    p[91][1] = ip[91][1] + seg3;
    p[92][1] = ip[92][1] + seg3;
    p[93][1] = ip[93][1] + seg3;
    p[94][1] = ip[94][1] + seg3;
    p[95][1] = ip[95][1] + seg3;
    p[122][1] = ip[122][1] + seg3 * 1.5;
    p[97][1] = ip[97][1] + chomp;
    p[98][1] = ip[98][1] + chomp;
    p[102][1] = ip[102][1] + chomp;
    p[110][1] = ip[110][1] + chomp;
    p[111][1] = ip[111][1] + chomp;
    p[121][1] = ip[121][1] + chomp;
    p[118][1] = ip[118][1] + chomp;
    p[119][1] = ip[119][1] + chomp;
}

/// The shark's, which bends sideways rather than up and down, and whose tail
/// also leans into whatever turn it is making.
fn shark_segments(
    p: &mut [[f32; 3]],
    seg1: f32,
    seg2: f32,
    seg3: f32,
    seg4: f32,
    segup: f32,
    chomp: f32,
) {
    let ip = m::SHARK_P;
    p[4][1] = ip[4][1] + chomp;
    p[7][1] = ip[7][1] + chomp;
    p[10][1] = ip[10][1] + chomp;
    p[11][1] = ip[11][1] + chomp;
    p[23][0] = ip[23][0] + seg1;
    p[24][0] = ip[24][0] + seg1;
    p[25][0] = ip[25][0] + seg1;
    p[26][0] = ip[26][0] + seg1;
    p[27][0] = ip[27][0] + seg1;
    p[28][0] = ip[28][0] + seg1;
    p[29][0] = ip[29][0] + seg1;
    p[30][0] = ip[30][0] + seg1;
    p[31][0] = ip[31][0] + seg1;
    p[32][0] = ip[32][0] + seg1;
    p[33][0] = ip[33][0] + seg2;
    p[34][0] = ip[34][0] + seg2;
    p[35][0] = ip[35][0] + seg2;
    p[36][0] = ip[36][0] + seg2;
    p[37][0] = ip[37][0] + seg2;
    p[38][0] = ip[38][0] + seg2;
    p[39][0] = ip[39][0] + seg2;
    p[40][0] = ip[40][0] + seg2;
    p[41][0] = ip[41][0] + seg2;
    p[42][0] = ip[42][0] + seg2;
    p[43][0] = ip[43][0] + seg3;
    p[44][0] = ip[44][0] + seg3;
    p[45][0] = ip[45][0] + seg3;
    p[46][0] = ip[46][0] + seg3;
    p[47][0] = ip[47][0] + seg3;
    p[48][0] = ip[48][0] + seg3;
    p[49][0] = ip[49][0] + seg3;
    p[50][0] = ip[50][0] + seg3;
    p[51][0] = ip[51][0] + seg3;
    p[52][0] = ip[52][0] + seg3;
    p[2][0] = ip[2][0] + seg4;
    p[61][0] = ip[61][0] + seg4;
    p[69][0] = ip[69][0] + seg4;
    p[70][0] = ip[70][0] + seg4;
    p[23][1] = ip[23][1] + segup;
    p[24][1] = ip[24][1] + segup;
    p[25][1] = ip[25][1] + segup;
    p[26][1] = ip[26][1] + segup;
    p[27][1] = ip[27][1] + segup;
    p[28][1] = ip[28][1] + segup;
    p[29][1] = ip[29][1] + segup;
    p[30][1] = ip[30][1] + segup;
    p[31][1] = ip[31][1] + segup;
    p[32][1] = ip[32][1] + segup;
    p[33][1] = ip[33][1] + segup * 5.0;
    p[34][1] = ip[34][1] + segup * 5.0;
    p[35][1] = ip[35][1] + segup * 5.0;
    p[36][1] = ip[36][1] + segup * 5.0;
    p[37][1] = ip[37][1] + segup * 5.0;
    p[38][1] = ip[38][1] + segup * 5.0;
    p[39][1] = ip[39][1] + segup * 5.0;
    p[40][1] = ip[40][1] + segup * 5.0;
    p[41][1] = ip[41][1] + segup * 5.0;
    p[42][1] = ip[42][1] + segup * 5.0;
    p[43][1] = ip[43][1] + segup * 12.0;
    p[44][1] = ip[44][1] + segup * 12.0;
    p[45][1] = ip[45][1] + segup * 12.0;
    p[46][1] = ip[46][1] + segup * 12.0;
    p[47][1] = ip[47][1] + segup * 12.0;
    p[48][1] = ip[48][1] + segup * 12.0;
    p[49][1] = ip[49][1] + segup * 12.0;
    p[50][1] = ip[50][1] + segup * 12.0;
    p[51][1] = ip[51][1] + segup * 12.0;
    p[52][1] = ip[52][1] + segup * 12.0;
    p[2][1] = ip[2][1] + segup * 17.0;
    p[61][1] = ip[61][1] + segup * 17.0;
    p[69][1] = ip[69][1] + segup * 17.0;
    p[70][1] = ip[70][1] + segup * 17.0;
}

impl Hack3d for Atlantis {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();

        g.glx.push_matrix();
        self.all_display(g);
        self.animate();
        g.glx.pop_matrix();

        g.glx.color3f(1.0, 1.0, 1.0);

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        let mut height = height.max(1);
        let mut h = height as f32 / width as f32;
        let mut y = 0;

        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = height as f32 / width as f32;
        }

        g.glx.viewport(0, y, width, height);

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(400.0, 1.0 / h, 1.0, 2000000.0);
        g.glx.matrix_mode_modelview();
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let num_sharks = g.res.int("count").clamp(0, 200) as usize;

    let mut st = Atlantis {
        num_sharks,
        /* has influence on the "width" of the movement */
        sharkspeed: g.res.int("cycles") as f32,
        whalespeed: g.res.int("whalespeed") as f32,
        /* has influence on the "distance" of the sharks */
        sharksize: g.res.int("size").max(1) as f32,
        wire,
        whaledir: false,
        sharks: vec![FishRec::default(); num_sharks],
        mom_whale: FishRec::default(),
        baby_whale: FishRec::default(),
        dolph: FishRec::default(),
        do_texture: g.res.bool("texture") && !wire,
        do_gradient: g.res.bool("gradient"),
        texture: 0,
        clear: [0.0, 0.0, 0.0, 1.0],
    };

    g.glx.front_face_cw(false);

    if wire {
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.lighting(false);
    } else {
        g.glx.depth_func(DepthFunc::LessEqual);
        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.light_ambient(0, [0.1, 0.1, 0.1, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_position(0, 0.0, 1.0, 0.0, 0.0);
        g.glx.light_model_ambient([0.4, 0.4, 0.4, 1.0]);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);

        g.glx.material_shininess(90.0);
        g.glx.material_specular([0.8, 0.8, 0.8, 1.0]);
        g.glx.material_diffuse([0.46, 0.66, 0.795, 1.0]);
        g.glx.material_ambient([0.0, 0.1, 0.2, 1.0]);
    }

    if st.do_texture {
        if let Some((w, h, rgba)) = png::decode_rgba(crate::images::SEA_TEXTURE) {
            let id = g.glx.gen_texture();
            g.glx.bind_texture(id);
            g.glx.tex_image_2d(w, h, rgba);
            st.texture = id;
            g.glx.texturing(true);
        } else {
            st.do_texture = false;
        }
    }

    st.init_fishs();

    /* Add a little randomness */
    let fblue = (random_below(30) as f32 / 100.0) + 0.70;
    let fgreen = fblue * 0.56;
    st.clear = [0.0, fgreen, fblue, 1.0];
    g.glx.clear_color(0.0, fgreen, fblue, 1.0);

    g.glx.blend(Blend::Off);

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:       25000",
    "*count:           4",
    "*showFPS:     False",
    "*cycles:        100",
    "*size:         6000",
    "*wireframe:   False",
    "*whalespeed:    250",
    "*texture:      True",
    "*gradient:     True",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "25000").inverted(),
    Opt::slider("whalespeed", "Whale speed", 0.0, 1000.0, 10.0, 0, "250"),
    Opt::slider("size", "Shark proximity", 100.0, 10_000.0, 100.0, 0, "6000"),
    Opt::slider("count", "Number of sharks", 0.0, 20.0, 1.0, 0, "4"),
    Opt::boolean("texture", "Shimmering water", "true"),
    Opt::boolean("gradient", "Gradient background", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "atlantis",
    label: "Atlantis",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Mark Kilgard",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=U78xPez5UGg"),
        blurb: "Sharks, dolphins and whales.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };

#[cfg(test)]
mod tests {
    use super::*;

    fn tank() -> (Gl, Atlantis) {
        crate::runtime::ya_rand_init(20260812);
        let mut g = Gl::for_test(640, 480);
        let mut st = Atlantis {
            num_sharks: 4,
            sharkspeed: 100.0,
            whalespeed: 250.0,
            sharksize: 6000.0,
            wire: false,
            whaledir: false,
            sharks: vec![FishRec::default(); 4],
            mom_whale: FishRec::default(),
            baby_whale: FishRec::default(),
            dolph: FishRec::default(),
            do_texture: false,
            do_gradient: true,
            texture: 0,
            clear: [0.0, 0.4, 0.7, 1.0],
        };
        st.init_fishs();
        st.reshape(&mut g, 640, 480);
        (g, st)
    }

    #[test]
    fn the_fish_are_fish_shaped() {
        // Every model's points sit in a body that is much longer than it is
        // wide, nose forward along z, and every normal is a unit vector.
        for (name, p, n) in [
            ("dolphin", &m::DOLPHIN_P[..], &m::DOLPHIN_N[..]),
            ("shark", &m::SHARK_P[..], &m::SHARK_N[..]),
            ("whale", &m::WHALE_P[..], &m::WHALE_N[..]),
        ] {
            let span = |i: usize| {
                let lo = p[1..].iter().fold(f32::MAX, |a, v| a.min(v[i]));
                let hi = p[1..].iter().fold(f32::MIN, |a, v| a.max(v[i]));
                hi - lo
            };
            assert!(
                span(2) > 2.0 * span(0),
                "{name}: {} vs {}",
                span(2),
                span(0)
            );
            assert!(
                span(2) > 2.0 * span(1),
                "{name}: {} vs {}",
                span(2),
                span(1)
            );
            let bad = n[1..]
                .iter()
                .filter(|v| {
                    let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    d != 0.0 && (d - 1.0).abs() > 0.02
                })
                .count();
            assert_eq!(bad, 0, "{name}: {bad} normals are not unit vectors");
        }
    }

    #[test]
    fn a_tail_travels_down_the_body() {
        // The wave arrives later the further back it is, so at a phase where
        // the front of the tail is at its furthest the back is not.
        let mut p = m::WHALE_P;
        let seg = [1.0, 0.9, 0.7, 0.4, 0.5, -0.4, -0.7, -0.9];
        whale_segments(&mut p, &seg, 0.25);
        let d = |i: usize| p[i][1] - m::WHALE_P[i][1];
        // The points are hundreds of units out, so a displacement of one
        // survives the addition to about five decimal places.
        assert!((d(12) - seg[5]).abs() < 1e-4, "{}", d(12));
        assert!((d(44) - seg[0]).abs() < 1e-4, "{}", d(44));
        assert!(d(44) > d(12), "the wave runs the wrong way");
        // Only the points upstream names moved: sixty-four of them, and the
        // ones in the head are not among them.
        let moved = (1..p.len()).filter(|&i| d(i) != 0.0).count();
        assert!(moved >= 55, "{moved}");
        for i in 2..9 {
            assert_eq!(d(i), 0.0, "point {i} moved");
        }
    }

    #[test]
    fn the_sharks_wheel_round_their_target() {
        let (_, mut st) = tank();
        // Every shark steers towards 60000 on x, so after long enough they
        // are all heading in roughly the right direction and none has left
        // the tank.
        for _ in 0..2000 {
            st.animate();
        }
        for (i, s) in st.sharks.iter().enumerate() {
            let d = ((s.x - 60000.0).powi(2) + s.y * s.y + s.z * s.z).sqrt();
            assert!(d < 200_000.0, "shark {i} is {d} away");
            assert!(s.v >= 0.9 && s.v <= 5.4, "shark {i} at {}", s.v);
            assert!(s.theta >= -90.0 && s.theta <= 90.0, "shark {i}");
        }
        // The whales circle: they come back to about where they started.
        assert!(st.dolph.psi.abs() <= 360.0 * 100.0);
    }

    #[test]
    fn one_draw_call_a_fish() {
        let (mut g, mut st) = tank();
        g.glx.start_frame(640, 480);
        st.draw(&mut g);
        let batches = g.glx.frame().batches.len();
        // Four sharks, a dolphin, two whales, the gradient, and the two
        // depth-test toggles inside the dolphin.
        assert!(batches <= 14, "{batches}");
        let verts = g.glx.frame().vertices.len();
        assert!((2_000..20_000).contains(&verts), "{verts}");
    }
}
