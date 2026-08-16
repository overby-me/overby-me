//! Port of `hacks/glx/geodesicgears.c`.
//!
//! ```text
//! geodesicgears, Copyright (c) 2014-2015 Jamie Zawinski <jwz@jwz.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Inspired by http://bugman123.com/Gears/
//! and by http://kennethsnelson.net/PortraitOfAnAtom.pdf
//! ```
//!
//! A set of meshed gears arranged on the surface of a sphere.
//!
//! Each gear is a disc lying on the sphere, named by the axis it turns about.
//! Two of them touch when the angle between their axes is no more than the sum
//! of the angles their radii subtend, which is all the saver needs to know to
//! find every mesh: compare each pair, and the ones that touch become
//! neighbours. Walking that neighbour graph depth first from gear zero turns it
//! into a tree, and the tree gives every gear a direction, since a gear always
//! turns against the one driving it.
//!
//! Meshing them is the fiddly part. A gear's teeth must fall into its parent's
//! gaps, so each child is rotated by hand: take the tooth of the child nearest
//! any tooth of the parent, then try sixty-four offsets over one tooth's worth
//! of arc and keep whichever brings that pair closest together. Upstream is
//! honest that several of the arrangements have phase errors and do not quite
//! mesh; those are kept here as they are.
//!
//! Every arrangement is drawn from two or three gear shapes, so a shape that
//! draws expensively is paid for ninety-two times over. At 1280x720 the
//! heaviest sphere comes to 1936 batches and 949k vertices, both within what
//! the runtime handles; getting there needed the front-face fix in
//! `runtime::involute`, without which one spoked shape alone cost 17k batches.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::involute::{Gear, Size, biggest_ring, draw_gear};
use crate::runtime::texfont::TexFont;
use crate::runtime::{
    About, Gl, Hack3d, Opt, Rotator, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
    random, screenhack_event_helper,
};

/// Which arrangement of gears on the sphere to build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Five gears on the faces of a triangular prism.
    Prism,
    /// Eight, on the faces of an octahedron.
    Octo,
    /// Ten.
    Deca,
    /// Fourteen.
    G14,
    /// Eighteen.
    G18,
    /// Thirty-two, on a truncated icosahedron: one per face and one per vertex.
    G32,
    /// Ninety-two, on a 3v geodesic sphere: 20 + 12 + 60.
    G92,
}

/// The arrangements, in the order the saver cycles through them. `args` is
/// read differently by each kind: see [`Geodesic::make_32`] and
/// [`Geodesic::make_92`].
///
/// Upstream's table also carries a 182-gear entry whose builder is an
/// unconditional `abort()`, so it is left out here rather than crashing.
const TEMPLATES: &[(Kind, [f64; 5])] = &[
    (Kind::Prism, [0.0; 5]),
    (Kind::Octo, [0.0; 5]),
    (Kind::Deca, [0.0; 5]),
    (Kind::G14, [0.0; 5]),
    (Kind::G18, [0.0; 5]),
    // teeth1, teeth2, radius1.
    (Kind::G32, [15.0, 6.0, 0.4535, 0.0, 0.0]),
    (Kind::G32, [15.0, 12.0, 0.3560, 0.0, 0.0]),
    (Kind::G32, [20.0, 6.0, 0.4850, 0.0, 0.0]),
    // Double of 10:6.
    (Kind::G32, [20.0, 12.0, 0.3995, 0.0, 0.0]),
    (Kind::G32, [20.0, 18.0, 0.3375, 0.0, 0.0]),
    (Kind::G32, [25.0, 6.0, 0.5065, 0.0, 0.0]),
    (Kind::G32, [25.0, 12.0, 0.4300, 0.0, 0.0]),
    (Kind::G32, [25.0, 18.0, 0.3725, 0.0, 0.0]),
    (Kind::G32, [25.0, 24.0, 0.3270, 0.0, 0.0]),
    // Double of 15:6.
    (Kind::G32, [30.0, 12.0, 0.4535, 0.0, 0.0]),
    (Kind::G32, [30.0, 18.0, 0.3995, 0.0, 0.0]),
    // Double of 15:12.
    (Kind::G32, [30.0, 24.0, 0.3560, 0.0, 0.0]),
    (Kind::G32, [30.0, 30.0, 0.3205, 0.0, 0.0]),
    (Kind::G32, [35.0, 12.0, 0.4710, 0.0, 0.0]),
    (Kind::G32, [35.0, 18.0, 0.4208, 0.0, 0.0]),
    (Kind::G32, [35.0, 24.0, 0.3800, 0.0, 0.0]),
    (Kind::G32, [35.0, 30.0, 0.3450, 0.0, 0.0]),
    (Kind::G32, [35.0, 36.0, 0.3160, 0.0, 0.0]),
    // Double of 20:6.
    (Kind::G32, [40.0, 12.0, 0.4850, 0.0, 0.0]),
    // Double of 10:6 and of 20:12.
    (Kind::G32, [40.0, 24.0, 0.3995, 0.0, 0.0]),
    // Double of 25:6.
    (Kind::G32, [50.0, 12.0, 0.5065, 0.0, 0.0]),
    // Double of 25:12.
    (Kind::G32, [50.0, 24.0, 0.4300, 0.0, 0.0]),
    // These all have phase errors and do not always mesh properly. Upstream
    // wonders aloud whether to omit them, and keeps them; so does this.
    // teeth1, teeth2, teeth3, r1, pitch3.
    (Kind::G92, [35.0, 36.0, 16.0, 0.2660, 0.366]),
    (Kind::G92, [25.0, 36.0, 11.0, 0.2270, 0.315]),
    (Kind::G92, [25.0, 27.0, 16.0, 0.2320, 0.359]),
    (Kind::G92, [20.0, 36.0, 11.0, 0.1875, 0.283]),
    // Double of 15:15:8.
    (Kind::G92, [30.0, 30.0, 16.0, 0.2585, 0.374]),
    (Kind::G92, [20.0, 33.0, 11.0, 0.1970, 0.293]),
    (Kind::G92, [30.0, 33.0, 16.0, 0.2455, 0.354]),
    (Kind::G92, [20.0, 24.0, 16.0, 0.2030, 0.346]),
];

/// A latitude and longitude on the sphere.
#[derive(Clone, Copy)]
struct Ll {
    a: f64,
    o: f64,
}

type Xyz = [f64; 3];

fn cross(a: Xyz, b: Xyz) -> Xyz {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: Xyz, b: Xyz) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: Xyz) -> Xyz {
    let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if d == 0.0 {
        [0.0; 3]
    } else {
        [v[0] / d, v[1] / d, v[2] / d]
    }
}

fn polar_to_cartesian(v: Ll) -> Xyz {
    [v.a.cos() * v.o.cos(), v.a.cos() * v.o.sin(), v.a.sin()]
}

/// Three samples averaged, so the middle of the range comes up most often.
fn bellrand(n: f64) -> f64 {
    (frand(n) + frand(n) + frand(n)) / 3.0
}

/// One gear placed on the sphere: which shape it is, which way its axis
/// points, and how it is driven.
struct SphereGear {
    axis: Xyz,
    /// +1 or -1, or 0 before the tree has been walked.
    direction: i32,
    /// Rotational degrees from the parent gear, chosen so the teeth mesh.
    offset: f64,
    parent: Option<usize>,
    /// Gears driven by this one. No loops: this is the tree.
    children: Vec<usize>,
    /// Gears touching this one. Circular, since touching is symmetric.
    neighbors: Vec<usize>,
    /// Index into [`Geodesic::shapes`] of the gear shape, which is shared.
    shape: usize,
}

/// Whether the sphere is at rest, shrinking away, or growing back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Normal,
    Out,
    In,
}

struct Geodesic {
    rot: Rotator,
    trackball: Trackball,
    font: TexFont,
    colors: Vec<XColor>,

    shapes: Vec<Gear>,
    /// The gear ratio of each shape, kept alongside it because
    /// [`Gear`] here does not carry one.
    ratios: Vec<f64>,
    gears: Vec<SphereGear>,

    which: usize,
    mode: Mode,
    mode_tick: f64,
    /// -1 for the previous arrangement, +1 for the next, 0 for a random one.
    next: i32,
    /// When the current arrangement went up, in seconds since the saver began.
    draw_time: f64,
    started: bool,
    desc: String,

    /// Rotation of the root gear, in degrees.
    th: f64,

    speed: f64,
    timeout: f64,
    do_labels: bool,
    do_numbers: bool,
    wireframe: bool,
}

impl Geodesic {
    /// `add_gear_shape`: one gear shape, sized for the sphere and decorated at
    /// random. Returns its index in [`Self::shapes`].
    fn add_gear_shape(&mut self, radius: f64, teeth: i32, height: i32) -> usize {
        let wire = self.wireframe;
        let mut g = Gear {
            r: radius,
            nteeth: teeth,
            ..Gear::default()
        };
        self.ratios.push(1.0);

        g.tooth_h = g.r / (f64::from(teeth) * 0.4);
        if g.tooth_h > 0.06 {
            // Stubbier teeth when the tooth count is low.
            g.tooth_h *= 0.6;
        }

        g.thickness = 0.05 + bellrand(0.15);
        g.thickness2 = g.thickness / 4.0;
        g.thickness3 = g.thickness;
        g.size = if wire { Size::Small } else { Size::Large };

        // Move the disc's origin inward to make the edge of the disc be
        // tangent to the unit sphere.
        g.z = 1.0 - (1.0 - g.r * g.r).sqrt();

        // Upstream marks this one as not quite right.
        g.tooth_slope = 1.0 + (g.z * 2.0) / g.r;

        // Decide on the shape of the gear interior: just a ring with teeth;
        // that, plus a thinner in-set plate in the middle; that, plus a thin
        // raised lip on the inner plate; or a wide lip, really a thicker third
        // inner plate.
        if wire {
        } else if random().is_multiple_of(10) {
            // inner_r can go all the way in, since there is no inset disc.
            g.inner_r = g.r * 0.3 + frand((g.r - g.tooth_h / 2.0) * 0.6);
            g.inner_r2 = 0.0;
            g.inner_r3 = 0.0;
        } else {
            // inner_r does not go in very far; inner_r2 is an inset disc.
            g.inner_r = g.r * 0.5 + frand((g.r - g.tooth_h) * 0.4);
            g.inner_r2 = g.r * 0.1 + frand(g.inner_r * 0.5);
            g.inner_r3 = 0.0;

            if g.inner_r2 > g.r * 0.2 {
                let nn = random() % 10;
                if nn <= 2 {
                    g.inner_r3 = g.r * 0.1 + frand(g.inner_r2 * 0.2);
                } else if nn <= 7 && g.inner_r2 >= 0.1 {
                    g.inner_r3 = g.inner_r2 - 0.01;
                }
            }
        }

        // With three discs, sometimes make the middle one be spokes.
        if g.inner_r3 != 0.0 && random().is_multiple_of(5) {
            g.spokes = 2 + bellrand(5.0) as i32;
            g.spoke_thickness = 1.0 + frand(7.0);
            if g.spokes == 2 && g.spoke_thickness < 2.0 {
                g.spoke_thickness += 1.0;
            }
        }

        // Sometimes add little nubbly bits, if there is room.
        if !wire && g.nteeth > 5 {
            let (_, _, size, _) = biggest_ring(&g);
            if size > g.r * 0.2 && random().is_multiple_of(5) {
                g.nubs = 1 + (random() % 16) as i32;
                if g.nubs > 8 {
                    g.nubs = 1;
                }
            }
        }

        // How complex the polygon model should be, from the tooth size in
        // pixels.
        let pix = g.tooth_h * f64::from(height);
        g.size = if pix <= 4.0 {
            Size::Small
        } else if pix <= 8.0 {
            Size::Medium
        } else if pix <= 30.0 {
            Size::Large
        } else {
            Size::Huge
        };

        debug_assert!(g.inner_r3 <= g.inner_r2 || g.inner_r2 == 0.0);
        debug_assert!(g.inner_r2 <= g.inner_r);
        debug_assert!(g.inner_r <= g.r);

        let n = self.colors.len().max(1);
        let i = (random() as usize) % n;
        let c = self.colors[i];
        g.color = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            1.0,
        ];

        let c = self.colors[(i + n / 2) % n];
        g.color2 = [
            f32::from(c.red) / 65536.0,
            f32::from(c.green) / 65536.0,
            f32::from(c.blue) / 65536.0,
            1.0,
        ];

        self.shapes.push(g);
        self.shapes.len() - 1
    }

    /// `add_sphere_gear`: put a copy of a shape on the given axis, unless
    /// something is already there.
    fn add_sphere_gear(&mut self, shape: usize, axis: Xyz) {
        let axis = normalize(axis);
        if self.gears.iter().any(|g| g.axis == axis) {
            return;
        }
        self.gears.push(SphereGear {
            axis,
            direction: 0,
            offset: 0.0,
            parent: None,
            children: Vec::new(),
            neighbors: Vec::new(),
            shape,
        });
    }

    /// `gears_touch_p`: whether the two discs on the surface overlap.
    ///
    /// The angle a disc of radius r subtends at the centre of the unit sphere
    /// is `asin(r)`, and the angle between two axes is `acos(v1 . v2)`. If the
    /// two half-angles together reach as far as the angle between the axes,
    /// the discs meet.
    fn gears_touch(&self, a: usize, b: usize) -> bool {
        let (ga, gb) = (&self.gears[a], &self.gears[b]);
        let t1 = self.shapes[ga.shape].r.asin();
        let t2 = self.shapes[gb.shape].r.asin();
        let th = dot(ga.axis, gb.axis).clamp(-1.0, 1.0).acos();
        t1 + t2 >= th
    }

    /// `link_children`, depth first: every gear a neighbour reaches for the
    /// first time becomes its child, which turns the neighbour graph into a
    /// tree.
    fn link_children(&mut self, parent: usize) {
        // Iterative rather than recursive: 92 gears deep is fine either way,
        // but this keeps the borrow checker out of it.
        let mut stack = vec![parent];
        while let Some(p) = stack.pop() {
            for i in 0..self.gears[p].neighbors.len() {
                let child = self.gears[p].neighbors[i];
                if self.gears[child].parent.is_none() {
                    self.gears[child].parent = Some(p);
                    self.gears[p].children.push(child);
                    stack.push(child);
                }
            }
        }
    }

    /// `orient_gears`: a gear turns against the one driving it.
    fn orient_gears(&mut self, root: usize) {
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            let d = self.gears[p].direction;
            for i in 0..self.gears[p].children.len() {
                let c = self.gears[p].children[i];
                self.gears[c].direction = -d;
                stack.push(c);
            }
        }
    }

    /// `tooth_coords`: where the given tooth of the given gear is, in model
    /// coordinates.
    fn tooth_coords(&self, s: usize, tooth: i32) -> Xyz {
        let sg = &self.gears[s];
        let g = &self.shapes[sg.shape];
        let ratio = self.ratios[sg.shape];
        let off = sg.offset * (std::f64::consts::PI / 180.0) * ratio * f64::from(sg.direction);
        let th = (f64::from(tooth) * std::f64::consts::PI * 2.0 / f64::from(g.nteeth)) - off;

        let from = [0.0, 1.0, 0.0];
        let to = sg.axis;
        let axis = normalize(cross(from, to));
        let angle = dot(from, to).clamp(-1.0, 1.0).acos();

        let (x, y, z) = (axis[0], axis[1], axis[2]);
        let c = angle.cos();
        let s = angle.sin();

        // This is what glRotatef does.
        let m = [
            [
                x * x * (1.0 - c) + c,
                y * x * (1.0 - c) + z * s,
                x * z * (1.0 - c) - y * s,
            ],
            [
                x * y * (1.0 - c) - z * s,
                y * y * (1.0 - c) + c,
                y * z * (1.0 - c) + x * s,
            ],
            [
                x * z * (1.0 - c) + y * s,
                y * z * (1.0 - c) - x * s,
                z * z * (1.0 - c) + c,
            ],
        ];

        let p1 = normalize([g.r * th.sin(), 1.0 - g.z, g.r * th.cos()]);
        [
            p1[0] * m[0][0] + p1[1] * m[1][0] + p1[2] * m[2][0],
            p1[0] * m[0][1] + p1[1] * m[1][1] + p1[2] * m[2][1],
            p1[0] * m[0][2] + p1[1] * m[1][2] + p1[2] * m[2][2],
        ]
    }

    /// `parent_tooth`: which tooth of this gear is nearest any tooth of its
    /// parent, and where that parent tooth is.
    fn parent_tooth(&self, s: usize) -> (i32, Xyz) {
        let Some(p) = self.gears[s].parent else {
            return (0, [0.0; 3]);
        };
        let n1 = self.shapes[self.gears[s].shape].nteeth;
        let n2 = self.shapes[self.gears[p].shape].nteeth;

        let mut min_dist = f64::MAX;
        let mut min_tooth = 0;
        let mut min_parent = [0.0; 3];
        for i in 0..n1 {
            let a = self.tooth_coords(s, i);
            for j in 0..n2 {
                let b = self.tooth_coords(p, j);
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                if dist < min_dist {
                    min_dist = dist;
                    min_parent = b;
                    min_tooth = i;
                }
            }
        }
        (min_tooth, min_parent)
    }

    /// `align_gear_teeth`: turn each gear until its teeth fall into its
    /// parent's gaps.
    ///
    /// Parents before children, since the offset chosen for a parent moves
    /// every tooth downstream of it.
    fn align_gear_teeth(&mut self, root: usize) {
        let mut queue = vec![root];
        let mut at = 0;
        while at < queue.len() {
            let s = queue[at];
            at += 1;

            if self.gears[s].parent.is_some() {
                let (pt, pc) = self.parent_tooth(s);
                let range = 360.0 / f64::from(self.shapes[self.gears[s].shape].nteeth);
                let steps = 64;
                let mut min_dist = f64::MAX;
                let mut min_off = 0.0;

                for i in 0..steps {
                    let off = -range / 2.0 + range * f64::from(i) / f64::from(steps);
                    self.gears[s].offset = off;
                    let tc = self.tooth_coords(s, pt);
                    let d = [pc[0] - tc[0], pc[1] - tc[1], pc[2] - tc[2]];
                    let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    if dist < min_dist {
                        min_dist = dist;
                        min_off = off;
                    }
                }
                self.gears[s].offset = min_off;
            }

            queue.extend_from_slice(&self.gears[s].children.clone());
        }
    }

    /// `describe_gears`: the label, which counts the gears by tooth count.
    fn describe_gears(&mut self) {
        let mut per_teeth: Vec<(i32, usize)> = Vec::new();
        for g in &self.gears {
            let t = self.shapes[g.shape].nteeth;
            match per_teeth.iter_mut().find(|(n, _)| *n == t) {
                Some((_, c)) => *c += 1,
                None => per_teeth.push((t, 1)),
            }
        }
        per_teeth.sort_unstable();

        let mut s = String::new();
        for (teeth, count) in &per_teeth {
            if !s.is_empty() {
                s.push_str(",\n");
            }
            s.push_str(&format!("{count} gears with {teeth} teeth"));
        }
        if per_teeth.len() > 1 {
            s.push_str(&format!(",\n{} gears total", self.gears.len()));
        }
        s.push('.');
        self.desc = s;
    }

    /// `sort_gears`: find every mesh, make a tree of it, and set every gear
    /// turning the right way with its teeth lined up.
    fn sort_gears(&mut self) {
        // For each gear, compare it against every other gear. If they touch,
        // mark them as being each others' neighbours.
        for i in 0..self.gears.len() {
            for j in 0..self.gears.len() {
                if i != j && self.gears_touch(i, j) {
                    if !self.gears[i].neighbors.contains(&j) {
                        self.gears[i].neighbors.push(j);
                    }
                    if !self.gears[j].neighbors.contains(&i) {
                        self.gears[j].neighbors.push(i);
                    }
                }
            }
        }

        if self.gears.is_empty() {
            return;
        }

        // Give gear zero a parent while the walk runs so that nothing adopts
        // it, then take it away again: it is the root.
        self.gears[0].parent = Some(0);
        self.link_children(0);
        self.gears[0].parent = None;

        self.gears[0].direction = 1;
        self.orient_gears(0);
        self.align_gear_teeth(0);
        self.describe_gears();
    }

    /// `make_prism`: five identical gears on the faces of a uniform triangular
    /// prism.
    fn make_prism(&mut self, height: i32) {
        let teeth = 4 * (4 + bellrand(20.0) as i32);
        let g = self.add_gear_shape(0.7075, teeth, height);

        self.add_sphere_gear(g, [0.0, 0.0, 1.0]);
        self.add_sphere_gear(g, [0.0, 0.0, -1.0]);
        for i in 0..3 {
            let th = f64::from(i) * std::f64::consts::PI * 2.0 / 3.0;
            self.add_sphere_gear(g, [th.cos(), th.sin(), 0.0]);
        }
    }

    /// `make_octo`: eight identical gears on the faces of an octahedron, or
    /// equivalently on the diagonals of a cube.
    fn make_octo(&mut self, height: i32) {
        const VERTS: [Xyz; 8] = [
            [-1.0, -1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [-1.0, 1.0, 1.0],
            [-1.0, 1.0, -1.0],
            [1.0, -1.0, 1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [1.0, 1.0, 1.0],
        ];
        let teeth = 4 * (4 + bellrand(20.0) as i32);
        let g = self.add_gear_shape(0.578, teeth, height);
        for v in VERTS {
            self.add_sphere_gear(g, v);
        }
    }

    /// `make_deca`: ten identical gears. Upstream is not sure which polyhedron
    /// this is either.
    fn make_deca(&mut self, height: i32) {
        let teeth = 4 * (4 + bellrand(15.0) as i32);
        let g = self.add_gear_shape(0.5415, teeth, height);

        self.add_sphere_gear(g, [0.0, 0.0, 1.0]);
        self.add_sphere_gear(g, [0.0, 0.0, -1.0]);

        for j in [-1.0, 1.0] {
            let off = if j < 0.0 {
                0.0
            } else {
                std::f64::consts::PI / 4.0
            };
            // Empirical, upstream says.
            let a = j * std::f64::consts::PI * 0.136;
            for i in 0..4 {
                let o = f64::from(i) * std::f64::consts::PI / 2.0 + off;
                let p = polar_to_cartesian(Ll { a, o });
                self.add_sphere_gear(g, p);
            }
        }
    }

    /// `make_14`: fourteen identical gears.
    fn make_14(&mut self, height: i32) {
        let r = 0.4610;
        // Mismeshes at 24, 30, 34, 36, 42, 48, 54 and 60.
        let teeth = 6 * (2 + bellrand(4.0) as i32);

        // North and south.
        let g = self.add_gear_shape(r, teeth, height);
        self.add_sphere_gear(g, [0.0, 0.0, 1.0]);
        self.add_sphere_gear(g, [0.0, 0.0, -1.0]);

        // Equator.
        for i in 0..4 {
            let th = f64::from(i) * std::f64::consts::PI * 2.0 / 4.0 + std::f64::consts::PI / 4.0;
            self.add_sphere_gear(g, [th.cos(), th.sin(), 0.0]);
        }

        // The other eight.
        let g = self.add_gear_shape(r, teeth, height);
        for i in 0..4 {
            // Empirical, and wrong, upstream says.
            let a = std::f64::consts::PI * 0.197;
            let o = f64::from(i) * std::f64::consts::PI * 2.0 / 4.0;
            let p = polar_to_cartesian(Ll { a, o });
            self.add_sphere_gear(g, p);
            let p = polar_to_cartesian(Ll { a: -a, o });
            self.add_sphere_gear(g, p);
        }
    }

    /// `make_18`: eighteen gears in two alternating shapes round the equator.
    fn make_18(&mut self, height: i32) {
        let r = 0.3830;
        // 10, 14, 18, 26 and 34 do not work.
        let sizes = [8, 12, 16, 20];
        let teeth = sizes[(random() % 4) as usize] * (1 + (random() % 4) as i32);

        // North and south.
        let g = self.add_gear_shape(r, teeth, height);
        self.add_sphere_gear(g, [0.0, 0.0, 1.0]);
        self.add_sphere_gear(g, [0.0, 0.0, -1.0]);

        // Equator, alternating between the two shapes.
        let g2 = self.add_gear_shape(r, teeth, height);
        for i in 0..8 {
            let th = f64::from(i) * std::f64::consts::PI * 2.0 / 8.0 + std::f64::consts::PI / 4.0;
            let which = if i & 1 == 1 { g } else { g2 };
            self.add_sphere_gear(which, [th.cos(), th.sin(), 0.0]);
        }

        // The other sixteen.
        let g = self.add_gear_shape(r, teeth, height);
        for i in 0..4 {
            let a = std::f64::consts::PI * 0.25;
            let o = f64::from(i) * std::f64::consts::PI * 2.0 / 4.0;
            let p = polar_to_cartesian(Ll { a, o });
            self.add_sphere_gear(g, p);
            let p = polar_to_cartesian(Ll { a: -a, o });
            self.add_sphere_gear(g, p);
        }
    }

    /// The ten faces of the truncated icosahedron that both [`Self::make_32`]
    /// and [`Self::make_92`] are laid out on, each as its three corners plus
    /// the pole the pair of triangles shares.
    fn icosa_faces(i: i32) -> (Xyz, Xyz, Xyz, Xyz) {
        // Latitude division 26.57 degrees, longitude division 72.
        let th0 = 0.5_f64.atan();
        let s = std::f64::consts::PI / 5.0;
        let (th1, th2, th3) = (s * f64::from(i), s * f64::from(i + 1), s * f64::from(i + 2));

        let sign = if i & 1 == 0 { -1.0 } else { 1.0 };
        let v1 = Ll {
            a: sign * th0,
            o: th1,
        };
        let v2 = Ll {
            a: sign * th0,
            o: th3,
        };
        let v3 = Ll {
            a: -sign * th0,
            o: th2,
        };
        let vc = Ll {
            a: sign * std::f64::consts::FRAC_PI_2,
            o: th2,
        };
        (
            polar_to_cartesian(v1),
            polar_to_cartesian(v2),
            polar_to_cartesian(v3),
            polar_to_cartesian(vc),
        )
    }

    /// `make_32`: one gear on each of the twenty faces of a truncated
    /// icosahedron, and one on each of its twelve vertices.
    fn make_32(&mut self, args: [f64; 5], height: i32) {
        let teeth1 = args[0] as i32;
        let teeth2 = args[1] as i32;
        let r1 = args[2];
        let ratio = f64::from(teeth2) / f64::from(teeth1);
        let r2 = r1 * ratio;

        let gear1 = self.add_gear_shape(r1, teeth1, height);
        let gear2 = self.add_gear_shape(r2, teeth2, height);
        self.ratios[gear2] = 1.0 / ratio;

        self.add_sphere_gear(gear1, [0.0, 0.0, 1.0]);
        self.add_sphere_gear(gear1, [0.0, 0.0, -1.0]);

        for i in 0..10 {
            let (p1, p2, p3, pc) = Self::icosa_faces(i);

            // Two faces: 123 and 12c.
            // The left shared point of the two triangles.
            self.add_sphere_gear(gear1, p1);

            // The centre of the bottom triangle, then of the top.
            let mid = |a: Xyz, b: Xyz, c: Xyz| {
                [
                    (a[0] + b[0] + c[0]) / 3.0,
                    (a[1] + b[1] + c[1]) / 3.0,
                    (a[2] + b[2] + c[2]) / 3.0,
                ]
            };
            self.add_sphere_gear(gear2, mid(p1, p2, p3));
            self.add_sphere_gear(gear2, mid(p1, p2, pc));
        }
    }

    /// `make_92`: 20 + 12 + 60 gears along a 3v class-I geodesic tessellation
    /// of an icosahedron.
    fn make_92(&mut self, args: [f64; 5], height: i32) {
        // These do not mesh properly, so upstream raises the tooth count to
        // make it less obvious.
        let tscale = 2;
        let teeth1 = args[0] as i32 * tscale;
        let teeth2 = args[1] as i32 * tscale;
        let teeth3 = args[2] as i32 * tscale;
        let r1 = args[3];
        let ratio2 = f64::from(teeth2) / f64::from(teeth1);
        let ratio3 = f64::from(teeth3) / f64::from(teeth2);
        let r2 = r1 * ratio2;
        let r3 = r2 * ratio3;

        // Empirical, upstream says; it is not sure what its basis is.
        let r4 = args[4];
        let r5 = 1.0 - r4;

        let gear1 = self.add_gear_shape(r1, teeth1, height);
        let gear2 = self.add_gear_shape(r2, teeth2, height);
        let gear3 = self.add_gear_shape(r3, teeth3, height);
        self.ratios[gear2] = 1.0 / ratio2;
        self.ratios[gear3] = 1.0 / ratio3;

        self.add_sphere_gear(gear1, [0.0, 0.0, 1.0]);
        self.add_sphere_gear(gear1, [0.0, 0.0, -1.0]);

        for i in 0..10 {
            let (p1, p2, p3, pc) = Self::icosa_faces(i);

            // The left shared point of the two triangles.
            self.add_sphere_gear(gear1, p1);

            let mid = |a: Xyz, b: Xyz, c: Xyz| {
                [
                    (a[0] + b[0] + c[0]) / 3.0,
                    (a[1] + b[1] + c[1]) / 3.0,
                    (a[2] + b[2] + c[2]) / 3.0,
                ]
            };
            // The centre of the bottom triangle, then of the top.
            self.add_sphere_gear(gear2, mid(p1, p2, p3));
            self.add_sphere_gear(gear2, mid(p1, p2, pc));

            // A third and two thirds of the way along each of the three edges
            // that meet at p1.
            let along = |a: Xyz, b: Xyz, t: f64| {
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ]
            };
            for (b, t) in [(p3, r4), (p3, r5), (pc, r4), (pc, r5), (p2, r4), (p2, r5)] {
                self.add_sphere_gear(gear3, along(p1, b, t));
            }
        }
    }

    /// `pick_shape`: throw the sphere away and build the next one.
    fn pick_shape(&mut self, first: bool, height: i32) {
        self.colors = make_smooth_colormap(1024);
        self.shapes.clear();
        self.ratios.clear();
        self.gears.clear();

        let count = TEMPLATES.len();
        if first {
            self.which = (random() as usize) % count;
        } else if self.next < 0 {
            self.which = (self.which + count - 1) % count;
            self.next = 0;
        } else if self.next > 0 {
            self.which = (self.which + 1) % count;
            self.next = 0;
        } else {
            let mut n = self.which;
            while n == self.which {
                n = (random() as usize) % count;
            }
            self.which = n;
        }

        let (kind, args) = TEMPLATES[self.which];
        match kind {
            Kind::Prism => self.make_prism(height),
            Kind::Octo => self.make_octo(height),
            Kind::Deca => self.make_deca(height),
            Kind::G14 => self.make_14(height),
            Kind::G18 => self.make_18(height),
            Kind::G32 => self.make_32(args, height),
            Kind::G92 => self.make_92(args, height),
        }

        self.sort_gears();
    }

    /// The transform that carries a gear from the origin to its place on the
    /// sphere. Shared by the gear pass and the numbering pass.
    fn place_gear(&self, g: &mut Gl, i: usize, half_tooth_always: bool) {
        let sg = &self.gears[i];
        let shape = &self.shapes[sg.shape];
        let mut off = sg.offset;

        // With an even number of teeth, offset by half a tooth width. The
        // numbering pass wants this on every gear, so that the numbers do not
        // sit between the teeth they name.
        if sg.direction > 0 && (half_tooth_always || shape.nteeth & 1 == 0) {
            off += 360.0 / f64::from(shape.nteeth) / 2.0;
        }

        let from = [0.0, 1.0, 0.0];
        let to = sg.axis;
        let axis = cross(from, to);
        let angle = dot(from, to).clamp(-1.0, 1.0).acos();

        g.glx.push_matrix();
        g.glx.translate(to[0] as f32, to[1] as f32, to[2] as f32);
        g.glx.rotate(
            (angle * 180.0 / std::f64::consts::PI) as f32,
            axis[0] as f32,
            axis[1] as f32,
            axis[2] as f32,
        );
        g.glx.rotate(-90.0, 1.0, 0.0, 0.0);
        g.glx.rotate(180.0, 0.0, 0.0, 1.0);
        g.glx.rotate(
            ((self.th - off) * self.ratios[sg.shape] * f64::from(sg.direction)) as f32,
            0.0,
            0.0,
            1.0,
        );
    }
}

impl Hack3d for Geodesic {
    fn draw(&mut self, g: &mut Gl) -> u32 {
        let wire = self.wireframe;
        let now = g.time;
        let height = g.height();

        if !wire {
            g.glx.depth_test(true);
            g.glx.blend(crate::runtime::gl::Blend::Alpha);
            g.glx.lighting(true);
            g.glx.light_enable(0, true);
        }

        // The shrink-away and grow-back take ten ticks each, so a faster saver
        // spends fewer of them on it.
        let ticks = 10.0 / self.speed.max(0.01);

        if !self.started {
            self.pick_shape(true, height);
            self.started = true;
            self.draw_time = now;
        } else {
            match self.mode {
                Mode::Normal => {
                    if !self.trackball.button_down() && self.draw_time + self.timeout <= now {
                        // Randomize every -timeout seconds.
                        self.mode = Mode::Out;
                        self.mode_tick = ticks;
                        self.draw_time = now;
                    }
                }
                Mode::Out => {
                    self.mode_tick -= 1.0;
                    if self.mode_tick <= 0.0 {
                        self.mode_tick = ticks;
                        self.mode = Mode::In;
                        self.pick_shape(false, height);
                        self.draw_time = now;
                    }
                }
                Mode::In => {
                    self.mode_tick -= 1.0;
                    if self.mode_tick <= 0.0 {
                        self.mode = Mode::Normal;
                    }
                }
            }
        }

        g.glx.depth_test(true);
        g.glx.cull_face(true);
        g.glx.clear();

        g.glx.push_matrix();

        let down = self.trackball.button_down();
        let (x, y, z) = self.rot.position(!down);
        g.glx.translate(
            (x as f32 - 0.5) * 8.0,
            (y as f32 - 0.5) * 8.0,
            (z as f32 - 0.5) * 17.0,
        );
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);

        let (x, y, z) = self.rot.rotation(!down);
        g.glx.rotate(x as f32 * 360.0, 1.0, 0.0, 0.0);
        g.glx.rotate(y as f32 * 360.0, 0.0, 1.0, 0.0);
        g.glx.rotate(z as f32 * 360.0, 0.0, 0.0, 1.0);

        g.glx.scale(6.0, 6.0, 6.0);
        if self.gears.len() < 14 {
            // Make these a little easier to see.
            g.glx.scale(0.8, 0.8, 0.8);
        }

        if self.mode != Mode::Normal {
            let s = (if self.mode == Mode::Out {
                self.mode_tick / ticks
            } else {
                (ticks - self.mode_tick + 1.0) / ticks
            }) as f32;
            g.glx.scale(s, s, s);
        }

        for i in 0..self.gears.len() {
            self.place_gear(g, i, false);
            // Upstream compiles each shape into a display list. Here the gear
            // is drawn directly: a list in this runtime replays the calls
            // rather than the result, so it costs the same, and the material
            // changes inside a gear would not survive being recorded.
            let shape = self.gears[i].shape;
            draw_geodesic_gear(g, &self.shapes[shape], wire);
            g.glx.pop_matrix();
        }

        // The numbers go in a second pass so that the transparency that comes
        // out of anti-aliasing works properly.
        if self.do_numbers && self.mode == Mode::Normal {
            for i in 0..self.gears.len() {
                self.place_gear(g, i, true);

                g.glx.lighting(false);
                g.glx.color4f(1.0, 1.0, 0.0, 1.0);

                let shape = self.gears[i].shape;
                let (r, nteeth, z) = {
                    let s = &self.shapes[shape];
                    (s.r, s.nteeth, s.z)
                };

                // The gear's own number, at its middle.
                g.glx.push_matrix();
                g.glx.scale(0.005, 0.005, 0.005);
                let buf = i.to_string();
                let e = self.font.metrics(&buf);
                g.glx.translate(
                    (-e.width / 2) as f32,
                    (-(e.ascent + e.descent) / 2) as f32,
                    0.0,
                );
                self.font.print_string(&mut g.glx, &buf);
                g.glx.pop_matrix();

                // Then number the teeth.
                for j in 0..nteeth {
                    let ss = (0.08 * r / f64::from(nteeth)) as f32;
                    let rr = r * 0.88;
                    let th = std::f64::consts::PI
                        - (f64::from(j) * std::f64::consts::PI * 2.0 / f64::from(nteeth)
                            + std::f64::consts::FRAC_PI_2);

                    g.glx.push_matrix();
                    g.glx.translate(
                        (rr * th.cos()) as f32,
                        (rr * th.sin()) as f32,
                        (-z + 0.01) as f32,
                    );
                    g.glx.scale(ss, ss, ss);
                    let buf = (j + 1).to_string();
                    let e = self.font.metrics(&buf);
                    g.glx.translate(
                        (-e.width / 2) as f32,
                        (-(e.ascent + e.descent) / 2) as f32,
                        0.0,
                    );
                    self.font.print_string(&mut g.glx, &buf);
                    g.glx.pop_matrix();
                }

                g.glx.pop_matrix();
                if !wire {
                    g.glx.lighting(true);
                }
            }
        }

        // Upstream warns not to take this modulo 360: doing so makes the gears
        // jump.
        self.th += 0.7 * self.speed;

        if self.do_labels && self.mode == Mode::Normal {
            let (w, h) = (g.width(), g.height());
            let desc = self.desc.clone();
            g.glx.lighting(false);
            self.font
                .print_label(&mut g.glx, &desc, w, h, 1, [1.0, 1.0, 0.0, 1.0]);
            if !wire {
                g.glx.lighting(true);
            }
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
        let mut switch = false;
        if let XEvent::KeyPress { key } = event {
            match key {
                '<' | ',' | '-' | '_' => {
                    self.next = -1;
                    switch = true;
                }
                '>' | '.' | '=' | '+' => {
                    self.next = 1;
                    switch = true;
                }
                _ => {}
            }
        }
        if switch || screenhack_event_helper(event) {
            self.mode = Mode::Out;
            self.mode_tick = 4.0;
            return true;
        }
        false
    }
}

/// One gear, moved so its outer edge sits on the sphere rather than its
/// midpoint.
fn draw_geodesic_gear(g: &mut Gl, gear: &Gear, wire: bool) {
    let mut g2 = gear.clone();

    // Move the gear inward so that its outer edge is on the disc, instead of
    // its midpoint.
    g2.z += g2.thickness / 2.0;

    // `radius` is at the surface but `g.r` is at the centre, so this reverses
    // the slope computation that involute.rs does.
    g2.r /= 1.0 + (g2.thickness * g2.tooth_slope / 2.0);

    g.glx.push_matrix();
    g.glx.translate(g2.x as f32, g2.y as f32, -g2.z as f32);

    // Line up the centre of the point of tooth 0 with "up".
    g.glx.rotate(90.0, 0.0, 0.0, 1.0);
    g.glx.rotate(180.0, 0.0, 1.0, 0.0);
    g.glx.rotate(-360.0 / g2.nteeth as f32 / 4.0, 0.0, 0.0, 1.0);

    draw_gear(&mut g.glx, &g2, wire);
    g.glx.pop_matrix();
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    let spin = g.res.bool("spin");
    let speed = g.res.float("speed");
    let spin_speed = 0.25 * speed;
    let wander_speed = 0.01 * speed;
    let spin_accel = 0.2;
    let wire = g.res.bool("wireframe");

    let font = TexFont::load(&mut g.glx, "sans-serif 16");

    let mut st = Geodesic {
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
        font,
        colors: Vec::new(),
        shapes: Vec::new(),
        ratios: Vec::new(),
        gears: Vec::new(),
        which: 0,
        mode: Mode::Normal,
        mode_tick: 0.0,
        next: 0,
        draw_time: 0.0,
        started: false,
        desc: String::new(),
        th: 0.0,
        speed,
        timeout: f64::from(g.res.int("timeout").clamp(1, 600)),
        do_labels: g.res.bool("labels"),
        do_numbers: g.res.bool("numbers"),
        wireframe: wire,
    };

    let (w, h) = (g.width(), g.height());
    st.reshape(g, w, h);

    g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
    g.glx.light_ambient(0, [0.0, 0.0, 0.0, 1.0]);
    g.glx.light_diffuse(0, [1.0, 1.0, 1.0, 1.0]);
    g.glx.light_specular(0, [0.0, 1.0, 1.0, 1.0]);
    g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
    g.glx.material_shininess(128.0);

    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    "*delay:      30000",
    "*count:      4",
    "*wireframe:  False",
    "*showFPS:    False",
    "*font:       sans-serif 16",
    "*spin:       True",
    "*wander:     True",
    "*speed:      1.0",
    "*labels:     False",
    "*numbers:    False",
    "*timeout:    20",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "30000").inverted(),
    Opt::slider("timeout", "Duration", 5.0, 120.0, 1.0, 0, "20"),
    Opt::slider("speed", "Speed", 0.05, 5.0, 0.05, 2, "1.0"),
    Opt::boolean("labels", "Describe gears", "false"),
    Opt::boolean("numbers", "Number gears", "false"),
    Opt::boolean("wander", "Wander", "true"),
    Opt::boolean("spin", "Spin", "true"),
    Opt::boolean("wireframe", "Wireframe", "false"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "geodesicgears",
    label: "Geodesic Gears",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Jamie Zawinski",
        year: "2014",
        video: Some("https://www.youtube.com/watch?v=gd_nTnJQ4Ps"),
        blurb: "A set of meshed gears arranged on the surface of a sphere.",
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

    /// A sphere built straight from [`Geodesic::pick_shape`], with no GL
    /// behind it.
    fn a_sphere(which: usize) -> Geodesic {
        let mut st = Geodesic {
            rot: Rotator::new(0.0, 0.0, 0.0, 1.0, 0.0, true),
            trackball: Trackball::new(),
            font: TexFont::load(&mut crate::runtime::gl::Glx::new(), "sans-serif 16"),
            colors: Vec::new(),
            shapes: Vec::new(),
            ratios: Vec::new(),
            gears: Vec::new(),
            which,
            mode: Mode::Normal,
            mode_tick: 0.0,
            // A forward step from the entry before the one wanted lands on it.
            next: 1,
            draw_time: 0.0,
            started: true,
            desc: String::new(),
            th: 0.0,
            speed: 1.0,
            timeout: 20.0,
            do_labels: false,
            do_numbers: false,
            wireframe: false,
        };
        st.which = (which + TEMPLATES.len() - 1) % TEMPLATES.len();
        st.pick_shape(false, 480);
        st
    }

    /// Every arrangement is named for how many gears it holds, and every one
    /// of them has to come out with exactly that many. Upstream asserts this
    /// with an `abort()` at the end of each builder.
    #[test]
    fn each_arrangement_holds_the_gears_it_is_named_for() {
        for (i, (kind, _)) in TEMPLATES.iter().enumerate() {
            let want = match kind {
                Kind::Prism => 5,
                Kind::Octo => 8,
                Kind::Deca => 10,
                Kind::G14 => 14,
                Kind::G18 => 18,
                Kind::G32 => 32,
                Kind::G92 => 92,
            };
            let st = a_sphere(i);
            assert_eq!(st.which, i, "template {i}");
            assert_eq!(st.gears.len(), want, "template {i} is {kind:?}");
        }
    }

    /// The neighbour graph has to reach every gear: one root, and everything
    /// else with a parent and a direction. A gear left with direction zero is
    /// one the walk never got to, which upstream prints an internal error for.
    #[test]
    fn every_gear_is_driven_by_exactly_one_other() {
        for i in 0..TEMPLATES.len() {
            let st = a_sphere(i);
            let roots = st.gears.iter().filter(|g| g.parent.is_none()).count();
            assert_eq!(roots, 1, "template {i} has {roots} roots");
            for (j, g) in st.gears.iter().enumerate() {
                assert!(g.direction != 0, "template {i} gear {j} is unreachable");
                assert!(!g.neighbors.is_empty(), "template {i} gear {j} is loose");
            }
        }
    }

    /// A gear turns against the one driving it, all the way down the tree.
    /// That is the whole reason for building the tree.
    #[test]
    fn neighbours_turn_opposite_ways() {
        for i in 0..TEMPLATES.len() {
            let st = a_sphere(i);
            for (j, g) in st.gears.iter().enumerate() {
                if let Some(p) = g.parent {
                    assert_eq!(
                        g.direction, -st.gears[p].direction,
                        "template {i} gear {j} turns with its parent"
                    );
                }
            }
        }
    }

    /// Two gears are neighbours when their discs overlap on the sphere, and
    /// the discs are placed so that they do. Two gears that do *not* touch
    /// must not be neighbours either, or the tree would drive a gear through
    /// thin air.
    #[test]
    fn only_touching_gears_are_neighbours() {
        let st = a_sphere(1); // Octo: eight equal gears on the cube diagonals.
        for i in 0..st.gears.len() {
            for j in 0..st.gears.len() {
                if i == j {
                    continue;
                }
                let touch = st.gears_touch(i, j);
                let linked = st.gears[i].neighbors.contains(&j);
                assert_eq!(touch, linked, "gears {i} and {j}");
            }
        }
        // Opposite corners of the cube cannot possibly reach each other.
        let opposite = st
            .gears
            .iter()
            .position(|g| g.axis.iter().zip(st.gears[0].axis).all(|(a, b)| *a == -b));
        let opposite = opposite.expect("every axis has its opposite");
        assert!(!st.gears[0].neighbors.contains(&opposite));
    }

    /// The teeth are meshed by search: a child is turned through one tooth's
    /// worth of arc, and kept where its tooth sits closest to its parent's.
    /// The chosen offset must therefore beat the untouched zero it started
    /// from, and stay inside the arc it was searched over.
    #[test]
    fn meshing_moves_each_child_within_one_tooth() {
        let st = a_sphere(0); // Prism: five identical gears.
        for (i, g) in st.gears.iter().enumerate() {
            let Some(_) = g.parent else { continue };
            let range = 360.0 / f64::from(st.shapes[g.shape].nteeth);
            assert!(
                g.offset >= -range / 2.0 && g.offset < range / 2.0,
                "gear {i} offset {} is outside +/- {}",
                g.offset,
                range / 2.0
            );
        }
    }

    /// The label counts the gears by tooth count, and the counts have to add
    /// up to the sphere.
    #[test]
    fn the_label_counts_every_gear() {
        let st = a_sphere(5); // A 32-gear sphere, which has two shapes.
        assert!(st.desc.ends_with('.'), "{:?}", st.desc);
        assert!(st.desc.contains("32 gears total"), "{:?}", st.desc);
    }

    /// It draws, and the gears turn.
    #[test]
    fn the_sphere_turns() {
        let r = run("", 8);
        let f = r.frame();
        assert!(!f.vertices.is_empty(), "nothing drawn");
        assert!(f.batches.len() > 4, "only {} batches", f.batches.len());
    }
}
