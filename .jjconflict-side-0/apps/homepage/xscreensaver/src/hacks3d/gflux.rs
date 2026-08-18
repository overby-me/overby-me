//! Port of `hacks/glx/gflux.c`.
//!
//! ```text
//! gflux - creates a fluctuating 3D grid
//! requires OpenGL or MesaGL
//!
//! Copyright (c) Josiah Pease, 2000, 2003
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Thanks go to all those who worked on...
//! MesaGL, OpenGL, UtahGLX, XFree86, gcc, vim, rxvt, the PNM (anymap) format
//! xscreensaver and the thousands of other tools, apps and daemons that make
//! linux usable
//! Personal thanks to Kevin Moss, Paul Sheahee and Jamie Zawinski
//! ```
//!
//! A sheet of picture rippling in three dimensions.
//!
//! The height of the sheet at a point is the sum of a handful of ripples, each
//! a sine of the squared distance from its own centre, and each one fading in
//! while the one before it fades out. There is no simulation in it: the surface
//! is a closed-form function of the place and the time, evaluated afresh at
//! every corner of the mesh, which is also how it gets its normals, by asking
//! for the height a little to either side.
//!
//! Upstream tiles the picture across the sheet with a texture matrix, a
//! translate and a halving that leave the coordinates outside the unit square
//! for the wrapping to bring back. There is no texture matrix here, so the
//! same arithmetic is done to the coordinates on the way out. Its vertical
//! scale is negative because its textures are stored bottom up; that is kept,
//! because the numbers land on the same place after wrapping and the picture
//! comes out the right way up either way.
//!
//! Upstream has a knob for flat shading, which this runtime has no equivalent
//! of. It is off by default and the configuration file does not offer it.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Glx, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};

const MAXWAVES: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Draw {
    Wire,
    Solid,
    Light,
    Checker,
    Grab,
}

struct GFlux {
    trackball: Trackball,
    /// Each wave's amplitude, frequency and centre.
    wa: [f64; MAXWAVES],
    freq: [f64; MAXWAVES],
    dispx: [f64; MAXWAVES],
    dispy: [f64; MAXWAVES],
    texture: u32,
    tex_xscale: f32,
    tex_yscale: f32,
    time: f64,
    anglex: f32,
    angley: f32,
    anglez: f32,
    counter: i32,
    new_wave: usize,
    waiting_for_image: bool,
    draw_mode: Draw,
    squares: i32,
    resolution: i32,
    speed: f32,
    rotationx: f32,
    rotationy: f32,
    rotationz: f32,
    zoom: f32,
    waves: usize,
    wave_change: i32,
    wave_height: f32,
    wave_freq: f32,
    scale: f32,
}

impl GFlux {
    /// The height of the sheet at a point: every wave is a sine of the squared
    /// distance from its own centre, so they spread as rings.
    fn get_grid(&self, x: f64, y: f64, a: f64) -> f64 {
        let tmp = 1.0 / self.waves as f64;
        let mut z = 0.0;
        for i in 0..self.waves {
            z += self.wa[i]
                * tmp
                * (self.freq[i]
                    * ((x + self.dispx[i]) * (x + self.dispx[i])
                        + (y + self.dispy[i]) * (y + self.dispy[i])
                        + a))
                    .sin();
        }
        z
    }

    /// Every so often one wave is replaced. It fades in over the same number
    /// of frames that the one before it fades out, so the surface never jumps.
    fn calc_grid(&mut self) {
        let tmp = 1.0 / self.wave_change as f64;
        if self.counter % self.wave_change == 0 {
            self.new_wave = ((self.counter as f64 * tmp) as usize) % self.waves;
            self.dispx[self.new_wave] = -frand(1.0);
            self.dispy[self.new_wave] = -frand(1.0);
            self.freq[self.new_wave] = self.wave_freq as f64 * frand(1.0);
            self.wa[self.new_wave] = 0.0;
        }
        self.counter += 1;
        self.wa[self.new_wave] += tmp;
        self.wa[(self.new_wave + 1) % self.waves] -= tmp;
    }

    /// Upstream's `genColour`: green where the sheet is high, blue where it is
    /// low.
    fn colour(z: f64) -> [f32; 3] {
        [0.0, 0.5 + 0.5 * z as f32, 0.5 - 0.5 * z as f32]
    }

    fn spin(&mut self, turning: bool) {
        if turning {
            self.time -= self.speed as f64;
            self.anglex -= self.rotationx;
            self.angley -= self.rotationy;
            self.anglez -= self.rotationz;
        }
    }

    /// The picture on the sheet, and a border drawn round the edge of it.
    fn display_texture(&mut self, g: &mut Glx) {
        let squares = self.squares as f64;
        let (dx, dy) = (2.0 / squares, 2.0 / squares);
        let (du, dv) = (2.0 / squares, 2.0 / squares);
        let (xs, ys) = (self.tex_xscale as f64, self.tex_yscale as f64);
        let (minx, miny, maxx, maxy) = (-1.0, -1.0, 1.0, 1.0);

        // Upstream's texture matrix, done to the coordinates instead: half
        // them and take one, and let the wrapping bring them back.
        let tc = |g: &mut Glx, u: f64, v: f64| {
            g.tex_coord2f((u * 0.5 - 1.0) as f32, (v * 0.5 - 1.0) as f32);
        };

        g.texturing(true);
        g.bind_texture(self.texture);
        g.color4f(0.5, 0.5, 0.5, 1.0);

        let t = self.time;
        let mut x = minx;
        let mut u = 0.0;
        while x < maxx - 0.01 {
            g.begin(Shape::QuadStrip);
            let mut y = miny;
            let mut v = 0.0;
            while y <= maxy + 0.01 {
                let z = self.get_grid(x, y, t);
                tc(g, u * xs, v * ys);
                g.normal3f(
                    (self.get_grid(x + dx, y, t) - self.get_grid(x - dx, y, t)) as f32,
                    (self.get_grid(x, y + dy, t) - self.get_grid(x, y - dy, t)) as f32,
                    1.0,
                );
                g.vertex3f(x as f32, y as f32, z as f32);

                let z = self.get_grid(x + dx, y, t);
                tc(g, (u + du) * xs, v * ys);
                g.normal3f(
                    (self.get_grid(x + dx + dx, y, t) - self.get_grid(x, y, t)) as f32,
                    (self.get_grid(x + dx, y + dy, t) - self.get_grid(x + dx, y - dy, t)) as f32,
                    1.0,
                );
                g.vertex3f((x + dx) as f32, y as f32, z as f32);
                y += dy;
                v += dv;
            }
            g.end();
            x += dx;
            u += du;
        }

        // A border round the grid.
        g.color4f(0.4, 0.4, 0.4, 1.0);
        g.texturing(false);
        g.begin(Shape::LineLoop);
        let mut x = minx;
        while x <= maxx {
            g.vertex3f(x as f32, miny as f32, self.get_grid(x, miny, t) as f32);
            x += dx;
        }
        let mut y = miny;
        while y <= maxy {
            g.vertex3f(maxx as f32, y as f32, self.get_grid(maxx, y, t) as f32);
            y += dy;
        }
        let mut x = maxx;
        while x >= minx {
            g.vertex3f(x as f32, maxy as f32, self.get_grid(x, maxy, t) as f32);
            x -= dx;
        }
        let mut y = maxy;
        while y >= miny {
            g.vertex3f(minx as f32, y as f32, self.get_grid(minx, y, t) as f32);
            y -= dy;
        }
        g.end();
    }

    /// The sheet in flat colour, or the same with normals so that it is lit.
    fn display_solid(&mut self, g: &mut Glx, lit: bool) {
        let squares = self.squares as f64;
        let (dx, dy) = (2.0 / squares, 2.0 / squares);
        let t = self.time;

        let mut x = -1.0;
        while x < 0.9999 {
            g.begin(Shape::QuadStrip);
            let mut y = -1.0;
            while y <= 1.0 {
                let z = self.get_grid(x, y, t);
                let c = Self::colour(z);
                g.color4f(c[0], c[1], c[2], 1.0);
                if lit {
                    g.normal3f(
                        (self.get_grid(x + dx, y, t) - self.get_grid(x - dx, y, t)) as f32,
                        (self.get_grid(x, y + dy, t) - self.get_grid(x, y - dy, t)) as f32,
                        1.0,
                    );
                }
                g.vertex3f(x as f32, y as f32, z as f32);

                let z = self.get_grid(x + dx, y, t);
                let c = Self::colour(z);
                g.color4f(c[0], c[1], c[2], 1.0);
                if lit {
                    g.normal3f(
                        (self.get_grid(x + dx + dx, y, t) - self.get_grid(x, y, t)) as f32,
                        (self.get_grid(x + dx, y + dy, t) - self.get_grid(x + dx, y - dy, t))
                            as f32,
                        1.0,
                    );
                }
                g.vertex3f((x + dx) as f32, y as f32, z as f32);
                y += dy;
            }
            g.end();
            x += dx;
        }
    }

    /// The sheet as a wire mesh: lines along one axis at the mesh spacing,
    /// each drawn at the finer resolution so it curves.
    fn display_wire(&mut self, g: &mut Glx) {
        let squares = self.squares as f64;
        let res = self.resolution as f64;
        let d1 = 2.0 / (squares * res) - 0.00001;
        let d2 = 2.0 / squares - 0.00001;
        let t = self.time;

        let mut x = -1.0;
        while x <= 1.0 {
            g.begin(Shape::LineStrip);
            let mut y = -1.0;
            while y <= 1.0 {
                let z = self.get_grid(x, y, t);
                let c = Self::colour(z);
                g.color4f(c[0], c[1], c[2], 1.0);
                g.vertex3f(x as f32, y as f32, z as f32);
                y += d1;
            }
            g.end();
            x += d2;
        }
        let mut y = -1.0;
        while y <= 1.0 {
            g.begin(Shape::LineStrip);
            let mut x = -1.0;
            while x <= 1.0 {
                let z = self.get_grid(x, y, t);
                let c = Self::colour(z);
                g.color4f(c[0], c[1], c[2], 1.0);
                g.vertex3f(x as f32, y as f32, z as f32);
                x += d1;
            }
            g.end();
            y += d2;
        }
    }
}

/// The checkerboard the `checker` mode puts on the sheet, which upstream
/// writes out as four by four pixels of two greys.
fn checker_texture(g: &mut Glx) -> u32 {
    let id = g.gen_texture();
    g.bind_texture(id);
    let mut px = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4u32 {
        for x in 0..4u32 {
            let v: u8 = if (x + y).is_multiple_of(2) {
                0xFF
            } else {
                0xAA
            };
            px.extend_from_slice(&[v, v, v, 255]);
        }
    }
    g.tex_image_2d(4, 4, px);
    g.tex_nearest(true);
    g.tex_clamp(false);
    id
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let mode = g.res.string("mode").to_ascii_lowercase();
    let mut draw_mode = match mode.as_str() {
        "wire" => Draw::Wire,
        "solid" => Draw::Solid,
        "light" => Draw::Light,
        "checker" => Draw::Checker,
        _ => Draw::Grab,
    };
    if wire {
        draw_mode = Draw::Wire;
    }

    let texture = match draw_mode {
        Draw::Checker => checker_texture(&mut g.glx),
        Draw::Grab => g.glx.gen_texture(),
        _ => 0,
    };

    let mut this = GFlux {
        trackball: Trackball::new(),
        wa: [0.0; MAXWAVES],
        freq: [0.0; MAXWAVES],
        dispx: [0.0; MAXWAVES],
        dispy: [0.0; MAXWAVES],
        texture,
        tex_xscale: if draw_mode == Draw::Checker { 4.0 } else { 1.0 },
        tex_yscale: if draw_mode == Draw::Checker { 4.0 } else { 1.0 },
        // Two of these never start in lockstep.
        time: frand(1000.0),
        anglex: 0.0,
        angley: 0.0,
        anglez: 0.0,
        counter: 0,
        new_wave: 0,
        waiting_for_image: draw_mode == Draw::Grab,
        draw_mode,
        squares: g.res.int("squares").max(2),
        resolution: g.res.int("resolution").max(1),
        speed: g.res.float("speed") as f32,
        rotationx: g.res.float("rotationx") as f32,
        rotationy: g.res.float("rotationy") as f32,
        rotationz: g.res.float("rotationz") as f32,
        zoom: g.res.float("zoom") as f32,
        waves: g.res.int("waves").clamp(1, MAXWAVES as i32) as usize,
        wave_change: g.res.int("waveChange").max(1),
        wave_height: g.res.float("waveHeight") as f32,
        wave_freq: g.res.float("waveFreq") as f32,
        scale: 1.0,
    };
    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for GFlux {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.scale = if width < height {
            width as f32 / height as f32
        } else {
            1.0
        };
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if crate::runtime::screenhack_event_helper(event) && self.draw_mode == Draw::Grab {
            // A new picture, on demand.
            self.waiting_for_image = true;
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.waiting_for_image {
            // Upstream keeps running while the picture is on its way.
            if let Some(img) = g.load_image(512, 512) {
                g.glx.bind_texture(self.texture);
                g.glx.tex_image_2d(img.width, img.height, img.pixels);
                g.glx.tex_clamp(false);
                self.waiting_for_image = false;
            }
        }

        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        let z = self.zoom;
        g.glx.frustum(-z, z, -0.8 * z, 0.8 * z, 2.0, 6.0);
        g.glx.translate(0.0, 0.0, -4.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx.scale(self.scale, self.scale, self.scale);

        g.glx.clear();
        g.glx.cull_face(false);
        g.glx.depth_test(self.draw_mode != Draw::Wire);
        let lit = matches!(self.draw_mode, Draw::Light | Draw::Checker | Draw::Grab);
        g.glx.lighting(lit);
        if lit {
            g.glx.light_enable(0, true);
            g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.light_position(0, 5.0, 5.0, 15.0, 1.0);
            g.glx.material_specular([0.5, 0.5, 0.5, 1.0]);
            g.glx.material_shininess(30.0);
            // The colour is the material: upstream turns on colour tracking
            // and then only ever sets colours.
            g.glx.color_material(true);
        }

        let turning = !self.trackball.button_down();

        // The texture mode turns the sheet before the user's own rotation and
        // the others turn it after, which is upstream's, not a slip.
        let m = self.trackball.matrix();
        if self.draw_mode == Draw::Grab || self.draw_mode == Draw::Checker {
            g.glx.mult_matrix(m);
            g.glx.rotate(self.anglex, 1.0, 0.0, 0.0);
            g.glx.rotate(self.angley, 0.0, 1.0, 0.0);
            g.glx.rotate(self.anglez, 0.0, 0.0, 1.0);
        } else {
            g.glx.rotate(self.anglex, 1.0, 0.0, 0.0);
            g.glx.rotate(self.angley, 0.0, 1.0, 0.0);
            g.glx.rotate(self.anglez, 0.0, 0.0, 1.0);
            g.glx.mult_matrix(m);
        }
        g.glx.scale(1.0, 1.0, self.wave_height);

        self.calc_grid_if(turning);
        let glx = &mut g.glx;
        match self.draw_mode {
            Draw::Grab | Draw::Checker => self.display_texture(glx),
            Draw::Light => self.display_solid(glx, true),
            Draw::Solid => self.display_solid(glx, false),
            Draw::Wire => self.display_wire(glx),
        }
        self.spin(turning);

        g.glx.texturing(false);
        g.res.int("delay") as u32
    }
}

impl GFlux {
    fn calc_grid_if(&mut self, turning: bool) {
        if turning {
            self.calc_grid();
        }
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       20000",
    "*showFPS:     False",
    "*wireframe:   False",
    "*mode:        grab",
    "*squares:     19",
    "*resolution:  4",
    "*flat:        0",
    "*speed:       0.05",
    "*rotationx:   0.01",
    "*rotationy:   0.0",
    "*rotationz:   0.1",
    "*waves:       3",
    "*waveChange:  50",
    "*waveHeight:  1.0",
    "*waveFreq:    3.0",
    "*zoom:        1.0",
];

const MODES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "grab",
        label: "Picture",
    },
    crate::runtime::opts::SelectItem {
        value: "wire",
        label: "Wire mesh",
    },
    crate::runtime::opts::SelectItem {
        value: "solid",
        label: "Flat lighting",
    },
    crate::runtime::opts::SelectItem {
        value: "light",
        label: "Directional lighting",
    },
    crate::runtime::opts::SelectItem {
        value: "checker",
        label: "Checkerboard",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("speed", "Wave speed", 0.0, 0.5, 0.005, 3, "0.05").inverted(),
    Opt::slider("squares", "Mesh density", 2.0, 40.0, 1.0, 0, "19"),
    Opt::slider("waves", "Waves", 1.0, 10.0, 1.0, 0, "3"),
    Opt::select("mode", "Mode", MODES, "grab"),
    Opt::slider("waveHeight", "Wave height", 0.0, 3.0, 0.1, 1, "1.0"),
    Opt::slider("zoom", "Zoom", 0.5, 3.0, 0.1, 1, "1.0"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "gflux",
    label: "GFlux",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Josiah Pease",
        year: "2000",
        video: Some("https://www.youtube.com/watch?v=vbRFlKH-LpA"),
        blurb: "A fluctuating 3D grid, with a picture stretched over it.",
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

    /// The sheet is a closed-form function of place and time: the same point
    /// at the same moment is the same height, and it stays within the range
    /// the waves can add up to.
    #[test]
    fn the_sheet_is_a_function_of_where_and_when() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let zs: Vec<f32> = f.vertices.iter().map(|v| v.pos[2]).collect();
        assert!(!zs.is_empty(), "the sheet was not drawn");
        assert!(
            zs.iter().all(|z| z.abs() <= 1.0),
            "the sheet went outside its range"
        );
    }

    /// Waves fade in as the one before them fades out, so the total is always
    /// bounded however long it runs.
    #[test]
    fn the_waves_take_turns() {
        let mut r = start(StartArgs::new(640, 480, "waves=3", 20260811));
        let mut hi = 0.0f32;
        for _ in 0..2000 {
            r.step();
            for v in &r.frame().vertices {
                hi = hi.max(v.pos[2].abs());
            }
        }
        assert!(hi > 0.05, "the sheet never moved: {hi}");
        assert!(hi <= 1.0, "the sheet ran away: {hi}");
    }

    /// The picture goes on as one texture, and its coordinates run outside
    /// the unit square for the wrapping to bring back, which is upstream's
    /// texture matrix done by hand.
    #[test]
    fn the_picture_wraps_onto_the_sheet() {
        let mut r = start(StartArgs::new(640, 480, "", 20260811));
        r.step();
        let f = r.frame();
        let textured: Vec<_> = f.batches.iter().filter(|b| b.texture.is_some()).collect();
        assert!(!textured.is_empty(), "the picture is not on it");
        let uvs: Vec<[f32; 2]> = textured
            .iter()
            .flat_map(|b| f.vertices[b.first..b.first + b.count].iter().map(|v| v.uv))
            .collect();
        let lo = uvs.iter().map(|uv| uv[0]).fold(f32::MAX, f32::min);
        let hi = uvs.iter().map(|uv| uv[0]).fold(f32::MIN, f32::max);
        assert!(lo <= -0.99 && hi >= -0.01, "the coordinates are {lo}..{hi}");
    }

    /// Every mode draws something, and only the wire one turns the depth test
    /// off.
    #[test]
    fn every_mode_draws() {
        for mode in ["wire", "solid", "light", "checker", "grab"] {
            let mut r = start(StartArgs::new(640, 480, &format!("mode={mode}"), 20260811));
            r.step();
            let f = r.frame();
            assert!(!f.vertices.is_empty(), "{mode} drew nothing");
            let depth = f.batches.iter().any(|b| b.depth_test);
            assert_eq!(depth, mode != "wire", "{mode} has the wrong depth test");
        }
    }
}
