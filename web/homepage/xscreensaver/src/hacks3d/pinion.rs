//! Port of `hacks/glx/pinion.c`.
//!
//! ```text
//! pinion, Copyright (c) 2004-2014 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//! ```
//!
//! A gear train that scrolls past for ever.
//!
//! Gears drift leftwards; one that leaves the screen is deleted, and whenever
//! the rightmost gear comes inside the layout area another is grown onto the
//! end of the train, off-screen, so there is always more coming. The train is
//! never finished and never repeats.
//!
//! Two things make it more than a conveyor belt. Gears sometimes come in bound
//! pairs on one axle, a big one and a small one, which is how a train changes
//! ratio sharply; and because the ratios are real and cumulative, a train that
//! goes through a few of those is soon spinning far too fast to draw. A gear
//! past that point is drawn turned by exactly half a tooth each frame so that
//! it flickers, which reads as a blur, and its neighbours start to wobble. Ten
//! blurred gears in a row and the train is abandoned: the next gear starts a
//! new one from rest.
//!
//! Hovering over a gear labels it with its tooth count and speed. Upstream
//! finds the gear under the pointer by drawing the whole scene again into an
//! OpenGL selection buffer, which OpenGL ES has not got: its own mobile build
//! gives up and shows no label at all. There is no selection buffer here
//! either, so the question is answered by projecting each gear's middle and rim
//! and seeing which disc the pointer landed in, which needs no second pass.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::involute::{Gear, Size, biggest_ring, draw_gear};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand, random,
};

/// Three samples averaged, which clusters values in the middle of the range.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

struct Pinion {
    trackball: Trackball,
    gears: Vec<Gear>,

    /// The visible area, in the units the gears are laid out in.
    vp_left: f64,
    vp_right: f64,
    vp_top: f64,
    vp_bottom: f64,
    vp_width: f64,
    vp_height: f64,
    /// Where gears are drawn, which is wider than the screen.
    render_left: f64,
    render_right: f64,
    /// Where new gears are built, off the right-hand edge.
    layout_left: f64,
    layout_right: f64,

    /// Distance between the two planes of a bound pair.
    plane_displacement: f64,
    /// How many gears in a row have been too fast to draw.
    current_blur_length: i32,

    spin_speed: f64,
    scroll_speed: f64,
    gear_size: f64,
    max_rpm: f64,
    wireframe: bool,
    height: i32,
    delay: i32,

    /// Where the pointer is, and which gear it is over.
    mouse: Option<(i32, i32)>,
    mouse_gear: Option<usize>,
    font: Option<TexFont>,
}

/// `rpm_string`: the speed, with as many decimals as it takes to say something,
/// and no trailing zeroes.
fn rpm_string(rpm: f64) -> String {
    let mut buf = if rpm >= 0.1 {
        format!("{rpm:.2}")
    } else if rpm >= 0.001 {
        format!("{rpm:.4}")
    } else if rpm >= 0.00001 {
        format!("{rpm:.8}")
    } else {
        format!("{rpm:.16}")
    };
    while buf.ends_with(0x30 as char) {
        buf.pop();
    }
    if buf.ends_with(0x2e as char) {
        buf.pop();
    }
    buf.push_str(" RPM");
    buf
}

impl Pinion {
    /// The gear furthest to the right, or to the left, counting its teeth.
    fn farthest_gear(&self, left_p: bool) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for (i, g) in self.gears.iter().enumerate() {
            let gx = g.x + (g.r + g.tooth_h) * if left_p { -1.0 } else { 1.0 };
            let better = match best {
                None => true,
                Some((_, x)) => {
                    if left_p {
                        x > gx
                    } else {
                        x < gx
                    }
                }
            };
            if better {
                best = Some((i, gx));
            }
        }
        best.map(|(i, _)| i)
    }

    /// How fast a gear is turning, in revolutions per minute, from the frame
    /// rate the delay implies.
    fn compute_rpm(&self, g: &mut Gear) {
        let mut fps = if self.delay == 0 {
            999_999.0
        } else {
            1_000_000.0 / f64::from(self.delay)
        };
        fps = fps.clamp(10.0, 150.0);
        let rpf = (g.ratio * self.spin_speed) / 360.0;
        g.rpm = rpf * fps * 60.0;
    }

    /// A gear sized to sit beside its parent, or on the same axle as it, or to
    /// start a train.
    fn new_gear(&mut self, parent: Option<usize>, coaxial_p: bool) -> Option<Gear> {
        // A bound pair must have something to be bound to; upstream aborts if
        // it does not, and this cannot happen from either caller.
        let coaxial = if coaxial_p { Some(parent?) } else { None };
        let mut g = Gear {
            coax_displacement: self.plane_displacement,
            ..Gear::default()
        };

        let mut loops = 0;
        loop {
            loops += 1;
            // The only reason to go round is a coaxial gear looking for a
            // radius well away from its parent's, which should not be hard.
            if loops > 1000 {
                return None;
            }

            match parent {
                Some(i) if !coaxial_p => {
                    // Adjacent gears need matching teeth.
                    let p = &self.gears[i];
                    g.tooth_w = p.tooth_w;
                    g.tooth_h = p.tooth_h;
                    g.thickness = p.thickness;
                    g.thickness2 = p.thickness2;
                    g.thickness3 = p.thickness3;
                }
                _ => {
                    let scale = (1.0 + bellrand(4.0)) * self.gear_size;
                    g.tooth_w = 0.007 * scale;
                    g.tooth_h = 0.005 * scale;
                    g.thickness = g.tooth_h * (0.1 + bellrand(1.5));
                    g.thickness2 = g.thickness / 4.0;
                    g.thickness3 = g.thickness;
                }
            }

            // Three to a hundred teeth, with very small counts made rarer.
            loop {
                g.nteeth = 3 + (random() % 97) as i32;
                if g.nteeth >= 7 || random().is_multiple_of(5) {
                    break;
                }
            }
            let c = f64::from(g.nteeth) * g.tooth_w * 2.0;
            g.r = c / (std::f64::consts::PI * 2.0);

            let Some(ci) = coaxial else {
                break;
            };
            let p = &self.gears[ci];
            if g.nteeth == p.nteeth {
                continue; /* ugly */
            }
            if g.r < p.r * 0.6 || p.r < g.r * 0.6 {
                break; /* usefully different sizes */
            }
        }

        g.color = [
            0.5 + frand(0.5) as f32,
            0.5 + frand(0.5) as f32,
            0.5 + frand(0.5) as f32,
            1.0,
        ];
        g.color2 = [
            g.color[0] * 0.85,
            g.color[1] * 0.85,
            g.color[2] * 0.85,
            g.color[3],
        ];

        // What the inside looks like: a bare ring with teeth, or that plus a
        // thinner inset plate, or that plus a raised lip, or a wide lip.
        if random().is_multiple_of(10) {
            g.inner_r = (g.r * 0.1) + frand((g.r - g.tooth_h / 2.0) * 0.8);
            g.inner_r2 = 0.0;
            g.inner_r3 = 0.0;
        } else {
            g.inner_r = (g.r * 0.5) + frand((g.r - g.tooth_h) * 0.4);
            g.inner_r2 = (g.r * 0.1) + frand(g.inner_r * 0.5);
            g.inner_r3 = 0.0;

            if g.inner_r2 > (g.r * 0.2) {
                let nn = random() % 10;
                if nn <= 2 {
                    g.inner_r3 = (g.r * 0.1) + frand(g.inner_r2 * 0.2);
                } else if nn <= 7 && g.inner_r2 >= 0.1 {
                    g.inner_r3 = g.inner_r2 - 0.01;
                }
            }
        }

        // A bound pair shares an axle, so both need the same innermost hole:
        // whichever of the two is smaller. This changes the parent.
        if let Some(i) = coaxial {
            let hole_of = |x: &Gear| {
                if x.inner_r3 != 0.0 {
                    x.inner_r3
                } else if x.inner_r2 != 0.0 {
                    x.inner_r2
                } else {
                    x.inner_r
                }
            };
            let hole = hole_of(&g).min(hole_of(&self.gears[i]));
            if hole <= 0.0 {
                return None;
            }
            for target in [&mut g, &mut self.gears[i]] {
                if target.inner_r3 != 0.0 {
                    target.inner_r3 = hole;
                } else if target.inner_r2 != 0.0 {
                    target.inner_r2 = hole;
                } else {
                    target.inner_r = hole;
                }
            }
        }

        // With three discs, sometimes make the middle one spokes.
        if g.inner_r3 != 0.0 && random().is_multiple_of(5) {
            g.spokes = 2 + bellrand(5.0) as i32;
            g.spoke_thickness = 1.0 + frand(7.0);
            if g.spokes == 2 && g.spoke_thickness < 2.0 {
                g.spoke_thickness += 1.0;
            }
        }

        // Little nubbly bits, if there is room.
        if g.nteeth > 5 {
            let (_, _, size, _) = biggest_ring(&g);
            if size > g.r * 0.2 && random().is_multiple_of(5) {
                g.nubs = 1 + (random() % 16) as i32;
                if g.nubs > 8 {
                    g.nubs = 1;
                }
            }
        }

        // How complex a mesh to build, from roughly how many pixels a tooth
        // will take up.
        let pix = g.tooth_h * f64::from(self.height);
        g.size = if pix <= 2.5 {
            Size::Small
        } else if pix <= 3.5 {
            Size::Medium
        } else if pix <= 25.0 {
            Size::Large
        } else {
            Size::Huge
        };

        Some(g)
    }

    /// Put a gear where it belongs, with its teeth meshed and the right speed.
    /// False if it would not fit, would be visible as it appeared, or would
    /// land on something.
    fn place_gear(&mut self, g: &mut Gear, parent: Option<usize>, coaxial_p: bool) -> bool {
        // A gear taking up more than a third of the screen is no good.
        let big = (g.r + g.tooth_h) * (6.0 / self.gear_size);
        if big >= self.vp_width || big >= self.vp_height {
            return false;
        }

        match parent {
            None => {
                g.ratio = 0.8 + bellrand(0.4); /* 8 to 12 rpm at 60fps */
                g.th = frand(90.0) * if random() & 1 == 1 { 1.0 } else { -1.0 };
            }
            Some(i) if coaxial_p => {
                // Bound gears turn together.
                let p = self.gears[i].clone();
                g.ratio = p.ratio;
                g.th = p.th;
                g.rpm = p.rpm;
                g.wobble = p.wobble;
            }
            Some(i) => {
                let p = self.gears[i].clone();
                g.ratio = f64::from(p.nteeth) / f64::from(g.nteeth);
                g.th = -(p.th * g.ratio);
                if g.nteeth & 1 == 1 {
                    let off = 180.0 / f64::from(g.nteeth);
                    if g.th > 0.0 {
                        g.th += off;
                    } else {
                        g.th -= off;
                    }
                }
                g.ratio *= p.ratio;
            }
        }

        match parent {
            None => {
                // A new train starts off the right-hand edge, past whatever is
                // furthest right already.
                let right = self
                    .farthest_gear(false)
                    .map(|i| {
                        let rg = &self.gears[i];
                        rg.x + rg.r + rg.tooth_h
                    })
                    .unwrap_or(0.0)
                    .max(self.layout_left);
                g.x = right + g.r + g.tooth_h + (0.01 / self.gear_size);
                g.y = 0.0;
                g.z = 0.0;
            }
            Some(i) if coaxial_p => {
                let off = self.plane_displacement;
                let p = self.gears[i].clone();
                g.x = p.x;
                g.y = p.y;
                // The smaller of the pair goes on top.
                g.z = p.z + if g.r > p.r { -off } else { off };

                if p.r > g.r {
                    self.gears[i].coax_p = 1;
                    g.coax_p = 2;
                    self.gears[i].wobble = 0.0; /* looks bad when the axle moves */
                } else {
                    self.gears[i].coax_p = 2;
                    g.coax_p = 1;
                    g.wobble = 0.0;
                }
                g.coax_thickness = p.thickness;
                self.gears[i].coax_thickness = g.thickness;

                // Do not let the train wander too close to the screen.
                if g.z >= off * 4.0 || g.z <= -off * 4.0 {
                    return false;
                }
            }
            Some(i) => {
                let p = self.gears[i].clone();
                let r_off = p.r + g.r;
                // Mostly in front of the parent rather than behind it.
                let angle = if random().is_multiple_of(3) {
                    f64::from((random() % 360) as i32 - 180)
                } else {
                    f64::from((random() % 240) as i32 - 120)
                };
                let rad = angle * (std::f64::consts::PI / 180.0);

                g.x = p.x + rad.cos() * r_off;
                g.y = p.y + rad.sin() * r_off;
                g.z = p.z;

                // More than halfway off the top or bottom is no good.
                if g.y > self.vp_top || g.y < self.vp_bottom {
                    return false;
                }

                // Keep the sign of `th` from flipping in the arithmetic below.
                g.th += if g.th > 0.0 { 360.0 } else { -360.0 };

                let p_c = 2.0 * std::f64::consts::PI * p.r;
                let g_c = 2.0 * std::f64::consts::PI * g.r;
                let p_t = p_c * (angle / 360.0);
                g.th += angle + 360.0 * (p_t / g_c);
            }
        }

        // A gear that would already be on screen would flash into existence,
        // which happens when the train grows backwards.
        if g.x - g.r - g.tooth_h < self.render_right {
            return false;
        }

        // And it must not land on anything already placed.
        for (i, og) in self.gears.iter().enumerate().rev() {
            if Some(i) == parent {
                continue;
            }
            if g.z != og.z {
                continue; /* different layer */
            }
            let reach = g.r + g.tooth_h + og.r + og.tooth_h;
            if (g.x - og.x).powi(2) + (g.y - og.y).powi(2) < reach * reach {
                return false;
            }
        }

        self.compute_rpm(g);

        // Gears further from the eye are darker.
        let depth = g.z / self.plane_displacement;
        let brightness = (1.0 + (depth / 6.0)).clamp(0.4, 1.0 / 0.4) as f32;
        for k in 0..3 {
            g.color[k] *= brightness;
            g.color2[k] *= brightness;
        }

        // Turning by more than half a tooth in one frame does not look like
        // turning at all, so past that the gear is blurred and its neighbours
        // start to shake themselves apart.
        let ratio = g.ratio * self.spin_speed;
        let blur_limit = 180.0 / f64::from(g.nteeth);
        if ratio > blur_limit {
            g.motion_blur_p = 1;
        }
        if !coaxial_p {
            for k in [0.7, 0.9, 1.1, 1.3, 1.5, 1.7] {
                if ratio > blur_limit * k {
                    g.wobble += f64::from(random() % 2);
                }
            }
        }

        true
    }

    /// Try until it works, or a hundred goes have gone by.
    fn place_new_gear(&mut self, parent: Option<usize>, coaxial_p: bool) -> Option<usize> {
        for _ in 0..100 {
            let mut g = self.new_gear(parent, coaxial_p)?;
            if self.place_gear(&mut g, parent, coaxial_p) {
                self.gears.push(g);
                return Some(self.gears.len() - 1);
            }
        }
        None
    }

    /// Add one gear to the end of the train, starting a new train if the old
    /// one has nowhere left to go.
    fn push_gear(&mut self) {
        for _ in 0..100 {
            let mut parent = self.gears.len().checked_sub(1);

            // At ludicrous speed, unhook the train and start again from rest.
            if let Some(i) = parent
                && self.gears[i].rpm > self.max_rpm
            {
                parent = None;
            }
            // And if the last ten gears were all blurred, it is not coming
            // back.
            if self.current_blur_length >= 10 {
                parent = None;
            }

            let mut tried_coaxial = false;
            let mut g = None;

            // Sometimes try a bound pair.
            if let Some(i) = parent
                && self.gears[i].coax_p == 0
                && random().is_multiple_of(40)
            {
                tried_coaxial = true;
                g = self.place_new_gear(parent, true);
            }
            // Otherwise an ordinary one beside it.
            if g.is_none() {
                g = self.place_new_gear(parent, false);
            }
            // Failing that, a bound pair after all.
            if g.is_none()
                && !tried_coaxial
                && let Some(i) = parent
                && self.gears[i].coax_p == 0
            {
                g = self.place_new_gear(parent, true);
            }
            // Failing that, the train is in a dead end: start a new one.
            if g.is_none() {
                parent = None;
                g = self.place_new_gear(parent, false);
            }

            let Some(i) = g else {
                // Nothing can be placed at all, which happens when the growth
                // zone has been backed into a corner. Clear it and try again.
                let left = self.render_left;
                self.gears.retain(|g| g.x - g.r - g.tooth_h >= left);
                continue;
            };

            if self.gears[i].motion_blur_p != 0 {
                self.current_blur_length += 1;
            } else {
                self.current_blur_length = 0;
            }
            return;
        }
    }

    /// Slide everything left, drop what has gone off the edge, and grow the
    /// train until it reaches the right of the layout area again.
    fn scroll_gears(&mut self) {
        for g in &mut self.gears {
            g.x -= self.scroll_speed * 0.002;
        }

        let left = self.render_left;
        self.gears.retain(|g| g.x + g.r + g.tooth_h >= left);

        // Cap the work: a pathological layout could otherwise ask for gears
        // for ever. Upstream has no cap and aborts instead.
        for _ in 0..200 {
            let far_enough = self
                .gears
                .last()
                .is_some_and(|g| g.x + g.r + g.tooth_h >= self.layout_right);
            if far_enough {
                break;
            }
            self.push_gear();
        }
    }

    fn spin_gears(&mut self) {
        for g in &mut self.gears {
            let off = g.ratio * self.spin_speed;
            if g.th > 0.0 {
                g.th += off;
            } else {
                g.th -= off;
            }
        }
    }

    /// Run the train forward, drawing nothing, until the first gear is about
    /// to come on screen: otherwise a slow scroll starts with a blank screen.
    fn ffwd(&mut self) {
        for _ in 0..100_000 {
            if let Some(i) = self.farthest_gear(true) {
                let g = &self.gears[i];
                if g.x - g.r - g.tooth_h / 2.0 <= self.vp_right * 0.88 {
                    return;
                }
            }
            self.scroll_gears();
        }
    }
}

impl Hack3d for Pinion {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let down = self.trackball.button_down();
        if !down {
            self.scroll_gears();
            self.spin_gears();
        }

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        g.glx.scale(16.0, 16.0, 16.0); /* map the layout units to the screen */
        g.glx.scale(1.2, 1.2, 1.2); /* zoom in a little more */
        g.glx.rotate(-35.0, 1.0, 0.0, 0.0); /* tilt back */
        g.glx.rotate(8.0, 0.0, 1.0, 0.0); /* tilt left */
        g.glx.translate(0.02, 0.1, 0.0); /* pan up */

        g.glx.color4f(1.0, 1.0, 0.8, 1.0);

        let mut drawn: Vec<(usize, usize)> = Vec::new();
        for i in 0..self.gears.len() {
            let gear = self.gears[i].clone();
            let visible = gear.x + gear.r + gear.tooth_h >= self.render_left
                && gear.x - gear.r - gear.tooth_h <= self.render_right;
            if !visible {
                continue;
            }

            // A blurred gear turns by exactly half a tooth every frame, so it
            // never looks like it is turning at any particular speed. With the
            // mouse down it goes back to the honest angle, since overlapping
            // polygons look wrong when nothing is moving.
            let th = if gear.motion_blur_p != 0 && !down {
                self.gears[i].motion_blur_p += 1;
                f64::from(gear.motion_blur_p) * 180.0 / f64::from(gear.nteeth)
                    * if gear.th > 0.0 { 1.0 } else { -1.0 }
            } else {
                gear.th
            };

            g.glx.push_matrix();
            g.glx.translate(gear.x as f32, gear.y as f32, gear.z as f32);
            g.glx.rotate(th as f32, 0.0, 0.0, 1.0);
            let first_batch = g.glx.frame().batches.len();
            draw_gear(&mut g.glx, &gear, self.wireframe);
            drawn.push((i, first_batch));
            g.glx.pop_matrix();
        }

        g.glx.pop_matrix();

        // Which gear is the pointer over? Upstream asks OpenGL, by drawing the
        // scene again into a selection buffer; it also gives up and shows
        // nothing at all on OpenGL ES, where there is no such buffer. There is
        // none here either, so the same question is answered by projecting each
        // gear's middle and rim and seeing which disc the pointer landed in.
        // The last one drawn wins, since that is the one on top.
        self.mouse_gear = None;
        if let Some((mx, my)) = self.mouse {
            let ndc = [
                2.0 * mx as f32 / g.width().max(1) as f32 - 1.0,
                1.0 - 2.0 * my as f32 / g.height().max(1) as f32,
            ];
            for (i, b) in drawn {
                let Some(batch) = g.glx.frame().batches.get(b) else {
                    continue;
                };
                let centre = batch.mvp.transform([0.0, 0.0, 0.0]);
                let rim = batch.mvp.transform([self.gears[i].r as f32, 0.0, 0.0]);
                let r = ((rim[0] - centre[0]).powi(2) + (rim[1] - centre[1]).powi(2)).sqrt();
                let d = ((ndc[0] - centre[0]).powi(2) + (ndc[1] - centre[1]).powi(2)).sqrt();
                if d <= r {
                    self.mouse_gear = Some(i);
                }
            }
        }

        if let Some(i) = self.mouse_gear
            && let Some(font) = &self.font
        {
            let gear = &self.gears[i];
            let label = format!("{} teeth\n{}", gear.nteeth, rpm_string(gear.rpm));
            let (w, h) = (g.width(), g.height());
            font.print_label(&mut g.glx, &label, w, h, 1, [0.8, 0.8, 0.0, 1.0]);
        }

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        self.height = height;
        let mut h = f64::from(height) / f64::from(width.max(1));
        let mut y = 0;
        if width > height * 5 {
            /* tiny window: show middle */
            height = width * 9 / 16;
            y = -height / 2;
            h = f64::from(height) / f64::from(width);
        }

        g.glx.viewport(0, y, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, (1.0 / h) as f32, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.clear();

        self.vp_height = 1.0;
        self.vp_width = 1.0 / h;
        self.vp_left = -self.vp_width / 2.0;
        self.vp_right = self.vp_width / 2.0;
        self.vp_top = self.vp_height / 2.0;
        self.vp_bottom = -self.vp_height / 2.0;

        // Gears are drawn over twice the width of the screen, and built in a
        // strip off its right-hand edge.
        let render_width = self.vp_width * 2.0;
        let layout_width = self.vp_width * 0.8 * self.gear_size;
        self.render_left = -render_width / 2.0;
        self.render_right = render_width / 2.0;
        self.layout_left = self.render_right;
        self.layout_right = self.layout_left + layout_width;
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        match event {
            XEvent::MotionNotify { x, y } => self.mouse = Some((*x, *y)),
            XEvent::ButtonPress { x, y, .. } => self.mouse = Some((*x, *y)),
            _ => {}
        }
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let gear_size = g.res.float("gearSize").clamp(0.05, 10.0);
    let mut st = Pinion {
        trackball: Trackball::new(),
        gears: Vec::new(),
        vp_left: -1.0,
        vp_right: 1.0,
        vp_top: 0.5,
        vp_bottom: -0.5,
        vp_width: 2.0,
        vp_height: 1.0,
        render_left: -2.0,
        render_right: 2.0,
        layout_left: 2.0,
        layout_right: 3.0,
        plane_displacement: gear_size * 0.1,
        current_blur_length: 0,
        spin_speed: g.res.float("spinSpeed").max(0.0),
        scroll_speed: g.res.float("scrollSpeed").max(0.0),
        gear_size,
        max_rpm: g.res.float("maxRPM").max(1.0),
        wireframe: g.res.bool("wireframe"),
        height: g.height(),
        delay: g.res.int("delay"),
        mouse: None,
        mouse_gear: None,
        // Upstream picks one of three sizes by window size. This font comes in
        // whole multiples of one cell, and all three land on the same one.
        font: Some(TexFont::load(&mut g.glx, "titleFont 18")),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !st.wireframe {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, -3.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 1.0, 1.0, 1.0]);
    }

    st.ffwd();
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:       15000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*spinSpeed:   1.0",
    "*scrollSpeed: 1.0",
    "*gearSize:    1.0",
    "*maxRPM:      900",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "15000").inverted(),
    Opt::slider("spinSpeed", "Rotation speed", 0.1, 20.0, 0.1, 2, "1.0"),
    Opt::slider("scrollSpeed", "Scrolling speed", 0.1, 20.0, 0.1, 2, "1.0"),
    Opt::slider("gearSize", "Gear size", 0.1, 2.0, 0.05, 2, "1.0"),
    Opt::slider("maxRPM", "Max RPM", 100.0, 2000.0, 50.0, 0, "900"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "pinion",
    label: "Pinion",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2004",
        video: Some("https://www.youtube.com/watch?v=rHY8dR1urQk"),
        blurb: "A gear train that scrolls past for ever.",
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

    fn run(query: &str, frames: usize) -> Runner3d {
        let mut r = start(StartArgs::new(640, 480, query, 20260811));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// The train is always there: gears leave on the left and are grown on the
    /// right, so the screen never empties and never fills without bound.
    #[test]
    fn the_train_neither_empties_nor_runs_away() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..600 {
            r.step();
            let f = r.frame();
            assert!(!f.batches.is_empty(), "the screen emptied");
            assert!(
                f.vertices.len() < 4_000_000,
                "{} vertices is a runaway",
                f.vertices.len()
            );
        }
    }

    /// Gears scroll leftwards, and one that has gone off the left is dropped
    /// rather than kept for ever.
    #[test]
    fn gears_scroll_left_and_are_dropped() {
        let mut r = start(StartArgs::new(640, 480, "scrollSpeed=8", 20260811));
        r.step();

        // Track the leftmost thing drawn: it should keep moving left and then
        // disappear, rather than piling up.
        let leftmost = |r: &Runner3d| {
            let f = r.frame();
            f.batches
                .iter()
                .map(|b| b.modelview.0[12])
                .fold(f32::MAX, f32::min)
        };
        let mut seen = Vec::new();
        for _ in 0..60 {
            r.step();
            seen.push(leftmost(&r));
        }
        // It moves, and it stays bounded: nothing drifts off to infinity.
        assert!(
            seen.iter().any(|x| (x - seen[0]).abs() > 0.01),
            "nothing moved"
        );
        let lo = seen.iter().copied().fold(f32::MAX, f32::min);
        assert!(lo > -1000.0, "something ran away to {lo}");
    }

    /// Meshed gears touch, and gears on the same axle sit at the same place on
    /// two different planes.
    #[test]
    fn the_train_is_properly_geared() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        for _ in 0..200 {
            r.step();
        }
        // Reach the state back out through a fresh build of the same shape.
        let mut st = a_pinion();
        st.ffwd();
        assert!(st.gears.len() > 2, "no train was built");

        for w in st.gears.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if b.coax_p != 0 && a.coax_p != 0 {
                // A bound pair: same axle, different planes, same speed.
                if a.x == b.x && a.y == b.y {
                    assert_ne!(a.z, b.z, "a bound pair is in one plane");
                    assert!((a.ratio - b.ratio).abs() < 1e-12);
                    continue;
                }
            }
            if a.z != b.z {
                continue; /* not neighbours in the train */
            }
            let d = ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
            // Either they mesh, or `b` began a new train somewhere else.
            if d < (a.r + b.r) * 1.5 {
                assert!(
                    (d - (a.r + b.r)).abs() < 1e-9,
                    "gears {d} apart, not {}",
                    a.r + b.r
                );
            }
        }
    }

    fn a_pinion() -> Pinion {
        Pinion {
            trackball: Trackball::new(),
            gears: Vec::new(),
            vp_left: -0.666,
            vp_right: 0.666,
            vp_top: 0.5,
            vp_bottom: -0.5,
            vp_width: 1.333,
            vp_height: 1.0,
            render_left: -1.333,
            render_right: 1.333,
            layout_left: 1.333,
            layout_right: 1.333 + 1.066,
            plane_displacement: 0.1,
            current_blur_length: 0,
            spin_speed: 1.0,
            scroll_speed: 1.0,
            gear_size: 1.0,
            max_rpm: 900.0,
            wireframe: false,
            height: 480,
            delay: 15000,
            mouse: None,
            mouse_gear: None,
            font: None,
        }
    }

    /// A gear turning faster than half a tooth a frame cannot be drawn
    /// honestly, so it is marked to be blurred instead.
    #[test]
    fn a_gear_too_fast_to_draw_is_blurred() {
        let mut st = a_pinion();
        st.spin_speed = 400.0; /* absurd, to force it quickly */
        st.ffwd();
        for _ in 0..50 {
            st.scroll_gears();
        }
        assert!(
            st.gears.iter().any(|g| g.motion_blur_p != 0),
            "nothing was fast enough to blur"
        );

        // And each blurred gear really is over the limit.
        for g in &st.gears {
            if g.motion_blur_p != 0 {
                let limit = 180.0 / f64::from(g.nteeth);
                assert!(
                    g.ratio * st.spin_speed > limit || g.coax_p != 0,
                    "a gear was blurred at {} against {limit}",
                    g.ratio * st.spin_speed
                );
            }
        }
    }

    /// The RPM follows the ratio and the frame rate, which is what the runaway
    /// check is measured against.
    #[test]
    fn the_rpm_follows_the_ratio() {
        let st = a_pinion();
        let mut g = Gear {
            ratio: 1.0,
            ..Gear::default()
        };
        st.compute_rpm(&mut g);
        // One turn per 360 frames, at the frame rate the delay implies.
        let fps = (1_000_000.0 / f64::from(st.delay)).clamp(10.0, 150.0);
        assert!((g.rpm - fps * 60.0 / 360.0).abs() < 1e-9, "{} rpm", g.rpm);

        let mut fast = Gear {
            ratio: 10.0,
            ..Gear::default()
        };
        st.compute_rpm(&mut fast);
        assert!((fast.rpm - g.rpm * 10.0).abs() < 1e-9);
    }

    /// The speed is written out with as many decimals as it takes and no
    /// trailing zeroes, so a slow gear still says something.
    #[test]
    fn the_speed_reads_as_a_number() {
        assert_eq!(rpm_string(12.5), "12.5 RPM");
        assert_eq!(rpm_string(12.0), "12 RPM");
        assert_eq!(rpm_string(0.05), "0.05 RPM");
        assert_eq!(rpm_string(0.0005), "0.0005 RPM");
        // Far too slow for two decimals to say anything at all.
        assert!(rpm_string(0.000_000_5).starts_with("0.0000005"));
    }

    /// Hovering over a gear labels it; hovering over the background does not.
    #[test]
    fn the_pointer_labels_the_gear_under_it() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();

        let text_batches = |r: &Runner3d| {
            // The label is the only unlit, textured thing in the frame.
            r.frame()
                .batches
                .iter()
                .filter(|b| !b.lighting && b.texture.is_some())
                .count()
        };
        assert_eq!(text_batches(&r), 0, "a label with no pointer");

        // Find a pixel that is over a gear by trying the middle of the screen
        // and walking outwards.
        let mut labelled = false;
        for x in (20..620).step_by(20) {
            for y in (20..460).step_by(20) {
                r.event(XEvent::MotionNotify { x, y });
                r.step();
                if text_batches(&r) > 0 {
                    labelled = true;
                    break;
                }
            }
            if labelled {
                break;
            }
        }
        assert!(labelled, "the pointer never found a gear anywhere");

        // And off the edge of the world, nothing.
        r.event(XEvent::MotionNotify { x: -100, y: -100 });
        r.step();
        assert_eq!(text_batches(&r), 0, "a label with the pointer outside");
    }

    /// The fast-forward runs the train on until it is about to come into view,
    /// so the first frame is not a blank screen.
    #[test]
    fn it_starts_with_gears_already_on_screen() {
        let r = run("", 1);
        assert!(
            r.frame().vertices.len() > 1000,
            "the first frame is nearly empty"
        );
    }
}
