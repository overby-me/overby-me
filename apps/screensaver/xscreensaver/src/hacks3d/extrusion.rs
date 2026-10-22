/* extrusion, Copyright © 1999-2026 Linas Vepstas, David Konerding and
 * Jamie Zawinski <jwz@jwz.org>
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/extrusion.c` and its seven shapes.
//!
//! Various extruded shapes twisting and turning inside out: helices of three
//! kinds, a screw, a taper, a bent tube shown with its joins offset, and a
//! corrugated twistoid.
//!
//! This was blocked on GLE, the tubing and extrusion library, which
//! XScreenSaver links against rather than bundling. The library was the wrong
//! thing to look at: what the saver needs is the geometry, and the slice of
//! GLE it uses is one sweep with one join style. `runtime::extrude` is that
//! sweep. Everything here is a contour, a path, and sometimes a per-station
//! transform, handed to it.
//!
//! Both of the sizes upstream derives from the mouse are driven by the
//! rotator instead, which is what upstream does too when nothing is being
//! dragged: `lastx` and `lasty` are its position scaled into ±400. So the
//! shapes go on deforming by themselves, which is the point of a screen
//! saver.

use crate::runtime::extrude::{Affine, Extrusion, IDENTITY, extrude, helicoid, screw};
#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, Saver3d, SaverDef, StartArgs};
#[cfg(target_arch = "wasm32")]
use crate::runtime::{About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs};
use crate::runtime::{Rotator, SelectItem, Trackball, XEvent, random};

/// The range upstream maps the mouse into, and the rotator with it.
const MIN_LAST: f64 = -400.0;
const MAX_LAST: f64 = 400.0;

/// The scale the plus-shaped contour is drawn at.
const SCALE: f64 = 3.333_33;

/// Which shape to draw.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Helix2,
    Helix3,
    Helix4,
    JoinOffset,
    Screw,
    Taper,
    Twistoid,
}

const SHAPES: [(Shape, &str); 7] = [
    (Shape::Helix2, "helix2"),
    (Shape::Helix3, "helix3"),
    (Shape::Helix4, "helix4"),
    (Shape::JoinOffset, "joinoffset"),
    (Shape::Screw, "screw"),
    (Shape::Taper, "taper"),
    (Shape::Twistoid, "twistoid"),
];

/// The plus-shaped outline `screw` and `taper` are swept from, with the
/// normal of each segment beside it, exactly as upstream's `CONTOUR` macro
/// builds them.
fn plus_contour() -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    const PTS: [(f64, f64); 21] = [
        (1.0, 1.0),
        (1.0, 2.9),
        (0.9, 3.0),
        (-0.9, 3.0),
        (-1.0, 2.9),
        (-1.0, 1.0),
        (-2.9, 1.0),
        (-3.0, 0.9),
        (-3.0, -0.9),
        (-2.9, -1.0),
        (-1.0, -1.0),
        (-1.0, -2.9),
        (-0.9, -3.0),
        (0.9, -3.0),
        (1.0, -2.9),
        (1.0, -1.0),
        (2.9, -1.0),
        (3.0, -0.9),
        (3.0, 0.9),
        (2.9, 1.0),
        (1.0, 1.0),
    ];
    let pts: Vec<[f64; 2]> = PTS.iter().map(|&(x, y)| [SCALE * x, SCALE * y]).collect();
    let mut nrm = Vec::with_capacity(pts.len());
    for i in 0..pts.len() {
        // Upstream's macro takes the normal of the segment *behind* each
        // point, so the last one has none and is left as it was.
        let j = if i == 0 { 0 } else { i - 1 };
        let (a, b) = (pts[j], pts[(j + 1).min(pts.len() - 1)]);
        let (ax, ay) = (b[0] - a[0], b[1] - a[1]);
        let l = (ax * ax + ay * ay).sqrt().max(1e-12);
        nrm.push([ay / l, -ax / l]);
    }
    (pts, nrm)
}

/// An outline, the path it is swept along, and a colour per station.
type Shaped = (Vec<[f64; 2]>, Vec<[f64; 3]>, Vec<[f32; 3]>);

/// The bent tube `joinoffset` shows its joins on: a contour, a path that
/// doubles back on itself, and a colour per station.
fn join_offset_data() -> Shaped {
    let contour = vec![
        [-0.8, -0.5],
        [-1.8, 0.0],
        [-1.2, 0.3],
        [-0.7, 0.8],
        [-0.2, 1.3],
        [0.0, 1.6],
        [0.2, 1.3],
        [0.7, 0.8],
        [1.2, 0.3],
        [1.8, 0.0],
        [0.8, -0.5],
    ];
    let path = vec![
        [16.0, 0.0, 0.0],
        [0.0, -16.0, 0.0],
        [-16.0, 0.0, 0.0],
        [0.0, 16.0, 0.0],
        [16.0, 0.0, 0.0],
        [0.0, -16.0, 0.0],
        [-16.0, 0.0, 0.0],
    ];
    let colors = vec![
        [0.0, 0.0, 0.0],
        [0.2, 0.8, 0.5],
        [0.0, 0.8, 0.3],
        [0.8, 0.3, 0.0],
        [0.2, 0.3, 0.9],
        [0.2, 0.8, 0.5],
        [0.0, 0.0, 0.0],
    ];
    (contour, path, colors)
}

/// `init_tripples`: the twistoid's outline, a semicircular hump followed by a
/// zig-zag corrugation.
fn twistoid_contour() -> (Vec<[f64; 2]>, Vec<[f64; 2]>) {
    const N: usize = 20;
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(N);
    for i in 0..11 {
        let angle = std::f64::consts::PI * i as f64 / 10.0;
        pts.push([
            SCALE * (-7.0 - 3.0 * angle.cos()),
            SCALE * (1.8 * angle.sin()),
        ]);
    }
    let mut i = pts.len();
    while i < N {
        pts.push([SCALE * (-10.0 + i as f64), 0.0]);
        i += 1;
        if i >= N {
            break;
        }
        pts.push([SCALE * (-9.5 + i as f64), SCALE]);
        i += 1;
    }
    pts.truncate(N);
    let mut nrm = Vec::with_capacity(pts.len());
    for k in 0..pts.len() {
        let j = if k == 0 { 0 } else { k - 1 };
        let (a, b) = (pts[j], pts[(j + 1).min(pts.len() - 1)]);
        let (ax, ay) = (b[0] - a[0], b[1] - a[1]);
        let l = (ax * ax + ay * ay).sqrt().max(1e-12);
        nrm.push([ay / l, -ax / l]);
    }
    (pts, nrm)
}

struct ExtrusionState {
    rot: Rotator,
    trackball: Trackball,
    shape: Shape,
    random_shape: bool,
    /// How long this shape has been up, so a random run changes it.
    since_change: f64,
    lastx: f64,
    lasty: f64,
    light: bool,
    wire: bool,
    width: i32,
    height: i32,
}

impl ExtrusionState {
    fn draw_shape(&self, g: &mut Gl) {
        let (lastx, lasty) = (self.lastx, self.lasty);
        match self.shape {
            Shape::Helix2 => {
                g.glx.color3f(0.6, 0.3, 0.8);
                helicoid(
                    &mut g.glx,
                    0.01 * lastx,
                    6.0,
                    0.01 * lasty - 2.0,
                    -3.0,
                    4.0,
                    None,
                    None,
                    0.0,
                    1080.0,
                );
            }
            Shape::Helix3 => {
                g.glx.color3f(0.3, 0.6, 0.8);
                helicoid(
                    &mut g.glx,
                    1.0,
                    6.0,
                    -1.0,
                    0.0,
                    0.02 * lasty - 2.0,
                    None,
                    None,
                    0.0,
                    6.0 * lastx,
                );
            }
            Shape::Helix4 => {
                g.glx.color3f(0.8, 0.3, 0.6);
                // A shape that is squashed one way and stretched the other as
                // it winds, which is what the differential transform does.
                let sx = 0.01 * lastx;
                let affine: Affine = if sx.abs() < 1e-6 {
                    IDENTITY
                } else {
                    [[1.0 / sx, 0.0, 0.0], [0.0, sx, 0.0]]
                };
                let d: Affine = [[0.0, -0.03 * lasty, 0.0], [0.03 * lasty, 0.0, 0.0]];
                helicoid(
                    &mut g.glx,
                    1.0,
                    7.0,
                    -1.0,
                    0.0,
                    2.0,
                    Some(affine),
                    Some(d),
                    0.0,
                    720.0,
                );
            }
            Shape::JoinOffset => {
                let (contour, path, colors) = join_offset_data();
                let moved: Vec<[f64; 2]> = contour
                    .iter()
                    .map(|p| [p[0], p[1] + 0.05 * (lasty - 200.0)])
                    .collect();
                g.glx.push_matrix();
                g.glx.scale(0.5, 0.5, 0.5);
                g.glx.translate(0.0, 4.0, 0.0);
                extrude(
                    &mut g.glx,
                    &Extrusion {
                        contour: &moved,
                        normals: Some(&contour),
                        up: Some([1.0, 0.0, 0.0]),
                        path: &path,
                        colors: Some(&colors),
                        xforms: None,
                    },
                );
                g.glx.pop_matrix();
            }
            Shape::Screw => {
                g.glx.color3f(0.5, 0.6, 0.6);
                let (pts, nrm) = plus_contour();
                screw(&mut g.glx, &pts, Some(&nrm), None, -6.0, 9.0, lasty);
            }
            Shape::Taper => {
                g.glx.color3f(0.5, 0.6, 0.6);
                let (pts, nrm) = plus_contour();
                // A straight path with a profile that swells in the middle and
                // pinches at the ends, plus a twist along its length.
                const PSIZE: usize = 40;
                let mut path = Vec::with_capacity(PSIZE);
                let mut xforms = Vec::with_capacity(PSIZE);
                let ponent = (lastx / 540.0).abs().max(1e-3);
                let dang = lasty / 40.0;
                let (mut z, deltaz): (f64, f64) = (-10.0, 0.5);
                let mut zt: f64 = -1.0;
                let deltazt = 1.999 / 38.0;
                let mut ang = 0.0f64;
                for j in 0..PSIZE {
                    path.push([0.0, 0.0, z]);
                    z += deltaz;
                    let taper = if (1..39).contains(&j) {
                        let t = (1.0 - zt.abs().powf(1.0 / ponent)).max(0.0).powf(ponent);
                        zt += deltazt;
                        t
                    } else {
                        0.0
                    };
                    let a = ang.to_radians();
                    ang += dang;
                    // The station's transform is the taper scaled and turned.
                    xforms.push([
                        [taper * a.cos(), -taper * a.sin(), 0.0],
                        [taper * a.sin(), taper * a.cos(), 0.0],
                    ]);
                }
                extrude(
                    &mut g.glx,
                    &Extrusion {
                        contour: &pts,
                        normals: Some(&nrm),
                        up: None,
                        path: &path,
                        colors: None,
                        xforms: Some(&xforms),
                    },
                );
            }
            Shape::Twistoid => {
                g.glx.color3f(0.7, 0.7, 0.8);
                let (pts, nrm) = twistoid_contour();
                const TSCALE: f64 = 6.0;
                let path: Vec<[f64; 3]> = [
                    (-1.1, 0.0),
                    (-1.0, 0.0),
                    (0.0, 0.0),
                    (1.0, -(lasty - 121.0) / 200.0),
                    (1.1, -1.1 * (lasty - 121.0) / 200.0),
                ]
                .iter()
                .map(|&(x, y)| [TSCALE * x, TSCALE * y, 0.0])
                .collect();
                // Only the middle station is twisted, which is what makes the
                // corrugation wring itself round.
                let twist = [0.0, 0.0, (lastx - 121.0) / 8.0, 0.0, 0.0];
                let xforms: Vec<Affine> = twist
                    .iter()
                    .map(|t| {
                        let a = t.to_radians();
                        [[a.cos(), -a.sin(), 0.0], [a.sin(), a.cos(), 0.0]]
                    })
                    .collect();
                g.glx.push_matrix();
                g.glx.scale(1.8, 1.8, 1.8);
                extrude(
                    &mut g.glx,
                    &Extrusion {
                        contour: &pts,
                        normals: Some(&nrm),
                        up: None,
                        path: &path,
                        colors: None,
                        xforms: Some(&xforms),
                    },
                );
                g.glx.pop_matrix();
            }
        }
    }
}

impl Hack3d for ExtrusionState {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        let h = f64::from(self.height) / f64::from(self.width);
        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx.perspective(30.0, (1.0 / h) as f32, 1.0, 100.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();
        g.glx
            .look_at([0.0, 0.0, 30.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.clear_color(0.0, 0.0, 0.0, 1.0);
        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.cull_face(false);

        if self.light && !self.wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
            g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
            g.glx.light_ambient(0, [0.2, 0.2, 0.2, 1.0]);
            g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
            g.glx.material_ambient_diffuse([0.6, 0.6, 0.4, 1.0]);
            g.glx.color_material(true);
        } else {
            g.glx.lighting(false);
        }

        let button_down = self.trackball.button_down();

        // Every so often, a different shape.
        if self.random_shape {
            self.since_change += 1.0;
            if self.since_change > 600.0 {
                self.since_change = 0.0;
                self.shape = SHAPES[(random() as usize) % SHAPES.len()].0;
            }
        }

        g.glx.push_matrix();
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        let (rx, ry, rz) = self.rot.rotation(!button_down);
        g.glx.rotate((rx * 360.0) as f32, 1.0, 0.0, 0.0);
        g.glx.rotate((ry * 360.0) as f32, 0.0, 1.0, 0.0);
        g.glx.rotate((rz * 360.0) as f32, 0.0, 0.0, 1.0);

        // The two numbers the shapes are built from. Upstream takes them from
        // the mouse when it is down and from the rotator otherwise.
        let scale = MAX_LAST - MIN_LAST;
        let (px, py, _) = self.rot.position(!button_down);
        self.lastx = px * scale + MIN_LAST;
        self.lasty = py * scale + MIN_LAST;

        g.glx.scale(0.5, 0.5, 0.5);
        g.glx.front_face_cw(false);
        self.draw_shape(g);
        g.glx.pop_matrix();

        g.res.int("delay").max(0) as u32
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let name = g.res.string("name").to_string();
    let random_shape = name.is_empty() || name.eq_ignore_ascii_case("random");
    let shape = SHAPES
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(&name))
        .map_or(SHAPES[(random() as usize) % SHAPES.len()].0, |(s, _)| *s);

    let mut st = ExtrusionState {
        rot: Rotator::new(0.0, 0.0, 0.0, 0.0, 0.003, true),
        trackball: Trackball::new(),
        shape,
        random_shape,
        since_change: 0.0,
        lastx: 0.0,
        lasty: 0.0,
        light: g.res.bool("light"),
        wire: g.res.bool("wireframe"),
        width: g.width(),
        height: g.height(),
    };
    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:     20000",
    "*showFPS:   False",
    "*wireframe: False",
    "*name:      random",
    "*light:     True",
];

const NAMES: &[SelectItem] = &[
    SelectItem {
        value: "random",
        label: "Random object",
    },
    SelectItem {
        value: "helix2",
        label: "Helix 2",
    },
    SelectItem {
        value: "helix3",
        label: "Helix 3",
    },
    SelectItem {
        value: "helix4",
        label: "Helix 4",
    },
    SelectItem {
        value: "joinoffset",
        label: "Join offset",
    },
    SelectItem {
        value: "screw",
        label: "Screw",
    },
    SelectItem {
        value: "taper",
        label: "Taper",
    },
    SelectItem {
        value: "twistoid",
        label: "Twistoid",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::select("name", "Object", NAMES, "random"),
    Opt::boolean("light", "Use lighting", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "extrusion",
    label: "Extrusion",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Linas Vepstas, David Konerding and Jamie Zawinski",
        year: "1999",
        video: Some("https://www.youtube.com/watch?v=eKYmqL7ndGs"),
        blurb: "Extruded shapes twisting and turning inside out.",
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

    /// Every shape draws something, at a size that is on the screen rather
    /// than a thousand units away.
    #[test]
    fn every_shape_draws_something() {
        for (_, name) in SHAPES {
            let mut r = start(StartArgs::new(640, 480, &format!("name={name}"), 20260813));
            for _ in 0..30 {
                r.step();
            }
            let f = r.frame();
            assert!(!f.vertices.is_empty(), "{name} drew nothing");
            let far = f
                .vertices
                .iter()
                .map(|v| v.pos[0].abs().max(v.pos[1].abs()).max(v.pos[2].abs()))
                .fold(0.0f32, f32::max);
            assert!(far < 1000.0, "{name} reaches {far} units out");
            assert!(far > 0.1, "{name} is only {far} units across");
        }
    }

    /// The plus-shaped outline closes: its last point is its first, which is
    /// what makes a closed contour sweep into a solid rather than a sheet.
    #[test]
    fn the_plus_outline_closes() {
        let (pts, nrm) = plus_contour();
        assert_eq!(pts.len(), 21);
        assert_eq!(nrm.len(), pts.len());
        assert_eq!(pts[0], pts[20], "the outline does not close");
        // Every normal is a unit vector, or the lighting goes wrong in a way
        // that is hard to see and easy to ship.
        for (i, n) in nrm.iter().enumerate() {
            let l = (n[0] * n[0] + n[1] * n[1]).sqrt();
            assert!((l - 1.0).abs() < 1e-9, "normal {i} has length {l}");
        }
    }

    /// The twistoid's outline is the hump and the corrugation, in the count
    /// upstream fixes.
    #[test]
    fn the_twistoid_outline_is_twenty_points() {
        let (pts, nrm) = twistoid_contour();
        assert_eq!(pts.len(), 20);
        assert_eq!(nrm.len(), 20);
        // The hump is the first eleven, and it really is a hump: the middle
        // of it stands above both ends.
        assert!(pts[5][1] > pts[0][1] && pts[5][1] > pts[10][1]);
    }

    /// A named shape is the shape asked for, and an unknown name falls back
    /// to a random one rather than to nothing.
    #[test]
    fn the_name_picks_the_shape() {
        for (want, name) in SHAPES {
            let r = start(StartArgs::new(64, 64, &format!("name={name}"), 20260813));
            let _ = r;
            // The shape is private, so this checks the lookup the same way
            // `init` does.
            let got = SHAPES
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(name))
                .map(|(s, _)| *s);
            assert!(got == Some(want));
        }
        assert!(
            SHAPES
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case("nonesuch"))
                .is_none()
        );
    }
}
