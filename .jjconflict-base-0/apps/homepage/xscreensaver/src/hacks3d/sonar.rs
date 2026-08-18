/* sonar, Copyright © 1998-2026 Jamie Zawinski and Stephen Martin
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/sonar.c` and `hacks/glx/sonar-sim.c`.
//!
//! A sonar scope: a curved glass dish with a range grid on it, a sweep arm
//! going round once every thirty seconds, and blips that light up as the arm
//! passes over them and fade out behind it.
//!
//! Upstream reads its blips from a *sensor*, of which it has two: an ICMP
//! pinger that plots the hosts on your network by response time, and a
//! simulation that makes two teams of them up. Only the second one can exist
//! here, since a browser cannot open a raw socket, so the ping options are
//! kept and all of them land on the same fallback upstream uses when it is not
//! setuid: the error is shown for six seconds and the simulation runs instead.
//! That is upstream's own failure path rather than something invented for the
//! port, which is why the message reads the way it does.
//!
//! The one thing done differently is the sweep's gradient. It is a quad strip
//! whose alpha falls off along its length, and upstream sets it by calling
//! `glMaterialfv` between columns. Material is batch state here, so that would
//! be a draw call per column: forty rings by forty-four columns is 1,760 of
//! them for the sweep alone. Enabling colour-material makes the same value
//! per-vertex data, which is what it wants to be, and the sweep is one call.
//! `GL_COLOR_MATERIAL` tracks `GL_AMBIENT_AND_DIFFUSE` by default, so this is
//! the same property upstream was setting.

use crate::runtime::gl::{Blend, Shape};
use crate::runtime::texfont::TexFont;
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, Saver3d, SaverDef, SelectItem, StartArgs};
#[cfg(target_arch = "wasm32")]
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, SelectItem, StartArgs};
use crate::runtime::{Rotator, Trackball, XEvent, frand, random};

use std::f64::consts::PI;

const TAU: f64 = PI * 2.0;

/// How finely the dish is divided. Upstream's comment on the first: "must be a
/// multiple of th_skip2 divisor".
const TH_STEPS: usize = 36 * 4;
const R_STEPS: usize = 40;

/// How much the dish curves up towards its rim.
const CURVATURE: f64 = PI * 0.4;

/// One blip on the scope.
///
/// Upstream keeps these on three singly-linked lists and moves them between
/// them by name; here they are three vectors, and `insert_by_name` is what
/// `copy_and_insert_bogie` does.
#[derive(Clone)]
struct Bogie {
    /// The host name, or the simulated one.
    name: String,
    /// The second line of the label: upstream's ping time.
    desc: Option<String>,
    /// Distance from the middle, 0 to 1.
    r: f64,
    /// Heading, 0 to 2 pi.
    th: f64,
    /// Time to live, 0 to 2 pi: how much of a sweep it has left before it has
    /// faded out.
    ttl: f64,
}

impl Bogie {
    /// `sonar_copy_bogie`, which takes the opportunity to normalise `th` into
    /// [0, 2pi).
    fn normalized(mut self) -> Bogie {
        self.th = self.th.rem_euclid(TAU);
        self
    }
}

/// The simulation sensor: `hacks/glx/sonar-sim.c`.
///
/// Two teams of blips that wander about. There is no second sensor here; see
/// the note at the top of the file.
struct Sim {
    targets: Vec<Bogie>,
}

impl Sim {
    /// `make_bogies`. Note that the `j` loop runs B first: the count and the
    /// name are both chosen by `j`, so they stay together.
    fn new(a_name: &str, b_name: &str, a_count: i32, b_count: i32) -> Sim {
        let mut targets = Vec::new();
        for j in 0..=1 {
            let (name, count) = if j == 1 {
                (a_name, a_count)
            } else {
                (b_name, b_count)
            };
            for i in 0..count {
                targets.insert(
                    0,
                    Bogie {
                        name: format!("{name}{:03}", i + 1),
                        desc: None,
                        r: 0.3 + frand(0.5),
                        th: frand(TAU),
                        ttl: 0.0,
                    },
                );
            }
        }
        Sim { targets }
    }

    /// `sim_scan`: an updated (moved) copy of the blips.
    fn scan(&mut self) -> Vec<Bogie> {
        let scale = 0.01;
        let mut list = Vec::with_capacity(self.targets.len());
        for b in &mut self.targets {
            b.r += scale * (0.5 - frand(1.0));
            b.th += scale * (0.5 - frand(1.0));
            while b.r < 0.2 {
                b.r += scale * 0.1;
            }
            while b.r > 0.9 {
                b.r -= scale * 0.1;
            }
            list.insert(0, b.clone().normalized());
        }
        list
    }
}

/// `copy_and_insert_bogie`: add it, replacing any blip already there under the
/// same name.
fn insert_by_name(list: &mut Vec<Bogie>, b: Bogie) {
    if let Some(i) = list.iter().position(|ob| ob.name == b.name) {
        list.remove(i);
    }
    list.insert(0, b.normalized());
}

/// Whether an angle lies between two others.
///
/// Upstream: "When those angles cross 0, it assumes the wedge is the smaller
/// one. That is: 5 lies between 10 and 350 degrees (a 20 degree wedge)."
fn point_in_wedge(th: f64, low: f64, high: f64) -> bool {
    if low < high {
        th > low && th <= high
    } else {
        th <= high || th > low
    }
}

/// Which of the three things the saver does on a given frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Get the startup message on screen before doing anything slow.
    Msg,
    /// Upstream's `gethostbyaddr` pause, which is where the sensor is made.
    Resolve,
    /// Sweep.
    Run,
}

struct SonarState {
    trackball: Trackball,
    rot: Rotator,
    sweep_offset: f64,
    sweep_th: f64,
    line_thickness: f32,
    font: TexFont,

    phase: Phase,
    sim: Option<Sim>,
    /// Set when the ping sensor cannot be had, and shown for six seconds.
    error: Option<String>,
    /// Cleared when the error is, so the two stay on the same clock.
    error_started: f64,
    /// The second line upstream puts in the corner: which subnet it is
    /// pinging. There is none to report here.
    desc: Option<String>,

    /// On screen and fading.
    displayed: Vec<Bogie>,
    /// Returned by the sensor, not yet swept over.
    pending: Vec<Bogie>,

    speed: f64,
    sweep_size: f64,
    font_size: f64,
    wobble: bool,
    wire: bool,
    width: i32,
    height: i32,
}

impl SonarState {
    /// `draw_screen`: the dish, as glass, as a grid of lines, or as the sweep.
    ///
    /// One function for three quite different things, exactly as upstream: the
    /// three display lists it compiles at startup are three calls to this.
    fn draw_screen(&self, g: &mut Gl, mesh_p: bool, sweep_p: bool) {
        let wire = self.wire;
        if wire && !(mesh_p || sweep_p) {
            return;
        }

        const GLASS: [f32; 4] = [0.0, 0.4, 0.0, 0.5];
        const LINES: [f32; 4] = [0.0, 0.7, 0.0, 0.5];
        const SWEEPC: [f32; 4] = [0.2, 1.0, 0.2, 0.5];

        g.glx.texturing(false);
        g.glx.front_face_cw(false);

        let (mut r_skip, mut th_skip, mut th_skip2, mut outer_r) = (1, 1, 1, 0);

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(20.0);
        g.glx
            .material_ambient_diffuse(if mesh_p { LINES } else { GLASS });
        if wire {
            g.glx.color3f(LINES[0], LINES[1], LINES[2]);
        }

        if mesh_p {
            th_skip = TH_STEPS / 12;
            th_skip2 = TH_STEPS / 36;
            r_skip = R_STEPS / 3;
            outer_r = (R_STEPS as f64 * 0.93) as usize;
            if !wire {
                g.glx.line_width(self.line_thickness);
            }
        }

        let ring: Vec<(f64, f64)> = (0..TH_STEPS)
            .map(|i| {
                let a = TAU * i as f64 / TH_STEPS as f64;
                (a.cos(), a.sin())
            })
            .collect();

        /* place the bottom of the disc on the xy plane. */
        let zoff = (CURVATURE / 2.0 * (PI / 2.0)).cos() / 2.0;

        // The sweep's alpha varies along its length, which upstream sets as a
        // material between columns. Here it is a vertex colour; see the note
        // at the top of the file.
        if sweep_p {
            g.glx.color_material(true);
        }

        let (mut r0, mut z0) = (0.0, 0.0);
        for i in (1..=R_STEPS).rev() {
            r0 = i as f64 / R_STEPS as f64;
            let r1 = ((i + 1) as f64 / R_STEPS as f64).min(1.0); /* avoid asin lossage */

            z0 = (CURVATURE / 2.0 * r0.asin()).cos() / 2.0 - zoff;
            let z1 = (CURVATURE / 2.0 * r1.asin()).cos() / 2.0 - zoff;

            g.glx.begin(if wire || mesh_p {
                Shape::Lines
            } else {
                Shape::QuadStrip
            });
            for j0 in 0..=TH_STEPS {
                if mesh_p
                    && (if i < outer_r {
                        !j0.is_multiple_of(th_skip)
                    } else {
                        !j0.is_multiple_of(th_skip2)
                    })
                {
                    continue;
                }

                if sweep_p {
                    let r = 1.0 - (j0 as f64 / (TH_STEPS as f64 * self.sweep_size));
                    g.glx
                        .color4f(SWEEPC[0], SWEEPC[1], SWEEPC[2], r.max(0.0) as f32);
                }

                let j1 = j0 % TH_STEPS;
                let (cx, cy) = ring[j1];
                g.glx
                    .normal3f((r0 * cx) as f32, (r0 * cy) as f32, z0 as f32);
                g.glx
                    .vertex3f((r0 * cx) as f32, (r0 * cy) as f32, z0 as f32);
                g.glx
                    .normal3f((r1 * cx) as f32, (r1 * cy) as f32, z1 as f32);
                g.glx
                    .vertex3f((r1 * cx) as f32, (r1 * cy) as f32, z1 as f32);

                if sweep_p && j0 as f64 >= TH_STEPS as f64 * self.sweep_size {
                    break;
                }
                if sweep_p && wire {
                    break;
                }
            }
            g.glx.end();

            if mesh_p
                && (i == outer_r
                    || i == R_STEPS
                    || (i.is_multiple_of(r_skip) && i < R_STEPS - r_skip))
            {
                g.glx.begin(Shape::LineLoop);
                for &(cx, cy) in &ring {
                    g.glx
                        .normal3f((r0 * cx) as f32, (r0 * cy) as f32, z0 as f32);
                    g.glx
                        .vertex3f((r0 * cx) as f32, (r0 * cy) as f32, z0 as f32);
                }
                g.glx.end();
            }
        }

        if sweep_p {
            g.glx.color_material(false);
        }

        /* one more polygon for the middle */
        if !wire && !sweep_p {
            g.glx.begin(if mesh_p {
                Shape::LineLoop
            } else {
                Shape::Polygon
            });
            for &(cx, cy) in &ring {
                g.glx
                    .normal3f((r0 * cx) as f32, (r0 * cy) as f32, z0 as f32);
                g.glx
                    .vertex3f((r0 * cx) as f32, (r0 * cy) as f32, z0 as f32);
            }
            g.glx.end();
        }
    }

    /// `draw_text`: a label at a polar position, or the startup message.
    ///
    /// A size of zero or less means a blip's label: yellow, faded by how much
    /// time it has left, with the blip's own square dot above it.
    fn draw_text(&self, g: &mut Gl, string: &str, r: f64, th: f64, ttl: f64, size: f64) {
        let wire = self.wire;
        let mut font_scale = 0.001 * (if size > 0.0 { size } else { self.font_size }) / 14.0;
        if self.width > 2560 {
            font_scale /= 2.0; /* Retina displays */
        }

        if size <= 0.0 {
            /* if size not specified, draw in yellow with alpha */
            let a = ((ttl / TAU) * 1.2).min(1.0) as f32;
            g.glx.material_ambient_diffuse([1.0, 1.0, 0.0, a]);
            if wire {
                g.glx.color3f(a, a, 0.0);
            }
        }

        let lines: Vec<&str> = string
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .collect();
        let mut max_w = 0;
        let mut lh = 0;
        for line in &lines {
            let m = self.font.metrics(line);
            lh = m.ascent + m.descent;
            max_w = max_w.max(m.width);
        }

        g.glx.push_matrix();
        g.glx
            .translate((r * th.cos()) as f32, (r * th.sin()) as f32, 0.0);
        g.glx
            .scale(font_scale as f32, font_scale as f32, font_scale as f32);

        if size <= 0.0 {
            /* Draw the dot */
            let s = (self.font_size * 1.7) as f32;
            g.glx.texturing(false);
            g.glx.front_face_cw(true);
            g.glx
                .begin(if wire { Shape::LineLoop } else { Shape::Quads });
            g.glx.vertex3f(0.0, s, 0.0);
            g.glx.vertex3f(s, s, 0.0);
            g.glx.vertex3f(s, 0.0, 0.0);
            g.glx.vertex3f(0.0, 0.0, 0.0);
            g.glx.end();
            g.glx.translate(-max_w as f32 / 2.0, -lh as f32, 0.0);
        } else {
            g.glx
                .translate(-max_w as f32 / 2.0, -(lh as f32) / 2.0, 0.0);
        }

        /* draw each line, centered */
        if !wire {
            g.glx.texturing(true);
        }
        for (n, line) in lines.iter().enumerate() {
            let w = self.font.metrics(line).width;
            g.glx.push_matrix();
            /* 'polys' stops Z-fighting. */
            g.glx
                .translate((max_w - w) as f32 / 2.0, 0.0, (n * 4) as f32);
            if wire {
                g.glx.begin(Shape::LineLoop);
                g.glx.vertex3f(0.0, 0.0, 0.0);
                g.glx.vertex3f(w as f32, 0.0, 0.0);
                g.glx.vertex3f(w as f32, lh as f32, 0.0);
                g.glx.vertex3f(0.0, lh as f32, 0.0);
                g.glx.end();
            } else {
                g.glx.front_face_cw(true);
                self.font.print_string(&mut g.glx, line);
            }
            g.glx.pop_matrix();
            g.glx.translate(0.0, -(lh as f32), 0.0);
        }
        g.glx.pop_matrix();

        if !wire {
            g.glx.depth_test(true);
            g.glx.lighting(true);
            g.glx.blend(Blend::Alpha);
        }
    }

    /// `draw_table`: "There's a disc with a hole in it around the screen, to
    /// act as a mask preventing slightly off-screen bogies from showing up.
    /// This clips 'em."
    fn draw_table(&self, g: &mut Gl) {
        if self.wire {
            return;
        }
        g.glx.texturing(false);
        g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
        g.glx.material_shininess(0.0);
        g.glx.material_ambient_diffuse([0.0, 0.0, 0.0, 1.0]);
        g.glx.front_face_cw(false);
        g.glx.begin(Shape::QuadStrip);
        g.glx.normal3f(0.0, 0.0, 1.0);
        for i in 0..=TH_STEPS {
            let a = TAU * i as f64 / TH_STEPS as f64;
            let (x, y) = (a.cos() as f32, a.sin() as f32);
            g.glx.vertex3f(x, y, 0.0);
            g.glx.vertex3f(x * 10.0, y * 10.0, 0.0);
        }
        g.glx.end();
    }

    /// `draw_angles`: the bearing numbers round the rim.
    fn draw_angles(&self, g: &mut Gl) {
        g.glx.material_specular([0.0, 0.0, 0.0, 1.0]);
        g.glx.material_ambient_diffuse([0.15, 0.15, 0.15, 1.0]);
        g.glx.translate(0.0, 0.0, 0.01);
        for i in (0..360).step_by(10) {
            let a = PI / 2.0 - (f64::from(i) / 180.0 * PI);
            self.draw_text(g, &i.to_string(), 1.07, a, 0.0, 10.0);
        }
    }

    /// `draw_bogies`.
    fn draw_bogies(&self, g: &mut Gl) {
        for b in &self.displayed {
            let s = match &b.desc {
                Some(d) => format!("{}\n{d}", b.name),
                None => b.name.clone(),
            };
            self.draw_text(g, &s, b.r, b.th, b.ttl, -1.0);

            /* Move *very slightly* forward so that the text is not all in the
            same plane: this prevents flickering with overlapping text as
            the textures fight for priority. */
            g.glx.translate(0.0, 0.0, 0.00002);
        }
    }

    /// `update_sensor_data`: ask the sensor and fold the answer into
    /// `pending`.
    fn update_sensor_data(&mut self) {
        let Some(sim) = &mut self.sim else { return };
        for b in sim.scan() {
            insert_by_name(&mut self.pending, b);
        }
    }

    /// `sweep`: move the arm, light up what it passes over, fade the rest.
    fn sweep(&mut self, now: f64) {
        /* Move the sweep forward (clockwise). */
        let cycle_secs = 30.0 / self.speed; /* one cycle every N seconds */
        let this_sweep =
            (cycle_secs - (now + self.sweep_offset).rem_euclid(cycle_secs)) / cycle_secs * TAU;
        let prev_sweep = self.sweep_th;
        let tick = (prev_sweep - this_sweep).rem_euclid(TAU);

        self.sweep_th = this_sweep;

        if prev_sweep < 0.0 {
            return; /* skip first time */
        }

        /* Go through the 'pending' sensor data, find those bogies who are
        just now being swept, and move them from 'pending' to 'displayed'. */
        let mut hit = Vec::new();
        self.pending.retain(|b| {
            if point_in_wedge(b.th, this_sweep, prev_sweep) {
                let mut b = b.clone();
                b.ttl = PI * 2.1;
                hit.push(b);
                false
            } else {
                true
            }
        });
        for b in hit {
            insert_by_name(&mut self.displayed, b);
        }

        /* Update TTL on all currently-displayed bogies; delete the dead. */
        for b in &mut self.displayed {
            b.ttl -= tick;
        }
        self.displayed.retain(|b| b.ttl > 0.0);

        self.update_sensor_data();
    }

    /// `init_sensor`. There is no ping sensor here, so asking for one gets
    /// upstream's own answer for a machine that cannot ping.
    fn init_sensor(&mut self, g: &mut Gl) {
        let ping = g.res.string("ping").to_string();
        if ping != "simulation" {
            self.error = Some("A browser cannot send ICMP.\nRunning simulation instead.".into());
        }
        self.error_started = g.elapsed();
        self.sim = Some(Sim::new(
            g.res.string("teamAName"),
            g.res.string("teamBName"),
            g.res.int("teamACount").clamp(1, 100),
            g.res.int("teamBCount").clamp(1, 100),
        ));
    }

    /// `draw_startup_blurb`.
    fn draw_startup_blurb(&mut self, g: &mut Gl) {
        let Some(msg) = self.error.clone() else {
            return;
        };
        g.glx.material_ambient_diffuse([0.0, 1.0, 0.0, 1.0]);
        g.glx.translate(0.0, 0.0, 0.3);
        self.draw_text(g, &msg, 0.0, 0.0, 0.0, 30.0);

        /* only leave error message up for N seconds */
        if self.error_started + 6.0 < g.elapsed() {
            self.error = None;
        }
    }
}

impl Hack3d for SonarState {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        let (mut height, mut y) = (height, 0);
        let mut h = f64::from(height) / f64::from(width);

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

        self.line_thickness = if self.wire {
            1.0
        } else {
            (f64::from(height) / 300.0).max(1.0) as f32
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wire;

        g.glx.clear_color(0.0, 0.0, 0.0, 1.0);
        g.glx.clear();

        if !wire {
            g.glx.texturing(true);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.cull_face(true);
            g.glx.depth_test(true);
            g.glx.blend(Blend::Alpha);

            g.glx.light_position(0, 0.05, 0.07, 1.00, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
        }

        g.glx.push_matrix();

        let mut s = 7.0f32;
        if self.width < self.height {
            s *= self.width as f32 / self.height as f32;
        }
        g.glx.scale(s, s, s);

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        if self.wobble {
            let max = 40.0;
            let button_down = self.trackball.button_down();
            let (x, _y, z) = self.rot.position(!button_down);
            g.glx.rotate((max / 2.0 - x * max) as f32, 1.0, 0.0, 0.0);
            g.glx.rotate((max / 2.0 - z * max) as f32, 0.0, 1.0, 0.0);
        }

        g.glx.push_matrix(); /* table */
        self.draw_table(g);
        g.glx.pop_matrix();

        g.glx.push_matrix(); /* text */
        g.glx.translate(0.0, 0.0, -0.01);
        self.draw_bogies(g);
        g.glx.pop_matrix();

        self.draw_screen(g, false, false); /* glass */

        g.glx.translate(0.0, 0.0, 0.004); /* sweep */
        g.glx.push_matrix();
        g.glx
            .rotate((self.sweep_th * 180.0 / PI) as f32, 0.0, 0.0, 1.0);
        if self.sweep_th >= 0.0 {
            self.draw_screen(g, false, true);
        }
        g.glx.pop_matrix();

        g.glx.translate(0.0, 0.0, 0.004); /* grid */
        self.draw_screen(g, true, false);

        g.glx.push_matrix(); /* angles */
        self.draw_angles(g);
        g.glx.pop_matrix();

        if let Some(desc) = self.desc.clone() {
            g.glx.push_matrix();
            g.glx.translate(0.0, 0.0, 0.00002);
            self.draw_text(g, &desc, 1.35, PI * 0.75, 0.0, 10.0);
            g.glx.pop_matrix();
        }

        if self.error.is_some() {
            self.phase = Phase::Msg;
        }

        match self.phase {
            /* Frame 1: get the message on screen. */
            Phase::Msg => {
                self.draw_startup_blurb(g);
                self.phase = Phase::Resolve;
            }
            /* Frame 2: upstream's gethostbyaddr may take a while. */
            Phase::Resolve => {
                if self.sim.is_none() {
                    self.init_sensor(g);
                }
                self.phase = Phase::Run;
            }
            /* Frame N: sweep away. */
            Phase::Run => {
                let now = g.elapsed();
                self.sweep(now);
            }
        }

        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed");
    let font = TexFont::load(&mut g.glx, "monospace bold 48");

    let mut st = SonarState {
        trackball: Trackball::new(),
        rot: Rotator::new(0.0, 0.0, 0.0, 0.0, speed * 0.003, true),
        sweep_offset: f64::from(random() % 60),
        sweep_th: -1.0,
        line_thickness: 1.0,
        font,
        phase: Phase::Msg,
        sim: None,
        error: None,
        error_started: 0.0,
        desc: None,
        displayed: Vec::new(),
        pending: Vec::new(),
        speed,
        sweep_size: g.res.float("sweepSize"),
        font_size: g.res.float("fontSize"),
        wobble: g.res.bool("wobble"),
        wire,
        width: g.width(),
        height: g.height(),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*speed:        1.0",
    "*sweepSize:    0.3",
    "*fontSize:     12",
    "*teamAName:    F18",
    "*teamBName:    MIG",
    "*teamACount:   4",
    "*teamBCount:   4",
    "*ping:         default",
    "*pingTimeout:  3000",
    "*resolve:      True",
    "*times:        True",
    "*wobble:       True",
];

const PING: &[SelectItem] = &[
    SelectItem {
        value: "default",
        label: "Ping local subnet",
    },
    SelectItem {
        value: "simulation",
        label: "Simulation (don't ping)",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.1, 8.0, 0.1, 1, "1.0"),
    Opt::select("ping", "Sensor", PING, "default"),
    Opt::slider("fontSize", "Font size", 6.0, 24.0, 1.0, 0, "12"),
    Opt::slider("sweepSize", "Trail length", 0.02, 0.7, 0.01, 2, "0.3"),
    Opt::spin("teamACount", "A count", 1.0, 100.0, "4"),
    Opt::spin("teamBCount", "B count", 1.0, 100.0, "4"),
    Opt::boolean("wobble", "Tilt", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "sonar",
    label: "Sonar",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski and Stephen Martin",
        year: "1998",
        video: Some("https://www.youtube.com/watch?v=XEL8g3qbthE"),
        blurb: "A sonar scope, sweeping for blips.",
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
        let mut r = start(StartArgs::new(640, 480, query, 20260812));
        for _ in 0..frames {
            r.step();
        }
        r
    }

    /// The wedge predicate, tested against its own definition rather than by
    /// eye: upstream's comment says 5 lies between 10 and 350 degrees, because
    /// when the pair crosses zero the wedge is the smaller one.
    #[test]
    fn the_wedge_is_the_smaller_one() {
        let d = |deg: f64| deg.to_radians();
        assert!(point_in_wedge(d(5.0), d(350.0), d(10.0)));
        assert!(!point_in_wedge(d(180.0), d(350.0), d(10.0)));
        // The ordinary case, not crossing zero.
        assert!(point_in_wedge(d(90.0), d(80.0), d(100.0)));
        assert!(!point_in_wedge(d(70.0), d(80.0), d(100.0)));
        // Half-open at the low end, closed at the high: a blip exactly on the
        // arm is swept once, not twice.
        assert!(!point_in_wedge(d(80.0), d(80.0), d(100.0)));
        assert!(point_in_wedge(d(100.0), d(80.0), d(100.0)));
    }

    /// The simulation makes both teams at the counts asked for, and its blips
    /// stay inside the dish however long they wander.
    #[test]
    fn the_blips_stay_on_the_scope() {
        crate::runtime::rand::ya_rand_init(20260812);
        let mut sim = Sim::new("F18", "MIG", 3, 5);
        assert_eq!(sim.targets.len(), 8);
        assert_eq!(
            sim.targets
                .iter()
                .filter(|b| b.name.starts_with("F18"))
                .count(),
            3
        );
        assert_eq!(
            sim.targets
                .iter()
                .filter(|b| b.name.starts_with("MIG"))
                .count(),
            5
        );

        for _ in 0..20_000 {
            for b in sim.scan() {
                assert!(
                    (0.2..=0.9).contains(&b.r),
                    "{} wandered off the dish at r={}",
                    b.name,
                    b.r
                );
                assert!(
                    (0.0..TAU).contains(&b.th),
                    "{} has heading {}",
                    b.name,
                    b.th
                );
            }
        }
    }

    /// A blip lights up when the arm passes over it and fades out behind it,
    /// which is the whole animation. The arm goes round once every thirty
    /// seconds, so a full cycle is what this has to cover.
    #[test]
    fn the_sweep_lights_blips_and_they_fade() {
        let mut r = start(StartArgs::new(640, 480, "ping=simulation", 20260812));
        let mut peak = 0;
        let mut ever_faded = false;
        let mut prev = 0;
        for _ in 0..2400 {
            r.step();
            let n = r.frame().batches.len();
            peak = peak.max(n);
            if n < prev {
                ever_faded = true;
            }
            prev = n;
        }
        assert!(peak > 0);
        assert!(
            ever_faded,
            "the scope only ever gained blips, so nothing was fading"
        );
    }

    /// Asking for a ping sensor gets upstream's answer for a machine that
    /// cannot ping: the message, and then the simulation anyway.
    #[test]
    fn a_ping_request_falls_back_to_the_simulation() {
        let mut r = start(StartArgs::new(640, 480, "ping=default", 20260812));
        for _ in 0..4 {
            r.step();
        }
        // Six seconds of frames at the default delay, plus a margin.
        for _ in 0..300 {
            r.step();
        }
        let f = r.frame();
        assert!(
            !f.vertices.is_empty(),
            "nothing was drawn after the message"
        );
    }

    /// The scope fits in a frame budget, blips and all.
    #[test]
    fn a_frame_fits_in_the_budget() {
        let r = run("", 600);
        let f = r.frame();
        assert!(!f.vertices.is_empty());
        assert!(
            f.vertices.len() < 200_000,
            "a frame came to {} vertices",
            f.vertices.len()
        );
        assert!(
            f.batches.len() < 400,
            "a frame came to {} batches",
            f.batches.len()
        );
    }
}
