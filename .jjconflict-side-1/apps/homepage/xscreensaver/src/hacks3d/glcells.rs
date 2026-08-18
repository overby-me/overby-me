/*
 * glcells.c
 *
 * Copyright (c) 2007 Matthias Toussaint
 *
 * Permission to use, copy, modify, distribute, and sell this software and its
 * documentation for any purpose is hereby granted without fee, provided that
 * the above copyright notice appear in all copies and that both that
 * copyright notice and this permission notice appear in supporting
 * documentation.  No representations are made about the suitability of this
 * software for any purpose.  It is provided "as is" without express or
 * implied warranty.
 */

//! Port of `hacks/glx/glcells.c`.
//!
//! Cells growing on a petri dish. It is a little simulation rather than an
//! animation: the dish is a grid of food, a cell eats what it is standing on,
//! ages, shoves itself away from its nearest neighbour, and when it is old
//! enough and has room it splits in two. When the colony has filled the dish
//! or starved to death the dish is reseeded.
//!
//! A cell is drawn as a wrinkled half-sphere. There is one sphere, made once
//! by subdividing a half-dodecahedron, and ten copies of it wrinkled by
//! increasing amounts; a cell picks the one that matches how much energy it has
//! left, so a healthy cell is smooth and a dying one is crumpled.
//!
//! This was deferred on volume, and the measurement is worth keeping. A cell at
//! upstream's quality of three is a half-dodecahedron subdivided three times,
//! 10 * 4^3 = 640 triangles, so a full colony of eight hundred is 1.54 million
//! vertices a frame. One step down is 160 triangles and 384 thousand, which is
//! comfortable, and the quality is an internal constant that upstream's own
//! configuration file does not offer. The colony is what the saver is about
//! rather than how round any one cell is, so that is the setting lowered here,
//! and the knob is offered so it can be put back.
//!
//! The other half of the answer is that every cell is the same shape under a
//! different matrix, which would be eight hundred draw calls. They are instead
//! transformed on the way out and drawn as one block, so a frame is three calls
//! however many cells there are.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape, TexEnv};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, random, random_below,
};
use std::collections::HashMap;

/// How many wrinkled copies of the sphere there are.
const NUM_CELL_SHAPES: usize = 10;
const TEX_SIZE: i32 = 64;

/// One cell of the colony.
#[derive(Clone, Copy, Default)]
struct Cell {
    /* position */
    x: f64,
    y: f64,
    /* movement vector */
    vx: f64,
    vy: f64,
    age: i32,
    /* minimum distance to other cells */
    min_dist: f64,
    /* health */
    energy: i32,
    /* random rot, so they don't look all the same */
    rotation: f64,
    /* current size of cell */
    radius: f64,
    /* current growth rate. might be <1.0 while dividing, >1.0 when finished
    dividing and food is available and 1.0 when grown up */
    growth: f64,
}

/// A shape with a normal at every vertex.
struct Smooth {
    vertex: Vec<[f32; 3]>,
    normal: Vec<[f32; 3]>,
    triangle: Vec<[usize; 3]>,
}

/// The half-dodecahedron every cell is made from, subdivided.
///
/// Upstream subdivides by splitting every triangle into four and then welding
/// the duplicated vertices back together with a search over the whole list,
/// which is where its comment about the cost comes from. The same shape falls
/// out of remembering each edge's midpoint as it is made, so that the two
/// triangles either side of an edge get the same vertex, and that is what is
/// done here: same vertices, same triangles, same shared normals.
fn create_sphere(divisions: u32) -> (Vec<[f32; 3]>, Vec<[usize; 3]>) {
    /* create vertexes for dodecaedron */
    let a_step = std::f64::consts::PI / 3.0;
    let mut vertex: Vec<[f32; 3]> = Vec::with_capacity(9);
    let mut a: f64 = 0.0;
    for _ in 0..6 {
        vertex.push([a.sin() as f32, -a.cos() as f32, 0.0]);
        a += a_step;
    }
    let mut a: f64 = -60.0 / 180.0 * std::f64::consts::PI;
    let e = 58.2825 / 180.0 * std::f64::consts::PI;
    for _ in 6..9 {
        vertex.push([
            (a.sin() * e.cos()) as f32,
            (-a.cos() * e.cos()) as f32,
            -e.sin() as f32,
        ]);
        a += 2.0 * a_step;
    }

    /* create triangles */
    const VI: [usize; 30] = [
        0, 7, 1, 1, 7, 2, 2, 8, 3, 3, 8, 4, 4, 6, 5, 5, 6, 0, 0, 6, 7, 2, 7, 8, 4, 8, 6, 6, 8, 7,
    ];
    let mut triangle: Vec<[usize; 3]> = VI.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();

    /* subdivide as specified */
    for _ in 0..divisions {
        let mut midpoints: HashMap<(usize, usize), usize> = HashMap::new();
        let mut next = Vec::with_capacity(triangle.len() * 4);
        for t in &triangle {
            let mut mid = [0usize; 3];
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let key = (a.min(b), a.max(b));
                mid[k] = *midpoints.entry(key).or_insert_with(|| {
                    let (va, vb) = (vertex[a], vertex[b]);
                    vertex.push([
                        0.5 * (va[0] + vb[0]),
                        0.5 * (va[1] + vb[1]),
                        0.5 * (va[2] + vb[2]),
                    ]);
                    vertex.len() - 1
                });
            }
            next.push([t[0], mid[0], mid[2]]);
            next.push([mid[0], t[1], mid[1]]);
            next.push([mid[2], mid[1], t[2]]);
            next.push([mid[0], mid[1], mid[2]]);
        }
        triangle = next;
    }

    /* normalize vertexes */
    for v in &mut vertex {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if l > 0.0 {
            for c in v.iter_mut() {
                *c /= l;
            }
        }
    }
    (vertex, triangle)
}

/// `create_ObjectSmooth`: a normal per vertex, averaged over the faces that
/// meet there.
fn smooth_normals(vertex: &[[f32; 3]], triangle: &[[usize; 3]]) -> Vec<[f32; 3]> {
    let mut normal = vec![[0.0f32; 3]; vertex.len()];
    for t in triangle {
        let (a, b, c) = (vertex[t[0]], vertex[t[1]], vertex[t[2]]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for &i in t {
            for k in 0..3 {
                normal[i][k] += n[k];
            }
        }
    }
    for n in &mut normal {
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if l > 0.0 {
            for c in n.iter_mut() {
                *c /= l;
            }
        }
    }
    normal
}

struct GlcellsState {
    width: f64,
    height: f64,
    /* we scale content with window size */
    screen_scale: f64,
    cell: Vec<Cell>,
    color: [f32; 4],
    /* cell radius */
    radius: f64,
    /* min distance from neighbours for forking */
    move_dist: f64,
    max_cells: usize,
    num_seeds: usize,
    keep_old_cells: bool,
    /* min age for division */
    divide_age: i32,
    minfood: i32,
    maxfood: i32,
    /* pause at end (all cells dead) */
    pause: i32,
    pause_counter: i32,
    wire: bool,
    /* the ten wrinkled copies of the sphere */
    shapes: Vec<Smooth>,
    /* our petri dish (e.g. screen) */
    food: Vec<i32>,
    texture: Option<u32>,
}

impl GlcellsState {
    /// `can_divide`: old enough, big enough, fed, and with room.
    fn can_divide(&self, cell: &Cell) -> bool {
        cell.min_dist > self.move_dist
            && cell.age >= self.divide_age
            && cell.radius > 0.99 * self.radius
            && cell.energy > 0
    }

    /// How wide the dish of food is, in the four-pixel cells it is divided
    /// into. Never nought: a window can be one pixel across, and a dish with
    /// no columns in it has nowhere to put a cell.
    fn food_width(&self) -> usize {
        ((self.width as usize) / 4).max(1)
    }

    /// `create_cells`: a fresh dish of food and a few seed cells.
    fn create_cells(&mut self) {
        let border = 200.0 * self.screen_scale;
        let w = self.width - 2.0 * border;
        let h = self.height - 2.0 * border;

        self.color = [
            0.5 + random_below(1000) as f32 * 0.0005,
            0.5 + random_below(1000) as f32 * 0.0005,
            0.5 + random_below(1000) as f32 * 0.0005,
            1.0,
        ];

        /* fill the screen with random food for our little critters */
        for f in &mut self.food {
            *f = random_interval(self.minfood, self.maxfood);
        }

        /* create the requested seed-cells */
        self.cell.clear();
        for _ in 0..self.num_seeds {
            self.cell.push(Cell {
                x: border + f64::from(random_below(w.max(1.0) as i32)),
                y: border + f64::from(random_below(h.max(1.0) as i32)),
                vx: 0.0,
                vy: 0.0,
                age: random_below(0x0f),
                min_dist: 500.0,
                energy: random_interval(5, 5 + 0x3f),
                rotation: frand01() * 360.0,
                radius: self.radius,
                growth: 1.0,
            });
        }
    }

    /// `tick`: one step of the simulation. Upstream's comment on it is "all
    /// this is rather expensive :(", and it is: every cell looks at every other
    /// to find its nearest neighbour.
    fn tick(&mut self) {
        let check_dist = 0.75 * self.move_dist;
        let grow_dist = 0.75 * self.radius;
        let adult_radius = self.radius;

        /* find number of cells capable of division and count living cells */
        let mut num_cells = 0;
        let mut num_living = 0;
        for c in &self.cell {
            if c.energy > 0 {
                num_living += 1;
            }
            if self.can_divide(c) {
                num_cells += 1;
            }
        }
        let new_num_cells = self.cell.len() + num_cells;

        /* end of simulation ? */
        if num_living == 0 || new_num_cells >= self.max_cells {
            if self.pause_counter > 0 {
                self.pause_counter -= 1;
            }
            if self.pause_counter > 0 {
                return;
            }
            self.create_cells();
            self.pause_counter = self.pause;
        } else if num_cells > 0 {
            /* any fertile candidates ? */
            let mut born = Vec::with_capacity(num_cells);
            for b in 0..self.cell.len() {
                if self.can_divide(&self.cell[b]) {
                    let c = &mut self.cell[b];
                    c.vx = f64::from(random_interval(-50, 50)) * 0.01;
                    c.vy = f64::from(random_interval(-50, 50)) * 0.01;
                    c.age = random_below(0x0f);
                    /* half energy for both plus some bonus for forking */
                    c.energy = c.energy / 2 + random_below(0x0f);
                    /* forking makes me shrink */
                    c.growth = 0.995;
                    let parent = *c;
                    born.push(Cell {
                        /* this one initially goes into the oposite direction */
                        vx: -parent.vx,
                        vy: -parent.vy,
                        /* same center */
                        x: parent.x,
                        y: parent.y,
                        age: random_below(0x0f),
                        energy: parent.energy,
                        rotation: frand01() * 360.0,
                        growth: parent.growth,
                        radius: parent.radius,
                        min_dist: 0.0,
                    });
                } else {
                    self.cell[b].vx = 0.0;
                    self.cell[b].vy = 0.0;
                }
            }
            self.cell.extend(born);
        }

        /* for each find a direction to escape */
        if self.cell.len() > 1 {
            for b in 0..self.cell.len() {
                if self.cell[b].energy <= 0 {
                    continue;
                }
                /* grow or shrink */
                self.cell[b].radius *= self.cell[b].growth;
                /* find closest neighbour */
                let mut min_dist = 100_000.0;
                let mut min_index = 0;
                for j in 0..self.cell.len() {
                    if j == b {
                        continue;
                    }
                    let dx = self.cell[b].x - self.cell[j].x;
                    let dy = self.cell[b].y - self.cell[j].y;
                    if dx.abs() < check_dist || dy.abs() < check_dist {
                        let dist = dx * dx + dy * dy;
                        if dist < min_dist {
                            min_dist = dist;
                            min_index = j;
                        }
                    }
                }
                /* escape step is away from closest normalized with distance */
                let vx = self.cell[b].x - self.cell[min_index].x;
                let vy = self.cell[b].y - self.cell[min_index].y;
                let len = (vx * vx + vy * vy).sqrt();
                if len > 0.0001 {
                    self.cell[b].vx = vx / len;
                    self.cell[b].vy = vy / len;
                }
                self.cell[b].min_dist = len;
                /* if not adult (radius too small) */
                if self.cell[b].radius < adult_radius {
                    /* if too small 60% stop shrinking */
                    if self.cell[b].radius < adult_radius * 0.6 {
                        self.cell[b].growth = 1.0;
                    }
                    /* at safe distance we start growing again */
                    if len > grow_dist && self.cell[b].energy > 30 {
                        self.cell[b].growth = 1.005;
                    }
                } else {
                    /* else keep size */
                    self.cell[b].growth = 1.0;
                }
            }
        } else if let Some(c) = self.cell.first_mut() {
            c.min_dist = 2.0 * self.move_dist;
        }

        /* now move em, snack and burn energy */
        let (w4, h4) = (self.food_width(), ((self.height as usize) / 4).max(1));
        for b in 0..self.cell.len() {
            if self.cell[b].energy <= 0 {
                continue;
            }
            /* agility depends on amount of energy */
            let fac = (f64::from(self.cell[b].energy) / 50.0).clamp(0.0, 1.0);
            self.cell[b].x += fac * (2.0 - 4.0 * frand01() + self.cell[b].vx);
            self.cell[b].y += fac * (2.0 - 4.0 * frand01() + self.cell[b].vy);

            /* get older and burn energy */
            self.cell[b].age += 1;
            self.cell[b].energy -= 1;

            /* have a snack */
            let x = ((self.cell[b].x as i64) / 4).clamp(0, w4 as i64 - 1) as usize;
            let y = ((self.cell[b].y as i64) / 4).clamp(0, h4 as i64 - 1) as usize;
            let offset = x + y * w4;
            if offset >= self.food.len() {
                continue;
            }

            /* don't eat if already satisfied */
            if self.cell[b].energy < 100 && self.food[offset] > 0 {
                self.food[offset] -= 1;
                self.cell[b].energy += 1;
                /* if you are hungry, eat more */
                if self.cell[b].energy < 50 && self.food[offset] > 0 {
                    self.food[offset] -= 1;
                    self.cell[b].energy += 1;
                }
            }
        }
    }

    /// Draw every cell of one kind as a single block.
    ///
    /// Upstream calls a display list per cell under its own matrix, which
    /// would be one draw call each. The shape is the same for all of them, so
    /// the matrix is applied on the way out instead and the whole colony goes
    /// down in one call.
    fn draw_cells(&self, g: &mut Gl, dead: bool) {
        let mut any = false;
        for c in &self.cell {
            if (c.energy > 0) == dead {
                continue;
            }
            let shape = if dead {
                NUM_CELL_SHAPES - 1
            } else {
                let fac = (f64::from(c.energy) / 50.0).clamp(0.0, 1.0);
                NUM_CELL_SHAPES - 1 - (9.0 * fac) as usize
            };
            let s = &self.shapes[shape.min(NUM_CELL_SHAPES - 1)];
            if !any {
                g.glx.begin(Shape::Triangles);
                any = true;
            }
            let (sin, cos) = c.rotation.to_radians().sin_cos();
            let (sin, cos) = (sin as f32, cos as f32);
            let r = c.radius as f32;
            let (cx, cy) = (c.x as f32, c.y as f32);
            for t in &s.triangle {
                for &i in t {
                    let v = s.vertex[i];
                    let n = s.normal[i];
                    // Turn about z, scale, then move: upstream's matrix.
                    g.glx
                        .normal3f(n[0] * cos - n[1] * sin, n[0] * sin + n[1] * cos, n[2]);
                    g.glx.vertex3f(
                        cx + r * (v[0] * cos - v[1] * sin),
                        cy + r * (v[0] * sin + v[1] * cos),
                        r * v[2],
                    );
                }
            }
        }
        if any {
            g.glx.end();
        }
    }
}

fn frand01() -> f64 {
    f64::from(random()) / f64::from(u32::MAX)
}

fn random_interval(min: i32, max: i32) -> i32 {
    let n = if max - min == 0 { 1 } else { max - min };
    min + random_below(n)
}

impl Hack3d for GlcellsState {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        self.width = f64::from(width);
        self.height = f64::from(height);
        self.screen_scale = f64::from(width) / 1600.0;

        self.radius = f64::from(g.res.int("radius").clamp(5, 200)) * self.screen_scale;
        self.move_dist = g.res.float("mindist").clamp(1.0, 3.0) * self.radius;

        g.glx.viewport(0, 0, width, height);
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        g.glx
            .ortho(0.0, width as f32, height as f32, 0.0, 200.0, 0.0);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        let n = ((width as usize) * (height as usize)) / 16;
        self.food = vec![0; n.max(1)];
    }

    fn event(&mut self, _g: &mut Gl, _event: &XEvent) -> bool {
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        if self.cell.is_empty() {
            self.create_cells();
        }
        /* life goes on... */
        self.tick();

        g.glx.clear();
        g.glx.depth_test(true);
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        // The light's *diffuse* colour is the colony's colour, which is why a
        // dish of cells is all one hue and why it changes when it reseeds.
        g.glx.light_ambient(0, [0.1, 0.1, 0.1, 1.0]);
        g.glx.light_diffuse(0, self.color);
        g.glx.light_position(0, -20.0, -10.0, -100.0, 0.0);

        if !self.wire {
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        } else {
            g.glx.lighting(false);
        }

        /* draw the dead cells if choosen */
        if self.keep_old_cells {
            self.draw_cells(g, true);
        }
        /* draw the living cells */
        self.draw_cells(g, false);

        /* draw cell nuclei */
        if !self.wire
            && let Some(tex) = self.texture
        {
            g.glx.lighting(false);
            g.glx.blend(Blend::Alpha);
            g.glx.depth_test(false);
            g.glx.texturing(true);
            g.glx.tex_env(TexEnv::Modulate);
            g.glx.bind_texture(tex);
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);

            g.glx.begin(Shape::Quads);
            for c in &self.cell {
                if !(c.energy > 0 || self.keep_old_cells) {
                    continue;
                }
                let z = -1.2 * c.radius as f32;
                let r = 0.5 * c.radius as f32;
                let (cx, cy) = (c.x as f32, c.y as f32);
                for (u, v, dx, dy) in [
                    (0.0, 0.0, -r, -r),
                    (0.0, 1.0, -r, r),
                    (1.0, 1.0, r, r),
                    (1.0, 0.0, r, -r),
                ] {
                    g.glx.tex_coord2f(u, v);
                    g.glx.vertex3f(cx + dx, cy + dy, z);
                }
            }
            g.glx.end();

            g.glx.texturing(false);
            g.glx.blend(Blend::Off);
        }

        g.res.int("delay").max(0) as u32
    }
}

/// `create_nucleus_texture`: a soft dark blob, which is the nucleus seen
/// through the cell.
fn nucleus_texture() -> Vec<u8> {
    let w2 = TEX_SIZE / 2;
    let s = (w2 * w2) as f32 / 4.0;
    let mut px = vec![0u8; 4 * (TEX_SIZE * TEX_SIZE) as usize];
    for y in 0..TEX_SIZE {
        for x in 0..TEX_SIZE {
            let r2 = ((x - w2) * (x - w2) + (y - w2) * (y - w2)) as f32;
            let v = 120.0 * (-r2 / s).exp();
            px[4 * (x + y * TEX_SIZE) as usize + 3] = v as u8;
        }
    }
    px
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let divisions = g.res.int("quality").clamp(0, 5) as u32;
    let (base, triangle) = create_sphere(divisions);

    // The wrinkles: one set of random offsets, applied at ten strengths, so
    // the ten shapes are the same cell at ten stages of decay rather than ten
    // different cells.
    let disturbance: Vec<f64> = (0..base.len()).map(|_| 0.05 - frand01() * 0.1).collect();
    let shapes: Vec<Smooth> = (0..NUM_CELL_SHAPES)
        .map(|shape| {
            let fac = shape as f64 / 10.0;
            let vertex: Vec<[f32; 3]> = base
                .iter()
                .zip(&disturbance)
                .map(|(v, d)| {
                    let m = (1.0 + fac * d) as f32;
                    [v[0] * m, v[1] * m, v[2] * m]
                })
                .collect();
            let normal = smooth_normals(&vertex, &triangle);
            Smooth {
                vertex,
                normal,
                triangle: triangle.clone(),
            }
        })
        .collect();

    let texture = {
        let id = g.glx.gen_texture();
        g.glx.bind_texture(id);
        g.glx.tex_image_2d(TEX_SIZE, TEX_SIZE, nucleus_texture());
        g.glx.tex_clamp(false);
        g.glx.tex_nearest(false);
        Some(id)
    };

    let minfood = g.res.int("minfood").clamp(0, 1000);
    let mut maxfood = g.res.int("maxfood").clamp(0, 1000);
    if maxfood < minfood {
        maxfood = minfood + 1;
    }
    let pause = g.res.int("pause").clamp(0, 400);

    let mut st = GlcellsState {
        width: 1.0,
        height: 1.0,
        screen_scale: 1.0,
        cell: Vec::new(),
        color: [1.0; 4],
        radius: 40.0,
        move_dist: 40.0,
        max_cells: g.res.int("maxcells").clamp(50, 10_000) as usize,
        num_seeds: g.res.int("seeds").clamp(1, 16) as usize,
        keep_old_cells: g.res.bool("keepold"),
        divide_age: g.res.int("divideage").clamp(1, 1000),
        minfood,
        maxfood,
        pause,
        pause_counter: pause,
        wire: g.res.bool("wireframe"),
        shapes,
        food: Vec::new(),
        texture,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);
    Box::new(st)
}

/// Upstream's quality is 3. See the note at the top of this file: three is
/// 1.54 million vertices for a full colony and two is 384 thousand, and what
/// the saver is about is the colony rather than how round one cell is.
const DEFAULTS: &[&str] = &[
    "*delay:      20000",
    "*showFPS:    False",
    "*wireframe:  False",
    "*maxcells:   800",
    "*radius:     40",
    "*seeds:      1",
    "*quality:    2",
    "*keepold:    False",
    "*minfood:    5",
    "*maxfood:    20",
    "*divideage:  20",
    "*mindist:    1.4",
    "*pause:      50",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "20000").inverted(),
    Opt::slider("pause", "Pause at end", 0.0, 400.0, 1.0, 0, "50"),
    Opt::slider("maxcells", "Max cells", 50.0, 5000.0, 10.0, 0, "800"),
    Opt::slider("radius", "Cell radius", 5.0, 80.0, 1.0, 0, "40"),
    Opt::slider("quality", "Cell quality", 0.0, 5.0, 1.0, 0, "2"),
    Opt::slider("minfood", "Min food", 0.0, 100.0, 1.0, 0, "5"),
    Opt::slider("maxfood", "Max food", 10.0, 100.0, 1.0, 0, "20"),
    Opt::slider("divideage", "Divide age", 1.0, 100.0, 1.0, 0, "20"),
    Opt::slider("mindist", "Min distance", 1.0, 3.0, 0.1, 1, "1.4"),
    Opt::slider("seeds", "Seeds", 1.0, 15.0, 1.0, 0, "1"),
    Opt::boolean("keepold", "Keep old cells", "false"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "glcells",
    label: "GL Cells",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Matthias Toussaint",
        year: "2007",
        video: Some("https://www.youtube.com/watch?v=94ac7nEQyBI"),
        blurb: "Cells growing on a petri dish.",
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

    /// Subdividing multiplies the triangles by four and welds the new
    /// vertices so that neighbouring faces share them.
    ///
    /// The test is Euler's formula rather than a vertex count: the base shape
    /// is a half-dodecahedron, which is a disc rather than a closed surface,
    /// so V - E + F is one and stays one however often it is subdivided.
    /// Welding that missed an edge would leave two vertices where there should
    /// be one and break it, and the shading would go faceted.
    #[test]
    fn subdivision_shares_its_new_vertices() {
        for divisions in 0..=3 {
            let (v, t) = create_sphere(divisions);
            assert_eq!(t.len(), 10 * 4usize.pow(divisions), "{divisions} divisions");

            let mut edges = std::collections::BTreeSet::new();
            for tri in &t {
                for k in 0..3 {
                    let (a, b) = (tri[k], tri[(k + 1) % 3]);
                    edges.insert((a.min(b), a.max(b)));
                }
            }
            let (vn, en, fnum) = (v.len() as i64, edges.len() as i64, t.len() as i64);
            assert_eq!(
                vn - en + fnum,
                1,
                "{divisions} divisions: V {vn} E {en} F {fnum}, so a vertex was not shared"
            );

            // Every vertex is on the unit sphere.
            for p in &v {
                let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-5, "a vertex of length {l}");
            }
        }
    }

    /// Every vertex normal points the same way relative to the surface.
    #[test]
    fn the_normals_all_point_outward() {
        let (v, t) = create_sphere(2);
        let n = smooth_normals(&v, &t);
        let mut out = 0;
        let mut inward = 0;
        for (p, q) in v.iter().zip(&n) {
            let d = p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
            if d > 0.0 { out += 1 } else { inward += 1 }
        }
        assert!(
            out == 0 || inward == 0,
            "{out} normals point out and {inward} point in"
        );
    }

    /// The measurement the deferral turned on: upstream's quality of three is
    /// 640 triangles a cell, and a full colony of eight hundred is over a
    /// million and a half vertices. One step down is a quarter of that.
    #[test]
    fn the_quality_knob_is_what_decides_the_frame() {
        let three = create_sphere(3).1.len() * 3 * 800;
        let two = create_sphere(2).1.len() * 3 * 800;
        assert_eq!(three, 1_536_000);
        assert_eq!(two, 384_000);
    }

    /// However many cells there are, the colony is one draw call, because they
    /// are all the same shape under a different matrix and the matrix is
    /// applied on the way out.
    #[test]
    fn the_colony_is_one_draw_call() {
        let r = run("maxcells=400&seeds=8", 300);
        let f = r.frame();
        assert!(!f.vertices.is_empty());
        // One for the cells and one for the nuclei.
        assert!(
            f.batches.len() <= 3,
            "{} batches for {} vertices",
            f.batches.len(),
            f.vertices.len()
        );
    }

    /// The colony grows: cells divide when they are old enough and have room.
    #[test]
    fn the_colony_grows_from_its_seeds() {
        let mut r = start(StartArgs::new(640, 480, "seeds=2&maxcells=200", 20260812));
        let mut most = 0;
        for _ in 0..600 {
            r.step();
            most = most.max(r.frame().vertices.len());
        }
        let one_cell = create_sphere(2).1.len() * 3;
        assert!(
            most > one_cell * 10,
            "the colony never got past {} cells",
            most / one_cell
        );
    }

    /// When the dish fills up or the colony starves, it is reseeded, and the
    /// new colony is a different colour.
    #[test]
    fn a_finished_colony_is_reseeded() {
        let mut r = start(StartArgs::new(
            640,
            480,
            "maxcells=60&seeds=1&pause=1",
            20260812,
        ));
        let mut colors = std::collections::BTreeSet::new();
        for _ in 0..1500 {
            r.step();
            if let Some(b) = r.frame().batches.first() {
                colors.insert(b.lights[0].diffuse.map(f32::to_bits));
            }
        }
        assert!(colors.len() > 1, "the dish was never reseeded");
    }
}
