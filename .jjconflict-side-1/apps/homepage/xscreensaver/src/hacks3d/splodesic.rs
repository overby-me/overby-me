//! Port of `hacks/glx/splodesic.c`.
//!
//! ```text
//! splodesic, Copyright (c) 2016 Jamie Zawinski <jwz@jwz.org>
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
//! A geodesic sphere experiences a series of eruptions.
//!
//! Every triangle of the sphere is a separate object with an altitude, a
//! velocity and a bit of gravity. Now and then one of them is given some
//! thrust, and the thrust spreads to its neighbours, and theirs, each time
//! multiplied by a little less than one until it falls below a tenth and
//! stops. So one eruption is a shockwave running outwards over the surface, and
//! every triangle it reaches is thrown up and falls back on its own.
//!
//! What makes it read as a sphere splitting rather than as confetti is that a
//! triangle only ever moves along the line from the middle of the sphere
//! through the middle of itself. Nothing rotates and nothing drifts sideways,
//! so however far the pieces fly they always look like they would fit back
//! together.
//!
//! The inside is a different colour from the outside, half the colourmap away,
//! which is why the picture is two-tone the moment anything opens up. Culling
//! is off and the lighting is two-sided so both are visible at once.
//!
//! Two departures from the C, neither of them visible. Upstream finds each
//! triangle's neighbours by comparing every triangle against every other one,
//! and says in a comment that there must be a faster way than that; at the
//! default depth that is thirteen million pairs of triangles to compare at
//! startup, which is a pause here rather than a shrug. This indexes the edges
//! instead, which is linear and finds exactly the same neighbours. And the
//! depth is capped at five rather than upstream's ten, which is what the panel
//! offers anyway: the count is twenty times four to the depth, so a depth of
//! ten is twenty million triangles and no browser is going to draw that.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::gl::Shape;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random_below,
};
use std::collections::HashMap;

/// `BELLRAND`: three uniform draws averaged, so the middle is likelier than
/// either end.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

/// A latitude and a longitude, which is how the geodesic is subdivided: the
/// midpoint of two corners is found in space and then turned back into an
/// angle, which is what pushes it out onto the sphere.
#[derive(Clone, Copy)]
struct Ll {
    a: f64,
    o: f64,
}

impl Ll {
    fn xyz(self) -> [f32; 3] {
        [
            (self.a.cos() * self.o.cos()) as f32,
            (self.a.cos() * self.o.sin()) as f32,
            self.a.sin() as f32,
        ]
    }
}

struct Triangle {
    p: [[f32; 3]; 3],
    neighbors: [usize; 3],
    altitude: f32,
    velocity: f32,
    thrust: f32,
    thrust_duration: i32,
    /// Whether this triangle has already taken thrust this tick, so the
    /// shockwave passes over it once rather than round in circles.
    hit: bool,
}

struct Splodesic {
    rot: Rotator,
    trackball: Trackball,
    triangles: Vec<Triangle>,
    colors: Vec<XColor>,
    ccolor: usize,
    speed: f64,
    depth: i32,
    wireframe: bool,
}

/// Creates a triangle specified by 3 polar endpoints.
fn make_triangle1(out: &mut Vec<Triangle>, v1: Ll, v2: Ll, v3: Ll) {
    out.push(Triangle {
        p: [v1.xyz(), v2.xyz(), v3.xyz()],
        neighbors: [0; 3],
        altitude: 0.0,
        velocity: 0.0,
        thrust: 0.0,
        thrust_duration: 0,
        hit: false,
    });
}

/// Computes the midpoint of a line between two polar coords.
fn midpoint2(v1: Ll, v2: Ll) -> Ll {
    let p1 = v1.xyz();
    let p2 = v2.xyz();
    let pm = [
        f64::from(p1[0] + p2[0]) / 2.0,
        f64::from(p1[1] + p2[1]) / 2.0,
        f64::from(p1[2] + p2[2]) / 2.0,
    ];
    let hyp = (pm[0] * pm[0] + pm[1] * pm[1]).sqrt();
    Ll {
        o: pm[1].atan2(pm[0]),
        a: pm[2].atan2(hyp),
    }
}

/// Creates triangular geodesic facets to the given depth.
fn make_triangle(out: &mut Vec<Triangle>, v1: Ll, v2: Ll, v3: Ll, depth: i32) {
    if depth <= 0 {
        make_triangle1(out, v1, v2, v3);
    } else {
        let v12 = midpoint2(v1, v2);
        let v23 = midpoint2(v2, v3);
        let v13 = midpoint2(v1, v3);
        let depth = depth - 1;

        make_triangle(out, v1, v12, v13, depth);
        make_triangle(out, v12, v2, v23, depth);
        make_triangle(out, v13, v23, v3, depth);
        make_triangle(out, v12, v23, v13, depth);
    }
}

/// Creates triangles of a geodesic to the given depth (frequency).
fn make_geodesic(depth: i32) -> Vec<Triangle> {
    let mut out = Vec::new();
    let th0 = 0.5f64.atan(); /* lat division: 26.57 deg */
    let s = std::f64::consts::PI / 5.0; /* lon division: 72 deg    */

    for i in 0..10 {
        let th1 = s * f64::from(i);
        let th2 = s * f64::from(i + 1);
        let th3 = s * f64::from(i + 2);
        let mut v1 = Ll { a: th0, o: th1 };
        let mut v2 = Ll { a: th0, o: th3 };
        let mut v3 = Ll { a: -th0, o: th2 };
        let mut vc = Ll {
            a: std::f64::consts::FRAC_PI_2,
            o: th2,
        };

        if i & 1 != 0 {
            /* north */
            make_triangle(&mut out, v1, v2, vc, depth);
            make_triangle(&mut out, v2, v1, v3, depth);
        } else {
            /* south */
            v1.a = -v1.a;
            v2.a = -v2.a;
            v3.a = -v3.a;
            vc.a = -vc.a;
            make_triangle(&mut out, v2, v1, vc, depth);
            make_triangle(&mut out, v1, v2, v3, depth);
        }
    }
    out
}

/// Link each triangle to its three neighbors.
///
/// Two triangles are neighbours if they share an edge, which is the same as
/// sharing two corners. Upstream compares every pair; this gives each distinct
/// corner a number, keys each edge by its two corner numbers, and reads the
/// pairs off the edges, which is the same answer in one pass.
fn link_neighbors(triangles: &mut [Triangle]) {
    // Corners are computed by different routes and land a hair apart, so they
    // are matched on a grid coarse enough to swallow that and far finer than
    // the gap between two real corners.
    const GRID: f32 = 1e4;
    let mut ids: HashMap<[i64; 3], u32> = HashMap::new();
    let mut corners = Vec::with_capacity(triangles.len() * 3);
    for t in triangles.iter() {
        let mut c = [0u32; 3];
        for (k, p) in t.p.iter().enumerate() {
            let key = [
                (p[0] * GRID).round() as i64,
                (p[1] * GRID).round() as i64,
                (p[2] * GRID).round() as i64,
            ];
            let next = ids.len() as u32;
            c[k] = *ids.entry(key).or_insert(next);
        }
        corners.push(c);
    }

    let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, c) in corners.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (c[k], c[(k + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }

    // Read the edges back in triangle order rather than walking the map, so
    // which neighbour is which does not depend on the hash. It matters: the
    // shockwave reaches a triangle by whichever of its neighbours got there
    // first, so a different order is a different picture.
    for (i, c) in corners.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (c[k], c[(k + 1) % 3]);
            let sharing = &edges[&(a.min(b), a.max(b))];
            let other = sharing.iter().copied().find(|&j| j != i);
            triangles[i].neighbors[k] = other.unwrap_or(i);
        }
    }

    for t in triangles.iter_mut() {
        t.altitude = 60.0; /* Fall in from space */
    }
}

impl Splodesic {
    /// Add thrust to the triangle, and propagate some of that to its neighbors.
    fn add_thrust(&mut self, i: usize, thrust: f32) {
        if self.triangles[i].hit {
            return;
        }
        self.triangles[i].hit = true;
        self.triangles[i].velocity += thrust;

        /* Eyeballed this to look roughly the same at various depths. Eh. */
        let dampen = match self.depth {
            0 => 0.5,
            1 => 0.7,
            2 => 0.9,
            3 => 0.98,
            4 => 0.985,
            _ => 0.993,
        };

        let thrust = thrust * dampen;
        if thrust > 0.1 {
            let n = self.triangles[i].neighbors;
            self.add_thrust(n[0], thrust);
            self.add_thrust(n[1], thrust);
            self.add_thrust(n[2], thrust);
        }
    }

    fn tick_triangles(&mut self) {
        let gravity = 0.1;

        /* Compute new velocities. */
        for i in 0..self.triangles.len() {
            if self.triangles[i].thrust > 0.0 {
                self.add_thrust(i, self.triangles[i].thrust);
                let t = &mut self.triangles[i];
                t.thrust_duration -= 1;
                if t.thrust_duration <= 0 {
                    t.thrust_duration = 0;
                    t.thrust = 0.0;
                }
            }
        }

        /* Apply new velocities. */
        for t in &mut self.triangles {
            t.altitude += t.velocity;
            t.velocity -= gravity;
            if t.altitude < 0.0 {
                t.velocity = 0.0;
                t.altitude = 0.0;
            }
            t.hit = false; /* Clear for next time */
        }

        /* Add eruptions. */
        if frand(1.0 / self.speed) < 0.2 {
            let n = random_below(self.triangles.len() as i32) as usize;
            let t = &mut self.triangles[n];
            t.thrust += gravity * 1.5;
            t.thrust_duration = 1 + bellrand(16.0) as i32;
        }

        self.ccolor += 1;
        if self.ccolor >= self.colors.len() {
            self.ccolor = 0;
        }
    }

    fn draw_triangles(&self, g: &mut Gl) {
        let wire = self.wireframe;
        let n = self.colors.len();
        let c0 = self.ccolor.min(n - 1);
        let c1 = (c0 + n / 2) % n;
        let rgb = |c: &XColor| {
            [
                f32::from(c.red) / 65536.0,
                f32::from(c.green) / 65536.0,
                f32::from(c.blue) / 65536.0,
                1.0,
            ]
        };
        let c = rgb(&self.colors[c0]);

        if wire {
            g.glx.color4f(c[0], c[1], c[2], c[3]);
        } else {
            g.glx.material_ambient_diffuse(c);
            // The inside is the other side of the colourmap from the outside.
            g.glx.material_back_ambient_diffuse(rgb(&self.colors[c1]));
        }

        g.glx.front_face_cw(false);
        for t in &self.triangles {
            let a = t.altitude * 0.25;
            g.glx.push_matrix();

            // The middle of the triangle, which is both the direction it flies
            // in and, unnormalised, its normal.
            let c = [
                t.p[0][0] + t.p[1][0] + t.p[2][0],
                t.p[0][1] + t.p[1][1] + t.p[2][1],
                t.p[0][2] + t.p[1][2] + t.p[2][2],
            ];
            if a > 0.0 {
                g.glx
                    .translate(a * c[0] / 3.0, a * c[1] / 3.0, a * c[2] / 3.0);
            }
            g.glx.begin(if wire {
                Shape::LineLoop
            } else {
                Shape::Triangles
            });
            g.glx.normal3f(c[0], c[1], c[2]);
            for p in &t.p {
                g.glx.vertex3f(p[0], p[1], p[2]);
            }
            g.glx.end();
            g.glx.pop_matrix();
        }
    }
}

impl Hack3d for Splodesic {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.depth_test(true);
        // Off, so the inside of the sphere shows through the gaps as the
        // pieces fly apart.
        g.glx.cull_face(false);
        if !self.wireframe {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }
        g.glx.clear();

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 6.0,
            (y as f32 - 0.5) * 6.0,
            (z as f32 - 0.5) * 8.0,
        );

        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(4.0, 4.0, 4.0);

        if !down {
            self.tick_triangles();
        }
        self.draw_triangles(g);

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

        let s = if g.width() < g.height() {
            g.width() as f32 / g.height() as f32
        } else {
            1.0
        };
        g.glx.scale(s, s, s);
        g.glx.clear();
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        if self.trackball.event(event, g.width(), g.height()) {
            return true;
        }
        if let XEvent::KeyPress { key } = event
            && (*key == ' ' || *key == '\t')
        {
            self.colors = make_smooth_colormap(1024);
            return true;
        }
        false
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let spin = g.res.bool("spin");
    let depth = g.res.int("freq").clamp(0, 5);

    let spin_speed = 0.5;
    let wander_speed = 0.005;
    let spin_accel = 1.0;

    let mut triangles = make_geodesic(depth);
    link_neighbors(&mut triangles);

    let mut st = Splodesic {
        rot: Rotator::new(
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            if spin { spin_speed } else { 0.0 },
            spin_accel,
            if g.res.bool("wander") {
                wander_speed
            } else {
                0.0
            },
            true,
        ),
        trackball: Trackball::new(),
        triangles,
        colors: make_smooth_colormap(1024),
        ccolor: 0,
        speed: g.res.float("speed").max(0.001),
        depth,
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    if !wire {
        // After the reshape, so the light is fixed to the camera. Its specular
        // is red, which is what puts the hot edge on a piece as it turns over.
        g.glx.light_position(0, 4.0, 1.4, 1.1, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [1.0, 0.2, 0.2, 1.0]);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(10.0);
    }

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:        30000",
    "*showFPS:      False",
    "*wireframe:    False",
    "*suppressRotationAnimation: True",
    "*spin:         True",
    "*wander:       True",
    "*speed:        1.0",
    "*freq:         4",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Eruption frequency", 0.01, 5.0, 0.01, 2, "1.0"),
    Opt::spin("freq", "Depth", 0.0, 5.0, "4"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "splodesic",
    label: "Splodesic",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2016",
        video: Some("https://www.youtube.com/watch?v=pwpTs1pEQmM"),
        blurb: "A geodesic sphere experiences a series of eruptions.",
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

    /// The geodesic is twenty faces subdivided four ways per level, and every
    /// corner of it is on the unit sphere. That last part is the whole trick:
    /// the midpoint of two corners is inside the sphere, and turning it back
    /// into an angle is what pushes it out onto the surface.
    #[test]
    fn a_geodesic_is_a_sphere() {
        for depth in 0..4 {
            let ts = make_geodesic(depth);
            assert_eq!(ts.len(), 20 * 4usize.pow(depth as u32));
            for t in &ts {
                for p in &t.p {
                    let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                    assert!((r - 1.0).abs() < 1e-5, "{p:?} is {r} from the middle");
                }
            }
        }
    }

    /// Every triangle has exactly three neighbours, and neighbouring is
    /// mutual. Upstream aborts if it is not, because the shockwave has nowhere
    /// to go otherwise.
    #[test]
    fn every_triangle_has_three_neighbours() {
        for depth in 0..4 {
            let mut ts = make_geodesic(depth);
            link_neighbors(&mut ts);
            for (i, t) in ts.iter().enumerate() {
                let mut n = t.neighbors;
                n.sort_unstable();
                assert!(n[0] != n[1] && n[1] != n[2], "{i} has a repeated neighbour");
                for &j in &t.neighbors {
                    assert_ne!(j, i, "{i} is its own neighbour");
                    assert!(
                        ts[j].neighbors.contains(&i),
                        "{i} claims {j}, which does not claim it back"
                    );
                }
            }
        }
    }

    /// A neighbour shares two corners, which is what makes the shockwave
    /// spread over the surface rather than jump across it.
    #[test]
    fn neighbours_share_an_edge() {
        let mut ts = make_geodesic(2);
        link_neighbors(&mut ts);
        for t in &ts {
            for &j in &t.neighbors {
                let shared =
                    t.p.iter()
                        .filter(|a| {
                            ts[j].p.iter().any(|b| {
                                (a[0] - b[0]).abs() < 1e-4
                                    && (a[1] - b[1]).abs() < 1e-4
                                    && (a[2] - b[2]).abs() < 1e-4
                            })
                        })
                        .count();
                assert_eq!(shared, 2, "a neighbour has to share exactly one edge");
            }
        }
    }

    /// The pieces start high and fall in, and an eruption throws some of them
    /// back out again. Both halves have to happen or it is a still life.
    #[test]
    fn it_falls_in_and_then_erupts() {
        let mut r = start(StartArgs::new(640, 480, "freq=2", 20260811));
        r.step();
        // A triangle's altitude is entirely in its matrix, so its vertices are
        // on the unit sphere however far it has flown, and where the matrix
        // puts the origin is how far out it is. At rest every one of them is
        // at the middle, so the spread between them is the eruption itself.
        let reach = |r: &Runner3d| {
            let f = r.frame();
            let os: Vec<[f32; 3]> = f
                .batches
                .iter()
                .map(|b| b.modelview.transform([0.0, 0.0, 0.0]))
                .collect();
            let n = os.len().max(1) as f32;
            let mid = [
                os.iter().map(|o| o[0]).sum::<f32>() / n,
                os.iter().map(|o| o[1]).sum::<f32>() / n,
                os.iter().map(|o| o[2]).sum::<f32>() / n,
            ];
            os.iter()
                .map(|o| {
                    ((o[0] - mid[0]).powi(2) + (o[1] - mid[1]).powi(2) + (o[2] - mid[2]).powi(2))
                        .sqrt()
                })
                .fold(0.0f32, f32::max)
        };
        let start_reach = reach(&r);
        assert!(start_reach > 10.0, "they start far out and fall in");

        // Long enough to land, which they do all together.
        let mut landed = f32::MAX;
        for _ in 0..200 {
            r.step();
            landed = landed.min(reach(&r));
        }
        assert!(
            landed < start_reach / 10.0,
            "{landed} is no closer than {start_reach}"
        );

        // And long enough for an eruption to throw some of them back out.
        let mut peak: f32 = 0.0;
        for _ in 0..600 {
            r.step();
            peak = peak.max(reach(&r));
        }
        assert!(peak > landed + 0.1, "nothing ever erupted");
    }
}
