//! Port of `hacks/glx/razzledazzle.c`.
//!
//! ```text
//! razzledazzle, Copyright (c) 2018-2020 Jamie Zawinski <jwz@jwz.org>
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
//! Dazzle camouflage: the paint scheme the Royal Navy put on ships in the
//! First World War, which made no attempt to hide them and instead broke up
//! their outline so badly that a submarine could not judge their size, range
//! or heading. Before radar that was worth a great deal.
//!
//! The pattern is a grid of quadrilaterals whose corners wander a little way
//! from where they started, each one filled with a set of parallel stripes in
//! two shades. Neighbouring cells stripe in different directions, and that is
//! the whole trick: there is no line in the picture that reads as an edge.
//!
//! The ship is not painted with the pattern, it is cut out of it. It goes into
//! the depth buffer with the colour mask shut, and then the sea and sky are
//! painted over the whole screen at the near plane, so they are rejected
//! exactly where the hull stands in front of them, and the dazzle already
//! there shows through the hole.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::XColor;
use crate::runtime::gl::Shape;
use crate::runtime::gllist::GlList;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, XEvent, frand, random,
};

const SHIPS: [&str; 8] = [
    crate::models::SHIPS_SHIP1,
    crate::models::SHIPS_SHIP2,
    crate::models::SHIPS_SHIP3,
    crate::models::SHIPS_SHIP4,
    crate::models::SHIPS_SHIP5,
    crate::models::SHIPS_SHIP6,
    crate::models::SHIPS_SHIP7,
    crate::models::SHIPS_SHIP8,
];

fn bellrand(n: f64) -> f32 {
    ((frand(n) + frand(n) + frand(n)) / 3.0) as f32
}

fn randsign() -> f32 {
    if random() & 1 == 1 { 1.0 } else { -1.0 }
}

/// One corner of the grid, and the stripes of the cell that starts there.
#[derive(Clone, Default)]
struct Node {
    /// Where the corner belongs, and where it has wandered to.
    gx: f32,
    gy: f32,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
    nstripes: usize,
    horiz: bool,
    drawn: bool,
    color1: [f32; 4],
    color2: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ships,
    Flat,
    Random,
}

/// The pattern itself, which is the whole saver apart from the ship and needs
/// nothing from GL to run.
struct Grid {
    xoff: f32,
    yoff: f32,
    dx: f32,
    dy: f32,
    nodes: Vec<Node>,
    /// The side of the grid, in cells.
    wh: usize,
    colors: Vec<XColor>,
    speed: f32,
    density: f32,
    thickness: f32,
    ncolors: usize,
    wire: bool,
}

struct Dazzle {
    grid: Grid,
    ships: Vec<Option<u32>>,
    /// Which ship is on the water, or none for the flat pattern.
    which_ship: Option<usize>,
    frames: f64,
    aspect: f32,
    wire: bool,
}

impl Grid {
    fn new(density: f32, speed: f32, thickness: f32, ncolors: usize, wire: bool) -> Self {
        let mut this = Grid {
            xoff: 0.0,
            yoff: 0.0,
            dx: 0.0,
            dy: 0.0,
            nodes: Vec::new(),
            wh: (density * 2.0) as usize,
            colors: Vec::new(),
            speed,
            density,
            thickness,
            ncolors: ncolors.max(1),
            wire,
        };
        this.randomize();
        this
    }

    fn node(&self, x: usize, y: usize) -> &Node {
        &self.nodes[(y % self.wh) * self.wh + (x % self.wh)]
    }
}

impl Dazzle {
    /// `draw_grid`, once for each of the nine copies that tile the screen.
    fn draw_grid(&mut self, g: &mut Gl, gx: i32, gy: i32) {
        let wire = self.wire;
        let this = &mut self.grid;
        let wh = this.wh;
        let density = this.density;
        let bx = (this.xoff as f64 % 2.0) as f32 + gx as f32 * 2.0;
        let by = (this.yoff as f64 % 2.0) as f32 + gy as f32 * 2.0;

        if wire {
            g.glx.color4f(1.0, 1.0, 1.0, 1.0);
        } else {
            g.glx.begin(Shape::Quads);
        }

        for y in 0..wh {
            for x in 0..wh {
                let (n0, n1, n2, n3) = (
                    this.node(x, y).clone(),
                    this.node(x + 1, y).clone(),
                    this.node(x + 1, y + 1).clone(),
                    this.node(x, y + 1).clone(),
                );
                // The last row and column wrap round, so they need a whole
                // grid added back to land on the far side rather than at zero.
                let xoff = if x < wh - 1 { 0.0 } else { wh as f32 };
                let yoff = if y < wh - 1 { 0.0 } else { wh as f32 };

                let x0 = n0.x / density - 1.0 + bx;
                let y0 = n0.y / density - 1.0 + by;
                let x1 = (n1.x + xoff) / density - 1.0 + bx;
                let y1 = n1.y / density - 1.0 + by;
                let x2 = (n2.x + xoff) / density - 1.0 + bx;
                let y2 = (n2.y + yoff) / density - 1.0 + by;
                let x3 = n3.x / density - 1.0 + bx;
                let y3 = (n3.y + yoff) / density - 1.0 + by;

                // Not quite right, as upstream says: all four corners being
                // off screen does not prove the quad is.
                let max = 0.75;
                let inside = |x: f32, y: f32| x >= -max && y >= -max && x <= max && y <= max;
                if !(inside(x0, y0) || inside(x1, y1) || inside(x2, y2) || inside(x3, y3)) {
                    continue;
                }
                this.nodes[(y % wh) * wh + (x % wh)].drawn = true;

                if wire {
                    g.glx.color4f(0.5, 0.0, 0.5, 1.0);
                    g.glx.begin(Shape::LineLoop);
                    for (x, y) in [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] {
                        g.glx.vertex3f(x, y, 0.0);
                    }
                    g.glx.end();
                }

                for i in 0..n0.nstripes {
                    let ss = i as f32 / n0.nstripes as f32;
                    let ss1 = (i + 1) as f32 / n0.nstripes as f32;
                    if i & 1 == 1 {
                        let c = n0.color1;
                        g.glx.color4f(c[0], c[1], c[2], c[3]);
                    } else if wire {
                        continue;
                    } else {
                        let c = n0.color2;
                        g.glx.color4f(c[0], c[1], c[2], c[3]);
                    }

                    // A stripe is the slice of the cell between two parallel
                    // cuts, taken across whichever pair of sides this cell
                    // has decided to run between.
                    let quad = if n0.horiz {
                        [
                            (
                                n0.x + (n3.x - n0.x) * ss,
                                n0.y + ((n3.y + yoff) - n0.y) * ss,
                            ),
                            (
                                (n1.x + xoff) + ((n2.x + xoff) - (n1.x + xoff)) * ss,
                                n1.y + ((n2.y + yoff) - n1.y) * ss,
                            ),
                            (
                                (n1.x + xoff) + ((n2.x + xoff) - (n1.x + xoff)) * ss1,
                                n1.y + ((n2.y + yoff) - n1.y) * ss1,
                            ),
                            (
                                n0.x + (n3.x - n0.x) * ss1,
                                n0.y + ((n3.y + yoff) - n0.y) * ss1,
                            ),
                        ]
                    } else {
                        [
                            (
                                n0.x + ((n1.x + xoff) - n0.x) * ss,
                                n0.y + (n1.y - n0.y) * ss,
                            ),
                            (
                                n3.x + ((n2.x + xoff) - n3.x) * ss,
                                (n3.y + yoff) + ((n2.y + yoff) - (n3.y + yoff)) * ss,
                            ),
                            (
                                n3.x + ((n2.x + xoff) - n3.x) * ss1,
                                (n3.y + yoff) + ((n2.y + yoff) - (n3.y + yoff)) * ss1,
                            ),
                            (
                                n0.x + ((n1.x + xoff) - n0.x) * ss1,
                                n0.y + (n1.y - n0.y) * ss1,
                            ),
                        ]
                    };

                    if wire {
                        g.glx.begin(Shape::Lines);
                    }
                    for (x, y) in quad {
                        g.glx
                            .vertex3f(x / density - 1.0 + bx, y / density - 1.0 + by, 0.0);
                    }
                    if wire {
                        g.glx.end();
                    }
                }
            }
        }

        if !wire {
            g.glx.end();
        }
    }
}

impl Grid {
    /// `move_grid`: every corner drifts, but never more than a fraction of a
    /// cell from where it belongs, so the grid stays a grid.
    fn move_grid(&mut self) {
        self.xoff += self.dx;
        self.yoff += self.dy;
        if random().is_multiple_of(50) {
            self.dx += frand(0.0002) as f32 * randsign() * self.speed;
            self.dy += frand(0.0002) as f32 * randsign() * self.speed;
        }
        self.dx = self.dx.min(0.003 * self.speed);
        self.dy = self.dy.min(0.003 * self.speed);

        let max = 1.0 / self.density * 3.0;
        let wire = self.wire;
        for i in 0..self.nodes.len() {
            let (x2, y2) = {
                let n = &self.nodes[i];
                (n.x + n.dx, n.y + n.dy)
            };
            {
                let n = &mut self.nodes[i];
                if x2 < n.gx + max && x2 >= n.gx - max && y2 < n.gy + max && y2 >= n.gy - max {
                    n.x = x2;
                    n.y = y2;
                }
            }
            if random().is_multiple_of(50) {
                let (a, b) = (
                    frand(0.0005) as f32 * randsign() * self.speed,
                    frand(0.0005) as f32 * randsign() * self.speed,
                );
                let n = &mut self.nodes[i];
                n.dx += a;
                n.dy += b;
            }

            // A cell nobody saw can be given new stripes without anyone
            // noticing them change.
            if !self.nodes[i].drawn {
                let a = random() as usize % self.ncolors;
                let b = (a + self.ncolors / 2) % self.ncolors;
                let cscale = 0.3;
                let (c1, c2) = (self.colors[a], self.colors[b]);
                let mut color1 = [
                    c1.red as f32 / 65536.0,
                    c1.green as f32 / 65536.0,
                    c1.blue as f32 / 65536.0,
                    1.0,
                ];
                let mut color2 = [
                    c2.red as f32 / 65536.0,
                    c2.green as f32 / 65536.0,
                    c2.blue as f32 / 65536.0,
                    1.0,
                ];
                if !wire {
                    for k in 0..3 {
                        color1[k] = cscale * color1[k] + 1.0 - cscale;
                        color2[k] *= cscale;
                    }
                }
                let horiz = random() & 1 == 1;
                let nstripes = 2 + bellrand(1.0 / self.thickness as f64) as usize;
                let n = &mut self.nodes[i];
                n.color1 = color1;
                n.color2 = color2;
                n.horiz = horiz;
                n.nstripes = nstripes;
            }
            self.nodes[i].drawn = false;
        }
    }

    /// `dazzle_randomize`: a fresh grid, a fresh pair of colours, and a
    /// thousand moves to shake it loose before anyone sees it.
    fn randomize(&mut self) -> (f32, f32) {
        self.ncolors = self.ncolors.max(1);
        self.colors = if self.ncolors < 3 {
            crate::runtime::color::make_random_colormap(self.ncolors, true)
        } else {
            crate::runtime::color::make_smooth_colormap(self.ncolors)
        };

        self.nodes = vec![Node::default(); self.wh * self.wh];
        for y in 0..self.wh {
            for x in 0..self.wh {
                let n = &mut self.nodes[self.wh * y + x];
                n.gx = x as f32;
                n.x = x as f32;
                n.gy = y as f32;
                n.y = y as f32;
            }
        }
        self.dx = 0.0;
        self.dy = 0.0;
        self.xoff = 0.0;
        self.yoff = 0.0;
        for _ in 0..1000 {
            self.move_grid();
        }
        self.dx = frand(0.0005) as f32 * randsign() * self.speed;
        self.dy = frand(0.0005) as f32 * randsign() * self.speed;
        (self.dx, self.dy)
    }

    /// A ship drifts a tenth as fast: it is a big thing seen far off.
    fn slow_down_for_a_ship(&mut self) {
        self.dx /= 10.0;
        self.dy /= 10.0;
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let mode = match g.res.string("mode") {
        "ships" | "ship" => Mode::Ships,
        "flat" => Mode::Flat,
        _ => Mode::Random,
    };
    let wire = g.res.bool("wireframe");
    let density = g.res.float("density") as f32;

    let which_ship = match mode {
        Mode::Flat => None,
        // In random mode one time in three there is no ship at all.
        Mode::Random if random().is_multiple_of(3) => None,
        _ => Some(random() as usize % SHIPS.len()),
    };

    let mut ships = Vec::new();
    if mode != Mode::Flat {
        for src in SHIPS {
            let model = GlList::parse(src);
            let list = g.glx.gen_lists(1);
            g.glx.new_list(list);
            g.glx.push_matrix();
            // Half the ships face the other way.
            if random() & 1 == 1 {
                g.glx.scale(-1.0, 1.0, 1.0);
                g.glx.translate(-1.0, 0.0, 0.0);
            }
            model.render(&mut g.glx, wire);
            g.glx.pop_matrix();
            g.glx.end_list();
            ships.push(Some(list));
        }
    }

    let mut grid = Grid::new(
        density,
        g.res.float("speed") as f32,
        g.res.float("thickness") as f32,
        (g.res.int("ncolors") as usize).saturating_sub(1),
        wire,
    );
    if which_ship.is_some() {
        grid.slow_down_for_a_ship();
    }

    let mut this = Dazzle {
        grid,
        ships,
        which_ship,
        frames: 0.0,
        aspect: 1.0,
        wire,
    };

    let (w, h) = (g.width(), g.height());
    this.reshape(g, w, h);
    Box::new(this)
}

impl Hack3d for Dazzle {
    fn reshape(&mut self, g: &mut Gl, width: i32, height: i32) {
        g.glx.viewport(0, 0, width, height);
        self.aspect = width as f32 / height as f32;
    }

    fn event(&mut self, _g: &mut Gl, event: &XEvent) -> bool {
        // Upstream lets you drag a corner of the grid about. There is no way
        // to put it back, so a click here just deals a new pattern.
        if matches!(event, XEvent::ButtonPress { .. }) {
            self.grid.randomize();
            if self.which_ship.is_some() {
                self.grid.slow_down_for_a_ship();
            }
            return true;
        }
        false
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if self.aspect > 5.0 {
            let s = 1.0 / self.aspect;
            g.glx.ortho(0.0, 1.0, 0.5 - s, 0.5 + s, -1.0, 1.0);
        } else {
            g.glx.ortho(0.0, 1.0, 1.0, 0.0, -1.0, 1.0);
        }
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        g.glx.lighting(false);
        g.glx.depth_test(false);
        g.glx.cull_face(false);
        g.glx.color_material(true);
        g.glx.clear();

        g.glx.push_matrix();
        g.glx.translate(0.5, 0.5, 0.0);
        if self.wire {
            g.glx.scale(0.2, 0.2, 1.0);
        }

        self.grid.move_grid();
        for y in -1..=1 {
            for x in -1..=1 {
                self.draw_grid(g, x, y);
            }
        }

        if let Some(which) = self.which_ship {
            if self.wire {
                g.glx.color4f(1.0, 0.0, 0.0, 1.0);
            } else {
                // Into the depth buffer but not the frame buffer.
                g.glx.color4f(0.0, 0.0, 0.0, 1.0);
                g.glx.color_mask(false);
                g.glx.clear_depth();
                g.glx.depth_test(true);
            }

            g.glx.push_matrix();
            g.glx.rotate(90.0, 1.0, 0.0, 0.0);
            g.glx.scale(0.9, 0.9, 0.9);
            g.glx.translate(-0.5, 0.0, -0.2);
            g.glx.scale(1.0, 1.0, self.aspect);
            // Wave the boat horizontally and vertically.
            let s = self.grid.speed as f64;
            g.glx.translate(
                ((self.frames / 80.0 * std::f64::consts::PI * s).cos() / 200.0) as f32,
                0.0,
                ((self.frames / 60.0 * std::f64::consts::PI * s).cos() / 300.0) as f32,
            );
            if let Some(Some(list)) = self.ships.get(which) {
                g.glx.call_list(*list);
            }
            g.glx.pop_matrix();

            // Wave the horizon vertically.
            g.glx.translate(
                0.0,
                ((self.frames / 120.0 * std::f64::consts::PI * s).cos() / 200.0) as f32,
                0.0,
            );

            if !self.wire {
                g.glx.color_mask(true);
                // Black out everything that is not a ship: sea below the
                // horizon, sky above it, both at the near plane, so the hull
                // in the depth buffer punches a hole in them.
                let horizon = 0.15;
                g.glx.color4f(0.7, 0.7, 1.0, 1.0);
                g.glx.begin(Shape::Quads);
                for (x, y) in [(-1.0, -1.0), (-1.0, horizon), (1.0, horizon), (1.0, -1.0)] {
                    g.glx.vertex3f(x, y, 0.0);
                }
                g.glx.end();
                g.glx.color4f(0.0, 0.05, 0.2, 1.0);
                g.glx.begin(Shape::Quads);
                for (x, y) in [(-1.0, horizon), (-1.0, 1.0), (1.0, 1.0), (1.0, horizon)] {
                    g.glx.vertex3f(x, y, 0.0);
                }
                g.glx.end();
                g.glx.depth_test(false);
            }
        }

        if self.wire {
            g.glx.color4f(0.0, 1.0, 1.0, 1.0);
            g.glx.begin(Shape::LineLoop);
            for (x, y) in [(-0.5, -0.5), (-0.5, 0.5), (0.5, 0.5), (0.5, -0.5)] {
                g.glx.vertex3f(x, y, 0.0);
            }
            g.glx.end();
        }

        g.glx.pop_matrix();
        self.frames += 1.0;

        g.res.int("delay") as u32
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:     30000",
    "*showFPS:   False",
    "*wireframe: False",
    "*ncolors:   2",
    "*speed:     1.0",
    "*density:   5.0",
    "*thickness: 0.1",
    "*mode:      random",
];

const MODES: &[crate::runtime::opts::SelectItem] = &[
    crate::runtime::opts::SelectItem {
        value: "random",
        label: "Ships or flat pattern",
    },
    crate::runtime::opts::SelectItem {
        value: "ships",
        label: "Ship outlines",
    },
    crate::runtime::opts::SelectItem {
        value: "flat",
        label: "Flat pattern",
    },
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::select("mode", "Object", MODES, "random"),
    Opt::slider("speed", "Speed", 0.1, 10.0, 0.1, 1, "1.0"),
    Opt::slider("ncolors", "Colors", 2.0, 20.0, 1.0, 0, "2"),
    Opt::slider("density", "Density", 1.0, 10.0, 0.5, 1, "5.0"),
    Opt::slider("thickness", "Lines", 0.05, 1.0, 0.05, 2, "0.1"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "razzledazzle",
    label: "Razzle Dazzle",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2018",
        video: Some("https://www.youtube.com/watch?v=tV_70VxJFfs"),
        blurb: "An infinitely scrolling sequence of dazzle camouflage \
                patterns, sometimes with a ship cut out of them.",
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
    use crate::runtime::rand::ya_rand_init;

    /// A grid on its own, with the generator seeded: nothing here needs GL.
    fn grid(density: f32, thickness: f32) -> Grid {
        ya_rand_init(20260811);
        Grid::new(density, 1.0, thickness, 1, false)
    }

    /// Every corner of the grid stays within three cells' worth of a third of
    /// a cell from where it belongs, however long it drifts.
    #[test]
    fn the_grid_stays_a_grid() {
        ya_rand_init(20260811);
        let mut d = Grid::new(5.0, 10.0, 0.1, 1, false);
        for _ in 0..400 {
            d.move_grid();
        }
        let max = 1.0 / d.density * 3.0;
        for n in &d.nodes {
            assert!(
                (n.x - n.gx).abs() <= max && (n.y - n.gy).abs() <= max,
                "a corner wandered to ({}, {}) from ({}, {})",
                n.x,
                n.y,
                n.gx,
                n.gy
            );
        }
    }

    /// The pattern tiles: nine copies are drawn, offset by two units each way,
    /// and cells too far off screen are dropped. However far it has scrolled,
    /// the copies between them always cover the whole visible square, which is
    /// the point of drawing nine of something that is only seen once.
    #[test]
    fn the_pattern_covers_the_screen_however_far_it_has_scrolled() {
        let mut r = start(StartArgs::new(640, 480, "mode=flat&speed=10", 20260811));
        for frame in 0..300 {
            r.step();
            let f = r.frame();
            let quads: Vec<_> = f
                .batches
                .iter()
                .filter(|b| b.primitive == crate::runtime::gl::Primitive::Triangles)
                .collect();
            // Nothing changes between the nine copies, so they are one batch.
            assert_eq!(quads.len(), 1, "the tiles did not merge on frame {frame}");
            let b = quads[0];
            assert!(
                b.count.is_multiple_of(3) && b.count > 100,
                "{} verts",
                b.count
            );

            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            let (mut lo_y, mut hi_y) = (f32::MAX, f32::MIN);
            for v in &f.vertices[b.first..b.first + b.count] {
                lo = lo.min(v.pos[0]);
                hi = hi.max(v.pos[0]);
                lo_y = lo_y.min(v.pos[1]);
                hi_y = hi_y.max(v.pos[1]);
            }
            assert!(
                lo <= -0.75 && hi >= 0.75 && lo_y <= -0.75 && hi_y >= 0.75,
                "frame {frame} left a gap: x {lo}..{hi}, y {lo_y}..{hi_y}"
            );
        }
    }

    /// The two shades of a cell are a light and a dark version of the same
    /// colour, which is what makes the pattern read as dazzle rather than as
    /// confetti.
    #[test]
    fn the_stripes_are_light_and_dark() {
        let d = grid(5.0, 0.1);
        for n in &d.nodes {
            let light: f32 = n.color1[..3].iter().sum();
            let dark: f32 = n.color2[..3].iter().sum();
            assert!(light > dark, "{light} is not lighter than {dark}");
            assert!(light > 2.0, "the light shade is only {light}");
            assert!(dark < 1.0, "the dark shade is {dark}");
        }
    }

    /// A cell has at least two stripes, or it would not be striped at all.
    #[test]
    fn every_cell_has_stripes() {
        let d = grid(5.0, 1.0);
        assert!(d.nodes.iter().all(|n| n.nstripes >= 2));
        // Thin lines mean many stripes: the count is 2 plus a bell curve over
        // one over the thickness.
        let d = grid(5.0, 0.05);
        let most = d.nodes.iter().map(|n| n.nstripes).max().unwrap_or(0);
        assert!(most > 5, "the thinnest setting only got to {most} stripes");
    }

    /// The ship is cut out of the pattern rather than painted on it: it goes
    /// into the depth buffer with the colour mask shut.
    #[test]
    fn the_ship_is_a_hole_in_the_pattern() {
        let mut r = start(StartArgs::new(640, 480, "mode=ships", 20260811));
        r.step();
        let f = r.frame();
        let masked = f.batches.iter().filter(|b| !b.color_mask).count();
        assert!(masked > 0, "nothing was drawn depth-only");
        // And the sea and sky that follow are drawn with the depth test on,
        // so the hull rejects them.
        let sea = f
            .batches
            .iter()
            .rev()
            .find(|b| b.color_mask && b.depth_test)
            .expect("no sea");
        assert!(sea.count >= 6, "the sea is not a quad");
    }
}
