//! Port of `hacks/glx/gleidescope.c`.
//!
//! ```text
//! gleidescope, Copyright (c) 2003 Andrew Dean
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
//! Texture loading code from 'glplanet' by Jamie Zawinski <jwz@jwz.org>.
//! Grab code from 'glslideshow' by Mike Oliphant, Ben Buxton and
//! Jamie Zawinski.
//! ```
//!
//! A kaleidoscope that operates on a loaded image.
//!
//! The whole thing is one hexagon repeated over a grid of a hundred and
//! twenty-seven of them. Each hexagon is six triangles round its middle, and
//! every one of the six takes the *same* three texture coordinates, so the same
//! triangle of the picture is reflected six ways about the centre. That is the
//! kaleidoscope: there is no mirror geometry anywhere, only one triangle of
//! image used six times.
//!
//! What moves is where that triangle sits on the picture. Its centre traces a
//! Lissajous figure and the triangle spins about the centre as it goes, each at
//! its own randomly chosen period, so the pattern never repeats in any
//! reasonable time. The camera meanwhile drifts, rolls and pulls in and out,
//! each on its own sine, and the roll accelerates and decelerates at random so
//! the twisting never settles into a rhythm.
//!
//! Upstream's `-image` also takes `GENERATE`, which paints twenty-five random
//! rectangles, circles and triangles into a texture instead of using a
//! picture. That is not ported: the panel here is generated from the saver's
//! XML, the XML has no control for it, and the picture channel always answers.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
    screenhack_event_helper,
};

/// sin 60, half the width of a hexagon.
const XOFFSET: f32 = 0.866_025_4;
/// cos 60 plus one, how far apart two rows of hexagons are.
const YOFFSET: f32 = 1.5;

const MAX_CAM_SPEED: f32 = 1.0;
const MAX_ANGLE_VEL: f32 = 1.0;
const INITIAL_ANGLE_VEL: f32 = 0.2;
const INITIAL_ANGLE_ACC: f32 = 0.001;
/// One frame in this many changes the twisting acceleration.
const TWISTING_PROBABILITY: u32 = 1000;

const RADIANS: f32 = std::f32::consts::PI / 180.0;
const ANGLE_120: f32 = std::f32::consts::PI * 2.0 / 3.0;
const ANGLE_240: f32 = std::f32::consts::PI * 4.0 / 3.0;

/// How many frames one picture takes to fade into the next.
const MAX_FADE: i32 = 500;

/// How far out the grid of hexagons reaches. Upstream writes the whole grid
/// out as a table of a hundred and twenty-seven entries and leaves the loop
/// that would have generated it commented out beside it, calling the table
/// terrible; this is that loop.
const GRID_SIZE: i32 = 6;

fn frandrange(x: f64, y: f64) -> f64 {
    x + frand(y - x)
}

/// Where every hexagon of the grid goes. A row of `2i+1` of them, widening to
/// the middle and narrowing again, which tiles the plane.
fn hex_grid() -> Vec<(f32, f32)> {
    let mut v = Vec::new();
    let mut i = GRID_SIZE;
    for y in -GRID_SIZE..=GRID_SIZE {
        let mut x = -i;
        while x <= i {
            v.push((XOFFSET * x as f32, YOFFSET * y as f32));
            x += 2;
        }
        if y < 0 {
            i += 1;
        } else {
            i -= 1;
        }
    }
    v
}

/// The six corners of a hexagon, and its middle. Every triangle runs from the
/// middle out to two neighbouring corners.
const HEX_VERTS: [[f32; 2]; 7] = [
    [0.0, 0.0],
    [0.0, 1.0],
    [XOFFSET, 0.5],
    [XOFFSET, -0.5],
    [0.0, -1.0],
    [-XOFFSET, -0.5],
    [-XOFFSET, 0.5],
];

/// The six triangles, wound so that consecutive ones are mirror images: 0-1-6,
/// 0-6-5, 0-5-4, and so on round. Which of the three texture coordinates each
/// corner takes alternates with them, and that alternation is the reflection.
const HEX_TRIS: [[(usize, usize); 3]; 6] = [
    [(0, 0), (1, 1), (6, 2)],
    [(0, 0), (6, 2), (5, 1)],
    [(0, 0), (5, 1), (4, 2)],
    [(0, 0), (4, 2), (3, 1)],
    [(0, 0), (3, 1), (2, 2)],
    [(0, 0), (2, 2), (1, 1)],
];

/// One picture, and where on it the triangle wanders.
struct Texture {
    id: Option<u32>,
    min_tx: f32,
    min_ty: f32,
    max_tx: f32,
    max_ty: f32,
    /// The periods and phases of the Lissajous figure the triangle's centre
    /// follows, and of its own spin.
    x_period: f32,
    y_period: f32,
    r_period: f32,
    x_phase: f32,
    y_phase: f32,
    r_phase: f32,
}

impl Texture {
    fn new() -> Self {
        Texture {
            id: None,
            min_tx: 0.0,
            min_ty: 0.0,
            max_tx: 1.0,
            max_ty: 1.0,
            x_period: frandrange(-2.0, 2.0) as f32,
            y_period: frandrange(-2.0, 2.0) as f32,
            r_period: frandrange(-2.0, 2.0) as f32,
            x_phase: frand(std::f64::consts::TAU) as f32,
            y_phase: frand(std::f64::consts::TAU) as f32,
            r_phase: frand(std::f64::consts::TAU) as f32,
        }
    }
}

struct Gleidescope {
    cam_x_speed: f32,
    cam_y_speed: f32,
    cam_z_speed: f32,
    cam_x_phase: f32,
    cam_y_phase: f32,
    cam_z_phase: f32,
    tic: f32,

    textures: [Texture; 2],
    visible: usize,
    /// Zero when not fading, otherwise how far through the crossfade it is.
    fade: i32,
    start_time: f64,

    grid: Vec<(f32, f32)>,

    /// Where the triangle is on the picture, and how fast that is changing.
    tangle: f32,
    tangle_vel: f32,
    tangle_acc: f32,
    /// The camera's roll, likewise.
    rangle: f32,
    rangle_vel: f32,
    rangle_acc: f32,

    /// Dragging turns the picture and the camera by hand.
    button_down: bool,
    xstart: i32,
    ystart: i32,
    xmouse: f64,
    ymouse: f64,

    size: i32,
    duration: f64,
    move_p: bool,
    rotate_p: bool,
    zoom_p: bool,
}

impl Gleidescope {
    /// `calculate_texture_coords`, in the Lissajous form upstream compiles in.
    ///
    /// The triangle's centre goes round a Lissajous figure inside the middle
    /// fifth of the picture, and the triangle itself turns about that centre.
    /// Three of upstream's four versions of this are `#if 0`'d out; this is
    /// the live one.
    fn texture_coords(&self, t: &Texture) -> [[f32; 2]; 3] {
        let width = t.max_tx - t.min_tx;
        let height = t.max_ty - t.min_ty;
        let centre_x = t.min_tx + width * 0.5;
        let centre_y = t.min_ty + height * 0.5;
        // The triangle is thirty per cent of the space and wanders over
        // twenty, so together they stay inside the picture.
        let t_radius_x = width * 0.3;
        let t_radius_y = height * 0.3;
        let m_radius_x = width * 0.2;
        let m_radius_y = height * 0.2;

        let angle = (self.ymouse as f32 * std::f32::consts::TAU) + self.tangle * RADIANS;
        let cx = centre_x + m_radius_x * (t.x_period * angle + t.x_phase).cos();
        let cy = centre_y + m_radius_y * (t.y_period * angle + t.y_phase).sin();

        let a2 = t.r_period * angle + t.r_phase;
        [
            [cx + t_radius_x * a2.cos(), cy + t_radius_y * a2.sin()],
            [
                cx + t_radius_x * (a2 + ANGLE_120).cos(),
                cy + t_radius_y * (a2 + ANGLE_120).sin(),
            ],
            [
                cx + t_radius_x * (a2 + ANGLE_240).cos(),
                cy + t_radius_y * (a2 + ANGLE_240).sin(),
            ],
        ]
    }

    /// `draw_hexagons`: the whole grid, from one triangle of the picture.
    fn draw_hexagons(&self, g: &mut Gl, translucency: i32, which: usize) {
        let t = &self.textures[which];
        let Some(id) = t.id else { return };
        let tc = self.texture_coords(t);

        g.glx
            .color4f(1.0, 1.0, 1.0, translucency as f32 / MAX_FADE as f32);
        g.glx.texturing(true);
        g.glx.blend(Blend::Alpha);
        g.glx.depth_mask(false);
        g.glx.bind_texture(id);

        // Upstream compiles one hexagon into a display list and calls it at
        // each of the hundred and twenty-seven places. Here the offset is
        // folded into the vertices instead, so the whole grid is one draw
        // call rather than one apiece.
        g.glx.begin(Shape::Triangles);
        for &(hx, hy) in &self.grid {
            for tri in HEX_TRIS {
                for (v, c) in tri {
                    g.glx.tex_coord2f(tc[c][0], tc[c][1]);
                    g.glx
                        .vertex3f(hx + HEX_VERTS[v][0], hy + HEX_VERTS[v][1], 0.0);
                }
            }
        }
        g.glx.end();

        g.glx.depth_mask(true);
        g.glx.texturing(false);
        g.glx.blend(Blend::Off);
    }

    /// Ask for a picture, and put it in the given slot when it arrives.
    fn setup_texture(&mut self, g: &mut Gl, which: usize) -> bool {
        let size = g.width().max(g.height());
        let Some(img) = g.load_image(size, size) else {
            return false;
        };
        let id = self.textures[which]
            .id
            .unwrap_or_else(|| g.glx.gen_texture());
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(img.width, img.height, img.pixels);
        // Upstream leaves this on repeat, which matters: the triangle's
        // corners can wander a little outside the picture.
        g.glx.tex_clamp(false);

        let t = &mut self.textures[which];
        // Upstream stamps the load time on the texture and copies it up; the
        // copy is the only reader, so only the copy is kept.
        *t = Texture {
            id: Some(id),
            ..Texture::new()
        };
        self.start_time = g.time;
        true
    }
}

impl Hack3d for Gleidescope {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        // The first picture, and then a fresh one every `duration` seconds.
        if self.textures[self.visible].id.is_none() {
            let v = self.visible;
            self.setup_texture(g, v);
        }

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.lighting(false);
        g.glx.clear();
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        self.tic += 0.005;
        let x_angle = self.cam_x_phase + self.tic * self.cam_x_speed;
        let y_angle = self.cam_y_phase + self.tic * self.cam_y_speed;
        let z_angle = self.cam_z_phase + self.tic * self.cam_z_speed;

        let (vx, vy) = if self.move_p {
            (x_angle.sin(), y_angle.sin())
        } else {
            (0.0, 0.0)
        };

        let size = self.size.clamp(0, 10);
        let vz = if size > 0 {
            size as f32
        } else if self.zoom_p {
            // The furthest away is the constant plus the multiplier.
            5.0 + 3.0 * z_angle.sin()
        } else {
            7.0
        };

        if self.rotate_p && !self.button_down {
            self.rangle += self.rangle_vel;
            let v = self.rangle_vel + self.rangle_acc;
            if v > -MAX_ANGLE_VEL && v < MAX_ANGLE_VEL {
                self.rangle_vel = v;
            }
            if random().is_multiple_of(TWISTING_PROBABILITY) {
                self.rangle_acc = INITIAL_ANGLE_ACC * frand(1.0) as f32;
                if self.rangle_vel > 0.0 {
                    self.rangle_acc = -self.rangle_acc;
                }
            }
        }

        // The camera stays square on to the grid and rolls about its own axis,
        // which upstream found smoother than turning the whole scene.
        let roll = (self.xmouse as f32 * std::f32::consts::TAU) + self.rangle * RADIANS;
        g.glx
            .look_at([vx, vy, vz], [vx, vy, 0.0], [roll.sin(), roll.cos(), 0.0]);

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);

        if self.fade == 0 {
            self.draw_hexagons(g, MAX_FADE, self.visible);
        } else {
            // Both pictures at once, the old one solid and the new one
            // fading up over it.
            self.draw_hexagons(g, MAX_FADE, 1 - self.visible);
            self.draw_hexagons(g, MAX_FADE - self.fade, self.visible);
            self.fade += 1;
            if self.fade > MAX_FADE {
                self.fade = 0;
                self.visible = 1 - self.visible;
            }
        }

        if !self.button_down {
            self.tangle += self.tangle_vel;
            let v = self.tangle_vel + self.tangle_acc;
            if v > -MAX_ANGLE_VEL && v < MAX_ANGLE_VEL {
                self.tangle_vel = v;
            }
            if random().is_multiple_of(TWISTING_PROBABILITY) {
                self.tangle_acc = INITIAL_ANGLE_ACC * frand(1.0) as f32;
                if self.tangle_vel > 0.0 {
                    self.tangle_acc = -self.tangle_acc;
                }
            }
        }

        // Time for the next picture?
        if self.start_time != 0.0 && self.fade == 0 && self.start_time + self.duration <= g.time {
            // The new picture goes into the other slot and is drawn solid
            // underneath while the old one, which `visible` still points at,
            // fades out over it. Only when the fade finishes does `visible`
            // change hands.
            let other = 1 - self.visible;
            if self.setup_texture(g, other) {
                self.fade = 1;
            }
        }

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
        g.glx.perspective(50.0, 1.0 / h, 0.1, 2000.0);
        g.glx.matrix_mode_modelview();
        g.glx.line_width(1.0);
        g.glx.point_size(1.0);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        match event {
            XEvent::ButtonPress { x, y, .. } => {
                self.xstart = *x;
                self.ystart = *y;
                self.button_down = true;
                true
            }
            XEvent::ButtonRelease { .. } => {
                self.button_down = false;
                true
            }
            XEvent::MotionNotify { x, y } if self.button_down => {
                self.xmouse += f64::from(x - self.xstart) / f64::from(g.width().max(1));
                self.ymouse += f64::from(y - self.ystart) / f64::from(g.height().max(1));
                self.xstart = *x;
                self.ystart = *y;
                true
            }
            _ => {
                if screenhack_event_helper(event) {
                    // Fetch a new picture at once.
                    self.start_time = -1.0;
                    self.fade = 0;
                    return true;
                }
                false
            }
        }
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // Upstream has a pair of options for each of these so that "unset" can be
    // told from "off", and picks at random when neither was given. The panel
    // here has one control each, so a control that is off means off.
    let move_p = g.res.bool("move");
    let rotate_p = g.res.bool("rotate");
    let zoom_p = g.res.bool("zoom");

    let mut st = Gleidescope {
        cam_x_speed: MAX_CAM_SPEED * frandrange(-0.5, 0.5) as f32,
        cam_y_speed: MAX_CAM_SPEED * frandrange(-0.5, 0.5) as f32,
        cam_z_speed: MAX_CAM_SPEED * frandrange(-0.5, 0.5) as f32,
        cam_x_phase: (random() % 360) as f32,
        cam_y_phase: (random() % 360) as f32,
        cam_z_phase: (random() % 360) as f32,
        tic: 0.0,
        textures: [Texture::new(), Texture::new()],
        visible: 0,
        fade: 0,
        start_time: 0.0,
        grid: hex_grid(),
        tangle: 0.0,
        tangle_vel: INITIAL_ANGLE_VEL * frandrange(-0.5, 0.5) as f32,
        tangle_acc: INITIAL_ANGLE_ACC * frandrange(-0.5, 0.5) as f32,
        rangle: 0.0,
        rangle_vel: INITIAL_ANGLE_VEL * frandrange(-0.5, 0.5) as f32,
        rangle_acc: INITIAL_ANGLE_ACC * frandrange(-0.5, 0.5) as f32,
        button_down: false,
        xstart: 0,
        ystart: 0,
        xmouse: 0.0,
        ymouse: 0.0,
        size: g.res.int("size").clamp(0, 10),
        duration: f64::from(g.res.int("duration").clamp(10, 300)),
        move_p,
        rotate_p,
        // A size was asked for, so there is nothing left for zoom to do.
        zoom_p: zoom_p && g.res.int("size") <= 0,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:    20000",
    "*showFPS:  False",
    "*size:     0",
    "*duration: 30",
    "*move:     True",
    "*rotate:   True",
    "*zoom:     False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("size", "Size of tube", 0.0, 10.0, 1.0, 0, "0"),
    Opt::slider("duration", "Image duration", 10.0, 300.0, 5.0, 0, "30"),
    Opt::boolean("move", "Move", "true"),
    Opt::boolean("rotate", "Rotate", "true"),
    Opt::boolean("zoom", "Zoom", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "gleidescope",
    label: "Gleidescope",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Andrew Dean",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=q6F-CDX6-tU"),
        blurb: "A kaleidoscope that operates on a loaded image.",
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

    /// The grid is the table upstream wrote out by hand: a hundred and
    /// twenty-seven hexagons in thirteen rows, widening to the middle.
    #[test]
    fn the_grid_is_the_one_upstream_tabulated() {
        let g = hex_grid();
        assert_eq!(g.len(), 127);

        // The first row and the last are seven wide, the middle thirteen.
        let row = |y: i32| {
            g.iter()
                .filter(|(_, gy)| (gy - YOFFSET * y as f32).abs() < 1e-4)
                .count()
        };
        assert_eq!(row(-6), 7);
        assert_eq!(row(0), 13);
        assert_eq!(row(6), 7);

        // The first three entries, straight out of upstream's table.
        assert!((g[0].0 - XOFFSET * -6.0).abs() < 1e-6);
        assert!((g[0].1 - YOFFSET * -6.0).abs() < 1e-6);
        assert!((g[1].0 - XOFFSET * -4.0).abs() < 1e-6);
        assert!((g[7].0 - XOFFSET * -7.0).abs() < 1e-6);
        assert!((g[7].1 - YOFFSET * -5.0).abs() < 1e-6);
    }

    /// Hexagons tile: every one has six neighbours a hexagon's width away,
    /// except at the edge of the grid.
    #[test]
    fn the_hexagons_tile_without_gaps() {
        let g = hex_grid();
        let step = (XOFFSET * XOFFSET + (YOFFSET / 1.5) * (YOFFSET / 1.5)).sqrt();
        let mut interior = 0;
        for (i, a) in g.iter().enumerate() {
            let n = g
                .iter()
                .enumerate()
                .filter(|(j, b)| {
                    *j != i
                        && ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt() - step * 2.0 < 1e-3
                })
                .count();
            assert!(n <= 6, "a hexagon has {n} neighbours");
            if n == 6 {
                interior += 1;
            }
        }
        // The middle of a grid this size is mostly interior.
        assert!(interior > 60, "only {interior} hexagons are surrounded");
    }

    /// The kaleidoscope: all six triangles of a hexagon use the same three
    /// texture coordinates, and consecutive triangles swap two of them, which
    /// is what makes each the mirror of the last.
    #[test]
    fn the_six_triangles_are_one_triangle_reflected() {
        for tri in HEX_TRIS {
            assert_eq!(tri[0].1, 0, "every triangle starts at the middle");
            let mut used: Vec<usize> = tri.iter().map(|(_, c)| *c).collect();
            used.sort_unstable();
            assert_eq!(used, vec![0, 1, 2], "a triangle repeats a corner");
        }
        // Neighbouring triangles share an edge of the hexagon, and give that
        // shared corner the same texture coordinate as each other.
        for i in 0..6 {
            let a = HEX_TRIS[i];
            let b = HEX_TRIS[(i + 1) % 6];
            let shared: Vec<_> = a
                .iter()
                .filter(|(v, _)| b.iter().any(|(w, _)| w == v))
                .collect();
            assert!(
                shared.len() >= 2,
                "triangles {i} and {} do not share an edge",
                (i + 1) % 6
            );
        }
    }

    /// The triangle stays inside the picture: its centre wanders over a fifth
    /// of it and the triangle itself reaches three tenths, so together they
    /// stay in bounds.
    #[test]
    fn the_triangle_stays_on_the_picture() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        r.step();

        let t = Texture {
            id: Some(1),
            min_tx: 0.0,
            min_ty: 0.0,
            max_tx: 1.0,
            max_ty: 1.0,
            ..Texture::new()
        };
        let mut st = a_kaleidoscope();
        for i in 0..2000 {
            st.tangle = i as f32 * 0.37;
            for c in st.texture_coords(&t) {
                assert!(
                    (0.0..=1.0).contains(&c[0]) && (0.0..=1.0).contains(&c[1]),
                    "a corner left the picture at {c:?}"
                );
            }
        }
    }

    /// The twisting is bounded: the velocity is only accepted while it is
    /// inside the limit, so it can never run away.
    #[test]
    fn the_twisting_stays_inside_its_limits() {
        let mut r = start(StartArgs::new(640, 480, "rotate=true", 20260812));
        for _ in 0..3000 {
            r.step();
        }
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "it stopped drawing");
    }

    /// It draws the grid from the picture, in one call rather than one per
    /// hexagon.
    #[test]
    fn the_whole_grid_is_drawn_from_the_picture() {
        let r = run("", 3);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        let textured: Vec<_> = f.batches.iter().filter(|b| b.texture.is_some()).collect();
        assert_eq!(textured.len(), 1, "the grid took {} calls", textured.len());
        assert_eq!(
            textured[0].count,
            127 * 6 * 3,
            "the grid is not 127 hexagons of six triangles"
        );
    }

    /// A kaleidoscope with no GL behind it.
    fn a_kaleidoscope() -> Gleidescope {
        Gleidescope {
            cam_x_speed: 0.1,
            cam_y_speed: 0.1,
            cam_z_speed: 0.1,
            cam_x_phase: 0.0,
            cam_y_phase: 0.0,
            cam_z_phase: 0.0,
            tic: 0.0,
            textures: [Texture::new(), Texture::new()],
            visible: 0,
            fade: 0,
            start_time: 0.0,
            grid: hex_grid(),
            tangle: 0.0,
            tangle_vel: 0.1,
            tangle_acc: 0.0,
            rangle: 0.0,
            rangle_vel: 0.1,
            rangle_acc: 0.0,
            button_down: false,
            xstart: 0,
            ystart: 0,
            xmouse: 0.0,
            ymouse: 0.0,
            size: 0,
            duration: 30.0,
            move_p: true,
            rotate_p: true,
            zoom_p: false,
        }
    }
}
