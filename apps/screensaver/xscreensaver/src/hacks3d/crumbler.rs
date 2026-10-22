//! Port of `hacks/glx/crumbler.c`.
//!
//! ```text
//! crumbler, Copyright © 2018-2025 Jamie Zawinski <jwz@jwz.org>
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
//! A ball breaks into pieces, all but one of the pieces fly away, and the one
//! that is left grows to fill the screen and breaks again. It never repeats
//! and it never gets anywhere.
//!
//! There is no geometry here, only points. A chunk is a cloud of a few
//! thousand random points, and what you see is their convex hull, worked out
//! by [`crate::runtime::quickhull`]. Breaking a chunk in pieces is a Voronoi
//! partition done the cheapest possible way: pick N of its own points at
//! random as seeds, and give every other point to the seed it is nearest to.
//! The pieces are then the hulls of those N sets, and they fit together
//! exactly because a Voronoi cell is convex and the cells tile the parent.
//!
//! The piece that is kept has a problem, which is that it has a Nth of the
//! points it started with and would get coarser every time round. So before
//! it is drawn again it is *padded*: new points are added on the line between
//! two existing ones, which is inside the hull by definition and so cannot
//! change the shape, only the number of points there are to split next time.
//!
//! The five states are a loop. IDLE is the piece sitting still; SPLIT is the
//! moment it comes apart, drawn with the whole old piece swelling and fading
//! over the new ones; PAUSE is the pieces holding their places; FLEE is the
//! unwanted pieces sliding outwards along their own directions from the
//! centre while the kept one slides in; ZOOM is that one growing back to full
//! size. Each state's speed is different, and the transition is what does the
//! arithmetic: the animation shows where a piece is going, and the state
//! change puts it there for real.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::easing::{Ease, ease};
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::quickhull::{Triangle, Vertex, quickhull3d};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Idle,
    Split,
    Pause,
    Flee,
    Zoom,
}

impl State {
    /// `bp->state = (bp->state + 1) % (ZOOM + 1)`.
    fn next(self) -> State {
        match self {
            State::Idle => State::Split,
            State::Split => State::Pause,
            State::Pause => State::Flee,
            State::Flee => State::Zoom,
            State::Zoom => State::Idle,
        }
    }
}

struct Chunk {
    /// Interior point cloud.
    verts: Vec<Vertex>,
    /// How many of the points were there before the last padding, which is
    /// where the wireframe changes colour.
    onverts: usize,
    /// Enclosing box.
    min: Vertex,
    max: Vertex,
    mid: Vertex,
    /// Midpoint as normalized vector from origin.
    vec: Vertex,
    /// Upstream compiles the hull into a display list; here it is kept as the
    /// triangles quickhull handed back, which is the same thing replayed.
    tris: Vec<Triangle>,
    color: usize,
    color_shift: i32,
}

/// `RANDSIGN()`.
fn randsign() -> i32 {
    if random() & 1 != 0 { 1 } else { -1 }
}

/// Create a roughly spherical cloud of N random points.
fn make_point_cloud(nverts: usize) -> Vec<Vertex> {
    let mut verts = Vec::with_capacity(nverts);
    while verts.len() < nverts {
        let v = Vertex::new(0.5 - frand(1.0), 0.5 - frand(1.0), 0.5 - frand(1.0));
        if v.x * v.x + v.y * v.y + v.z * v.z < 0.25 {
            verts.push(v);
        }
    }
    verts
}

impl Chunk {
    fn new(verts: Vec<Vertex>, color: usize) -> Chunk {
        let mut c = Chunk {
            verts,
            onverts: 0,
            min: Vertex::default(),
            max: Vertex::default(),
            mid: Vertex::default(),
            vec: Vertex::default(),
            tris: Vec::new(),
            color,
            color_shift: 1 + (random() % 3) as i32 * randsign(),
        };
        c.render();
        c
    }

    /// `render_chunk`: the enclosing box, the midpoint, and the hull.
    fn render(&mut self) {
        self.min = Vertex::new(999999.0, 999999.0, 999999.0);
        self.max = Vertex::new(-999999.0, -999999.0, -999999.0);

        for v in &self.verts {
            self.min.x = self.min.x.min(v.x);
            self.min.y = self.min.y.min(v.y);
            self.min.z = self.min.z.min(v.z);
            self.max.x = self.max.x.max(v.x);
            self.max.y = self.max.y.max(v.y);
            self.max.z = self.max.z.max(v.z);
        }

        self.mid = Vertex::new(
            (self.max.x + self.min.x) / 2.0,
            (self.max.y + self.min.y) / 2.0,
            (self.max.z + self.min.z) / 2.0,
        );

        /* midpoint as normalized vector from origin */
        let d =
            (self.mid.x * self.mid.x + self.mid.y * self.mid.y + self.mid.z * self.mid.z).sqrt();
        self.vec = Vertex::new(self.mid.x / d, self.mid.y / d, self.mid.z / d);

        let tris = quickhull3d(&self.verts);
        // Upstream treats an empty hull as having run out of memory and gives
        // up. Nothing can run out here, and the point count is guarded, so an
        // empty answer would mean a degenerate cloud; keep the old shape.
        if !tris.is_empty() {
            self.tris = tris;
        }
    }

    /// Make sure the chunk contains at least N points.
    ///
    /// As we subdivide, the number of points is reduced. This adds new points
    /// to the interior that do not affect the shape of the outer hull.
    fn pad(&mut self, min: usize) {
        if self.verts.len() >= min {
            return;
        }
        let n = self.verts.len();
        while self.verts.len() < min {
            let j0 = random() as usize % n;
            let mut j1 = j0;
            while j0 == j1 {
                j1 = random() as usize % n;
            }

            let r = 0.2 + frand(0.6);
            let along = |a: f64, b: f64| a + r * (b - a).abs() * if a > b { -1.0 } else { 1.0 };
            let (a, b) = (self.verts[j0], self.verts[j1]);
            self.verts.push(Vertex::new(
                along(a.x, b.x),
                along(a.y, b.y),
                along(a.z, b.z),
            ));
        }
        self.onverts = n;
    }

    /// `draw_chunk`, less the colour, which the caller sets.
    fn draw(&self, g: &mut Gl, wire: bool, mono: [f32; 4]) {
        if wire {
            // Upstream draws each triangle as its own GL_LINE_LOOP. Three
            // segments say the same thing and go out as one block rather than
            // one for every triangle.
            g.glx.begin(Shape::Lines);
            for t in &self.tris {
                for i in 0..3 {
                    for v in [t.vertices[i], t.vertices[(i + 1) % 3]] {
                        g.glx.vertex3f(v.x as f32, v.y as f32, v.z as f32);
                    }
                }
            }
            g.glx.end();

            g.glx.point_size(1.0);
            g.glx.color3f(0.0, 1.0, 0.0);
            g.glx.begin(Shape::Points);
            for (i, v) in self.verts.iter().enumerate() {
                if i > 0 && i == self.onverts {
                    g.glx.end();
                    g.glx.color3f(1.0, 0.0, 0.0);
                    g.glx.begin(Shape::Points);
                }
                g.glx.vertex3f(v.x as f32, v.y as f32, v.z as f32);
            }
            g.glx.end();
            g.glx.color4f(mono[0], mono[1], mono[2], mono[3]);
            return;
        }

        g.glx.begin(Shape::Triangles);
        for t in &self.tris {
            g.glx
                .normal3f(t.normal.x as f32, t.normal.y as f32, t.normal.z as f32);
            for v in t.vertices {
                g.glx.vertex3f(v.x as f32, v.y as f32, v.z as f32);
            }
        }
        g.glx.end();
    }
}

struct Crumbler {
    rot: Rotator,
    trackball: Trackball,
    state: State,
    tick: f32,
    chunks: Vec<Chunk>,
    ghost: Option<Chunk>,
    colors: Vec<XColor>,
    speed: f32,
    fracture: i32,
    do_wander: bool,
    wire: bool,
}

impl Crumbler {
    /// Returns a list of N new chunks.
    ///
    /// Pick N key-points from the cloud, create N new chunks, and put each of
    /// the old points in the chunk whose key-point it is closest to.
    fn split_chunk(&mut self, from: usize, nchunks: usize) -> Vec<Chunk> {
        let c = &self.chunks[from];
        let mut retries = 0;
        loop {
            // Fill keys with random numbers that are not duplicates.
            let mut keys: Vec<usize> = Vec::with_capacity(nchunks);
            let mut colors: Vec<usize> = Vec::with_capacity(nchunks);
            let ncolors = self.colors.len();
            for _ in 0..nchunks {
                loop {
                    let k = random() as usize % c.verts.len();
                    if !keys.contains(&k) {
                        keys.push(k);
                        break;
                    }
                }
                colors.push((c.color + (random() as usize % (1 + (ncolors / 3)))) % ncolors);
            }

            /* Add the verts to the approprate chunks */
            let mut sets: Vec<Vec<Vertex>> = vec![Vec::new(); nchunks];
            for v0 in &c.verts {
                let mut target_chunk = 0;
                let mut target_d2 = 9999999.0;
                for (j, &k) in keys.iter().enumerate() {
                    let v1 = c.verts[k];
                    let x = v1.x - v0.x;
                    let y = v1.y - v0.y;
                    let z = v1.z - v0.z;
                    let d2 = x * x + y * y + z * z;
                    if d2 < target_d2 {
                        target_d2 = d2;
                        target_chunk = j;
                    }
                }
                sets[target_chunk].push(*v0);
            }

            // It is possible that the keys we have chosen have resulted in one
            // or more cells that have 3 or fewer points in them. If that's the
            // case, re-randomize.
            if sets.iter().any(|s| s.len() <= 3) {
                retries += 1;
                if retries > 100 {
                    // Upstream calls this unsplittable and aborts. There is
                    // nothing to abort to, so the chunk stays whole and the
                    // next tick tries again.
                    return Vec::new();
                }
                continue;
            }

            let nverts = c.verts.len();
            let mut out = Vec::with_capacity(nchunks);
            for (i, verts) in sets.into_iter().enumerate() {
                let mut c2 = Chunk::new(verts, colors[i]);
                if i == 0 {
                    /* The one we're gonna keep */
                    c2.pad(nverts);
                    c2.render();
                }
                out.push(c2);
            }
            return out;
        }
    }

    fn tick_crumbler(&mut self) {
        if self.trackball.button_down() {
            return;
        }

        let ts = match self.state {
            State::Idle => 0.02,
            State::Split => 0.01,
            State::Pause => 0.008,
            State::Flee => 0.005,
            State::Zoom => 0.03,
        };

        self.tick += ts * self.speed;

        if self.tick < 1.0 {
            return;
        }

        self.tick = 0.0;
        self.state = self.state.next();

        match self.state {
            State::Idle => {
                // We already animated it zooming to full size. Now make it
                // real.
                let c = &mut self.chunks[0];
                let x = c.max.x - c.min.x;
                let y = c.max.y - c.min.y;
                let z = c.max.z - c.min.z;
                let s = 1.0 / x.max(y.max(z));

                for v in &mut c.verts {
                    v.x *= s;
                    v.y *= s;
                    v.z *= s;
                }

                // Re-render it to move the verts in the display list too.
                // This also recomputes min, max and mid.
                c.render();
            }

            State::Split => {
                let frac = if self.fracture >= 2 {
                    self.fracture as usize
                } else {
                    2 + (2 * (random() as usize % 5))
                };
                let chunks = self.split_chunk(0, frac);
                if chunks.is_empty() {
                    // Nothing was splittable; stay whole and try again.
                    self.state = State::Idle;
                    return;
                }
                self.ghost = Some(self.chunks.remove(0));
                self.chunks = chunks;
            }

            State::Pause => {}

            State::Flee => {
                self.ghost = None;
            }

            State::Zoom => {
                self.chunks.truncate(1);

                // We already animated the remaining chunk moving toward the
                // origin. Make it real.
                let c = &mut self.chunks[0];
                let mid = c.mid;
                for v in &mut c.verts {
                    v.x -= mid.x;
                    v.y -= mid.y;
                    v.z -= mid.z;
                }

                // Re-render it to move the verts in the display list too.
                // This also recomputes min, max and mid (now 0).
                c.render();
            }
        }
    }

    fn draw_chunk(&mut self, g: &mut Gl, which: Option<usize>, alpha: f32) {
        let ncolors = self.colors.len();
        let c = match which {
            Some(i) => &mut self.chunks[i],
            None => self.ghost.as_mut().expect("a ghost"),
        };
        let x = &self.colors[c.color];
        let color = [
            x.red as f32 / 65536.0,
            x.green as f32 / 65536.0,
            x.blue as f32 / 65536.0,
            alpha,
        ];
        g.glx.color4f(color[0], color[1], color[2], color[3]);
        g.glx.material_ambient_diffuse(color);

        c.color = (c.color as i32 + c.color_shift).rem_euclid(ncolors as i32) as usize;

        let wire = self.wire;
        c.draw(g, wire, color);
    }
}

impl Hack3d for Crumbler {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        self.tick_crumbler();

        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.clear();

        g.glx.push_matrix();

        {
            let down = self.trackball.button_down();
            let (x, y, z) = self.rot.position(!down);
            g.glx.translate(
                (x as f32 - 0.5) * 8.0,
                (y as f32 - 0.5) * 8.0,
                (z as f32 - 0.5) * 15.0,
            );

            let m = self.trackball.matrix();
            g.glx.mult_matrix(m);

            let (x, y, z) = self.rot.rotation(!down);
            g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
            g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
            g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);
        }

        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(128.0);

        let s = if self.do_wander { 10.0 } else { 13.0 };
        g.glx.scale(s, s, s);

        let mut alpha = 1.0;
        for i in 0..self.chunks.len() {
            g.glx.push_matrix();

            match self.state {
                State::Flee => {
                    let r = ease(Ease::InOutSine, self.tick as f64) as f32;
                    // Move everybody toward the origin, so that chunk #0 ends
                    // up centered there.
                    let mid = self.chunks[i].mid;
                    g.glx
                        .translate(-r * mid.x as f32, -r * mid.y as f32, -r * mid.z as f32);
                    if i != 0 {
                        // Move this chunk away from the center, along a vector
                        // from the origin to its midpoint.
                        let d2 = r * 6.0;
                        let v = self.chunks[i].vec;
                        g.glx
                            .translate(v.x as f32 * d2, v.y as f32 * d2, v.z as f32 * d2);
                        alpha = 1.0 - r;
                    }
                }

                State::Zoom => {
                    let c = &self.chunks[0];
                    let x = c.max.x - c.min.x;
                    let y = c.max.y - c.min.y;
                    let z = c.max.z - c.min.z;
                    let size0 = x.max(y.max(z)) as f32;
                    let size1 = 1.0;
                    let r = 1.0 - ease(Ease::InOutSine, self.tick as f64) as f32;
                    let s = 1.0 / (size0 + r * (size1 - size0));
                    g.glx.scale(s, s, s);
                }

                _ => {}
            }

            self.draw_chunk(g, Some(i), alpha);
            g.glx.pop_matrix();
        }

        /* Draw the old one, fading out. */
        if !self.wire && self.state == State::Split && self.ghost.is_some() {
            let alpha = 1.0;
            let mut s = 2.0 * ease(Ease::InOutSine, (1.0 - self.tick) as f64 / 2.0) as f32;
            s *= 1.01;
            g.glx.scale(s, s, s);
            self.draw_chunk(g, None, alpha);
        }

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
        self.trackball.event(event, g.width(), g.height())
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let wire = g.res.bool("wireframe");
    let speed = g.res.float("speed") as f32;
    let density = g.res.float("density") as f32;
    let do_spin = g.res.bool("spin");
    let do_wander = g.res.bool("wander");

    if !wire {
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.depth_test(true);
        g.glx.cull_face(true);

        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
        g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
        g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);

        g.glx.blend(Blend::Alpha);
    }

    let spin_speed = 0.5 * speed;
    let spin_accel = 0.3;
    let wander_speed = 0.01 * speed;

    let mut colors = make_smooth_colormap(1024);
    /* brighter colors, please... */
    for c in &mut colors {
        let brighten = |f: u16| (65535.0 * (0.3 + 0.7 * (f as f64 / 65535.0))) as u16;
        c.red = brighten(c.red);
        c.green = brighten(c.green);
        c.blue = brighten(c.blue);
    }

    // Upstream asks for 4500 points times the density and halves its way down
    // until the allocation succeeds, which on a system that overcommits is
    // always the first try. Nothing allocates a square of the point count
    // here, so the count is taken as asked.
    let nverts = ((4500.0 * density) as usize).max(8);
    let mut verts = make_point_cloud(nverts);

    /* Let's shrink it to a point then zoom in. */
    for v in &mut verts {
        v.x /= 500.0;
        v.y /= 500.0;
        v.z /= 500.0;
    }

    let mut st = Crumbler {
        rot: Rotator::new(
            if do_spin { spin_speed } else { 0.0 } as f64,
            if do_spin { spin_speed } else { 0.0 } as f64,
            if do_spin { spin_speed } else { 0.0 } as f64,
            spin_accel,
            if do_wander { wander_speed } else { 0.0 } as f64,
            true,
        ),
        trackball: Trackball::new(),
        state: State::Zoom,
        tick: 0.0,
        chunks: Vec::new(),
        ghost: None,
        colors,
        speed,
        fracture: g.res.int("fracture"),
        do_wander,
        wire,
    };
    st.chunks.push(Chunk::new(verts, 0));

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
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
    "*density:      1.0",
    "*fracture:     0",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("speed", "Speed", 0.05, 2.0, 0.05, 2, "1.0"),
    Opt::slider("density", "Polygons", 0.2, 5.0, 0.1, 1, "1.0"),
    Opt::slider("fracture", "Fractures", 0.0, 20.0, 1.0, 0, "0"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "crumbler",
    label: "Crumbler",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=oERz1IPluYQ"),
        blurb: "Randomly subdivides a ball into voronoi chunks, then further \
                subdivides one of the remaining pieces.",
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

    /// A saver in a known state, without the resource plumbing `init` walks
    /// through.
    fn crumbler(nverts: usize, fracture: i32) -> Crumbler {
        crate::runtime::ya_rand_init(20260812);
        let mut verts = make_point_cloud(nverts);
        for v in &mut verts {
            v.x /= 500.0;
            v.y /= 500.0;
            v.z /= 500.0;
        }
        Crumbler {
            rot: Rotator::new(0.5, 0.5, 0.5, 0.3, 0.01, true),
            trackball: Trackball::new(),
            state: State::Zoom,
            tick: 0.0,
            chunks: vec![Chunk::new(verts, 0)],
            ghost: None,
            colors: make_smooth_colormap(1024),
            speed: 1.0,
            fracture,
            do_wander: true,
            wire: false,
        }
    }

    #[test]
    fn the_pieces_add_up_to_the_whole() {
        let mut st = crumbler(2000, 6);
        let parent = st.chunks[0].verts.len();
        let pieces = st.split_chunk(0, 6);
        assert_eq!(pieces.len(), 6);

        // Every point of the parent ends up in exactly one piece, and the
        // piece kept is padded back to the parent's count.
        let mut total = 0;
        for (i, p) in pieces.iter().enumerate() {
            assert!(p.verts.len() > 3, "piece {i} has {}", p.verts.len());
            assert!(!p.tris.is_empty(), "piece {i} has no hull");
            total += if i == 0 { p.onverts } else { p.verts.len() };
        }
        assert_eq!(total, parent);
        assert_eq!(pieces[0].verts.len(), parent);

        // A piece is smaller than the parent it came from, and its direction
        // from the centre is a unit vector.
        let span = |c: &Chunk| {
            (c.max.x - c.min.x)
                .max(c.max.y - c.min.y)
                .max(c.max.z - c.min.z)
        };
        let whole = span(&st.chunks[0]);
        for p in &pieces {
            assert!(span(p) < whole, "{} !< {whole}", span(p));
            let d = (p.vec.x * p.vec.x + p.vec.y * p.vec.y + p.vec.z * p.vec.z).sqrt();
            assert!((d - 1.0).abs() < 1e-9, "{d}");
        }
    }

    #[test]
    fn the_padding_stays_inside_the_hull() {
        crate::runtime::ya_rand_init(20260812);
        let mut c = Chunk::new(make_point_cloud(400), 0);
        let before = c.tris.len();
        let (min, max) = (c.min, c.max);
        c.pad(1600);
        assert_eq!(c.verts.len(), 1600);
        assert_eq!(c.onverts, 400);
        c.render();
        // The box has not grown, so the padding is all interior, and the hull
        // is about as complex as it was.
        assert!(c.min.x >= min.x - 1e-9 && c.max.x <= max.x + 1e-9);
        assert!(c.min.y >= min.y - 1e-9 && c.max.y <= max.y + 1e-9);
        assert!(c.min.z >= min.z - 1e-9 && c.max.z <= max.z + 1e-9);
        assert!(c.tris.len() < before * 3, "{} vs {before}", c.tris.len());
    }

    #[test]
    fn it_runs_all_the_way_round_the_five_states() {
        let mut g = Gl::for_test(640, 480);
        let mut st = crumbler(1200, 6);
        st.reshape(&mut g, 640, 480);

        let mut seen = Vec::new();
        let mut batches = 0;
        for _ in 0..3000 {
            g.glx.start_frame(640, 480);
            st.draw(&mut g);
            batches = batches.max(g.glx.frame().batches.len());
            if !seen.contains(&st.state) {
                seen.push(st.state);
            }
            assert!(!g.glx.frame().batches.is_empty());
        }
        assert_eq!(seen.len(), 5, "{seen:?}");
        // One block per piece, and never more pieces than the fracture.
        assert!(batches <= 8, "{batches}");
    }
}
