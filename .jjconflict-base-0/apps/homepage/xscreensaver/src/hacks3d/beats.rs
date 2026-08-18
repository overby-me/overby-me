//! Port of `hacks/glx/beats.c`.
//!
//! ```text
//! beats, Copyright (c) 2020 David Eccles (gringer) <hacking@gringene.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software
//! and its documentation for any purpose is hereby granted without
//! fee, provided that the above copyright notice appear in all copies
//! and that both that copyright notice and this permission notice
//! appear in supporting documentation.  No representations are made
//! about the suitability of this software for any purpose.  It is
//! provided "as is" without express or implied warranty.
//!
//! Beats changes the position of objects in time with a
//! synchronisation signal (or more correctly, based on the time
//! elapsed since the last synchronisation point). By default, the
//! system clock is used for this signal, with synchronisation
//! happening every minute. The location of objects is entirely
//! dependant on this synchronisation signal; there is no multi-object
//! state that needs to be stored, although there may be some styling
//! state required.
//! ```
//!
//! It is a clock, though it never says so. Every ball's position is a pure
//! function of the time of day: nothing is integrated, nothing is remembered,
//! and the whole scene could be reconstructed from a wristwatch. The balls
//! come round together once a minute, which is the synchronisation the comment
//! means, and the colours turn once an hour and once a half-day.
//!
//! Four arrangements, chosen by a hash of the current minute and hour, so it
//! changes on its own and always at a round number of minutes: a clockwise
//! sweep, a rain dance, a metronome and a galaxy.
//!
//! The motion blur is done the honest way and is most of the cost: the whole
//! scene is drawn twenty-one times, each from the clock as it was ten more
//! milliseconds ago, fading out. Since position is a function of time alone,
//! that is simply the same code with a different argument.
//!
//! One thing cannot be faithful. Upstream's cycle hash includes the day of the
//! year, and the host here supplies the time of day but not the date, so that
//! term is zero. Which of the four arrangements is showing at ten past three
//! therefore differs from upstream's; that it changes every minute or so, and
//! is the same for everyone watching at the same moment, does not.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::hsv_to_rgb;
use crate::runtime::gl::Blend;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, SelectItem, StartArgs, unit_sphere,
};

/// How densely to render spheres.
const SPHERE_SLICES: i32 = 16;
const SPHERE_STACKS: i32 = 16;

/// Milliseconds back per blur frame, and how many of them.
const BLUR_OFFSET: i32 = 10;
const FRAMES_PER_BLUR: i32 = 20;

struct Beats {
    beats_list: u32,
    ball_count: usize,
    /// Which arrangement to show, or `None` to let the clock choose.
    preset_cycle: Option<usize>,
    use_tick: bool,
    use_blur: bool,
}

/// `getFracColour`: top red, right yellow, bottom dark green, left blue.
///
/// The arithmetic is in tenths of a degree because upstream wanted it in fixed
/// point, and the four quadrants are stretched unevenly on purpose: the hue
/// covers 0 to 60 in the first quarter turn and 240 to 360 in the last, which
/// is what puts red at the top and keeps green dark at the bottom.
fn frac_colour(pos_frac: f32, s: f64) -> [f32; 4] {
    let theta = (((pos_frac * 3600.0) as i32 % 3600) + 3600) % 3600;
    let (h, v) = if theta < 900 {
        ((theta * 600) / 900, 100)
    } else if theta < 1800 {
        (
            ((theta - 900) * 600) / 900 + 600,
            100 - ((theta - 900) / 18),
        )
    } else if theta < 2700 {
        (
            ((theta - 1800) * 1200) / 900 + 1200,
            ((theta - 1800) / 18) + 50,
        )
    } else {
        (((theta - 2700) * 1200) / 900 + 2400, 100)
    };
    let (r, g, b) = hsv_to_rgb(h / 10, s, f64::from(v) / 100.0);
    [
        f32::from(r) / 65535.0,
        f32::from(g) / 65535.0,
        f32::from(b) / 65535.0,
        1.0,
    ]
}

impl Hack3d for Beats {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.blend(Blend::Alpha);

        let num_objects = self.ball_count.max(2);
        let delta_limit = if self.use_blur {
            BLUR_OFFSET * FRAMES_PER_BLUR
        } else {
            1
        };

        g.glx.push_matrix();
        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        let now = g.wall_clock();
        let mut time_delta = 0;
        while time_delta <= delta_limit {
            let ball_alpha = if time_delta < BLUR_OFFSET {
                1.0
            } else {
                1.0 / FRAMES_PER_BLUR as f32
            };
            let blur_frac = ((1.0 - time_delta as f32 / delta_limit as f32)
                * std::f32::consts::FRAC_PI_2)
                .sin()
                * ball_alpha;

            // The clock as it was `time_delta` milliseconds ago. Everything
            // below is a function of this and nothing else.
            let then = now - f64::from(time_delta) / 1000.0;
            let then = then.rem_euclid(24.0 * 60.0 * 60.0);
            let tm_h = (then / 3600.0) as i32;
            let tm_m = ((then / 60.0) as i32) % 60;
            let tm_s = (then as i32) % 60;
            let mut sec_frac = (then - then.floor()) as f32;

            // Upstream's hash also folds in the day of the year, which the
            // host does not supply; see the note at the top.
            let time_seed = (((tm_m + 1) as i64)
                * ((tm_m + 1) as i64)
                * (((tm_h + 1) * 37) as i64)
                * 1151
                * 1_233_599)
                .rem_euclid(653);
            let cycle = self.preset_cycle.unwrap_or((time_seed % 4) as usize);

            if self.use_tick && (cycle == 0 || cycle == 3) && time_seed % 2 == 0 {
                /* sine-wave 'tick' motion, converts linear 0..1 to
                pause/fast/pause 0..1 */
                sec_frac = (1.0 - ((0.5 - sec_frac) * std::f32::consts::PI).sin()) / 2.0;
            }

            let min_frac = tm_s as f32 / 60.0;
            /* the position in the synchronisation cycle of one minute */
            let min_prop = (min_frac - min_frac.trunc()) + (sec_frac / 60.0);
            let m2m = min_prop * 2.0 * std::f32::consts::PI;

            /* change colour based on the minute and hour */
            let hour_prop = tm_m as f32 / 60.0 + min_prop / 60.0;
            let hour_prop = hour_prop - hour_prop.trunc();
            let half_day_prop = tm_h as f32 / 12.0 + hour_prop / 12.0;
            let half_day_prop = half_day_prop - half_day_prop.trunc();

            for oi in 0..num_objects {
                g.glx.push_matrix();
                g.glx.scale(1.1, 1.1, 1.1);

                /* Object Fraction Position - 0..1 depending on native Z order */
                let o_fp = oi as f32 / (num_objects - 1) as f32;
                /* set Z distance between [-3.5 .. 0.5] (common to all cycles) */
                let z = o_fp * 4.0 - 3.5;

                // A third of the balls take the second hand's colour, a third
                // the hour's and a third the half-day's, so the group is read
                // as three nested clocks.
                let mut bcolor = if o_fp < 1.0 / 3.0 {
                    frac_colour(min_prop, 1.0)
                } else if o_fp < 2.0 / 3.0 {
                    frac_colour(hour_prop, 1.0)
                } else {
                    frac_colour(half_day_prop, 1.0)
                };

                let oi = oi as f32;
                match cycle {
                    0 => {
                        /* clockwise */
                        g.glx.rotate(-min_prop * 360.0 * (oi + 1.0), 0.0, 0.0, 1.0);
                        g.glx.translate(0.0, 5.0, 0.0);
                    }
                    1 => {
                        /* rain dance */
                        let y = 10.0 * (m2m * (oi + 1.0)).cos() / 2.0;
                        g.glx.translate(0.0, 0.0, -20.0);
                        g.glx.rotate(min_prop * 360.0, 0.0, 1.0, 0.0);
                        g.glx.translate(0.0, y, 20.0);
                    }
                    2 => {
                        /* metronome */
                        let theta = (-m2m * (oi + 1.0)).sin() * 90.0;
                        g.glx.translate(0.0, -5.0, 0.0);
                        g.glx.rotate(theta, 0.0, 0.0, 1.0);
                        g.glx.translate(0.0, 10.0, 0.0);
                    }
                    _ => {
                        /* galaxy */
                        let mp = (num_objects - 1) as f32 / 2.0;
                        let op = mp - oi;
                        let dist = (op.abs() + 0.5) as i32;
                        // Each ball travels a whole number of loops per cycle,
                        // so the whole galaxy comes back together every minute.
                        let path_length = ((60.0 / dist.max(1) as f32) + 0.5) as i32 as f32 * 720.0;
                        let delta = path_length / 2.0;
                        let theta = -min_prop * delta - 180.0;
                        g.glx.translate(0.0, 0.0, -20.0);
                        g.glx.rotate(min_prop * 360.0 - 180.0, 1.0, 0.0, 0.0);
                        g.glx.translate(0.0, 0.0, 20.0);
                        g.glx.translate(0.0, -5.0, 0.0);
                        g.glx.translate(0.0, 0.0, -20.0);
                        g.glx.rotate(theta, 0.0, 1.0, 0.0);
                        g.glx.translate(0.0, 0.0, 20.0);
                    }
                }

                /* spread out based on Z position */
                g.glx.translate(0.0, 0.0, (z - 0.5) * 10.0);

                g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
                g.glx.material_shininess(92.0);
                if self.use_blur {
                    bcolor[3] = if time_delta == 0 { 1.0 } else { blur_frac };
                }
                g.glx.material_ambient_diffuse(bcolor);
                g.glx.call_list(self.beats_list);
                g.glx.pop_matrix();
            }
            time_delta += BLUR_OFFSET;
        }
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }

    fn reshape(&mut self, g: &mut Gl, width: i32, mut height: i32) {
        let mut h = height as f32 / width.max(1) as f32;
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
        g.glx.perspective(30.0, 1.0 / h, 1.0, 100.0);

        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx.clear();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let cycle = g.res.int("cycle");
    let mut st = Beats {
        beats_list: 0,
        ball_count: g.res.int("count").clamp(2, 100) as usize,
        preset_cycle: (0..=3).contains(&cycle).then_some(cycle as usize),
        use_tick: g.res.bool("tick"),
        use_blur: g.res.bool("blur"),
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
    g.glx.light_ambient(0, [0.02, 0.02, 0.02, 1.0]);
    g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
    g.glx.light_specular(0, [0.2, 0.2, 0.2, 0.2]);

    st.beats_list = g.glx.gen_lists(1);
    g.glx.new_list(st.beats_list);
    g.glx.scale(0.71, 0.71, 0.71);
    unit_sphere(&mut g.glx, SPHERE_STACKS, SPHERE_SLICES, wire);
    g.glx.end_list();

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*count:        30",
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*cycle:        -1",
    "*tick:         True",
    "*blur:         True",
];

/// The four arrangements, or the clock's own choice.
const CYCLES: &[SelectItem] = &[
    SelectItem {
        value: "-1",
        label: "Random cycle style",
    },
    SelectItem {
        value: "0",
        label: "Clockwise cycle",
    },
    SelectItem {
        value: "1",
        label: "Rain dance cycle",
    },
    SelectItem {
        value: "2",
        label: "Metronome cycle",
    },
    SelectItem {
        value: "3",
        label: "Galaxy cycle",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("count", "Number of balls", 2.0, 100.0, 1.0, 0, "30"),
    Opt::select("cycle", "Cycle style", CYCLES, "-1"),
    Opt::boolean("tick", "Tick", "true"),
    Opt::boolean("blur", "Motion Blur", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "beats",
    label: "Beats",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "David Eccles",
        year: "2020",
        video: Some("https://www.youtube.com/watch?v=u7N5l0LXryg"),
        blurb: "Balls moving in time with the clock, once round every minute.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner3d {
    Runner3d::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver3d = Saver3d { def: &DEF, start };
