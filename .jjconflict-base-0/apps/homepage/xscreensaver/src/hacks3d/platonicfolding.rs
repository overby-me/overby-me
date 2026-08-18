//! Port of `hacks/glx/platonicfolding.c`.
//!
//! ```text
//! platonicfolding --- Displays the unfolding and folding of the Platonic
//! solids.
//!
//! Copyright (c) 2025-2026 Carsten Steger <carsten@mirsanmir.org>.
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
//! ```
//!
//! The five Platonic solids, opening out into a flat net and closing up
//! again. Every unfolding of a Platonic solid lies flat without overlapping
//! itself, which is not true of polyhedra in general; the tetrahedron has
//! sixteen of them, the cube and the octahedron three hundred and eighty-four
//! each, and the dodecahedron and icosahedron over five million.
//!
//! A net is a spanning tree of the graph whose vertices are the faces and
//! whose edges join faces that touch, so upstream picks one at random by
//! giving every edge a random weight and taking the minimum spanning tree.
//! That is Kruskal's algorithm over a disjoint-set forest, and then a
//! depth-first walk to turn the chosen edges into a tree of faces hanging off
//! one another. Each face's place in the flat net is worked out once, and the
//! fold is a rotation about the edge it hangs from, either all edges at once
//! or one after another.
//!
//! Upstream has two colourings. Faces get their own colours from where their
//! centre lands under a random rotation, which is what this draws. The other
//! wraps the Earth round the solid by a gnomonic projection, with the day and
//! night maps either side of the terminator and magma inside, and it lives
//! entirely in a fragment shader: upstream's own build without GLSL falls
//! back to face colours, which is what happens here too. Working it out per
//! vertex would not do, because a face of a tetrahedron covers a quarter of
//! the sky and the projection is nothing like linear across it. Baking one
//! texture per face would, since the sun is fixed for the whole run, and that
//! is what to do if this ever gets the other half.
//!
//! Since both settings of the coloration knob would come to the same thing,
//! the knob is not on the panel: one that does nothing is worse than one that
//! is not there.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver3d;
use crate::runtime::gl::{Blend, Shape};
use crate::runtime::{
    About, Gl, Hack3d, Opt, Runner3d, SaverDef, StartArgs, Trackball, XEvent, frand,
};
use std::f32::consts::PI;

const SQRT_2_2: f32 = std::f32::consts::FRAC_1_SQRT_2;
const SQRT_2_4: f32 = 0.353_553_4;
const SQRT_3_2: f32 = 0.866_025_4;
const COS_36: f32 = 0.809_017;
const SIN_36: f32 = 0.587_785_25;
const COS_72: f32 = 0.309_017;
const SIN_72: f32 = 0.951_056_5;
const DODECA_IN_RAD: f32 = 1.309_017;
const ICOSA_IN_RAD: f32 = 1.309_017;

/// A four by four matrix, row major, as upstream writes them.
type Mat = [[f32; 4]; 4];

const IDENTITY: Mat = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// `mult_matrix`: `c = c * m`.
fn mult_matrix(c: &mut Mat, m: &Mat) {
    let t = *c;
    for j in 0..4 {
        for i in 0..4 {
            c[i][j] = t[i][0] * m[0][j] + t[i][1] * m[1][j] + t[i][2] * m[2][j] + t[i][3] * m[3][j];
        }
    }
}

/// `translate_matrix`: add a translation from the right.
fn translate_matrix(m: &mut Mat, t: [f32; 3]) {
    for row in m.iter_mut() {
        row[3] += t[0] * row[0] + t[1] * row[1] + t[2] * row[2];
    }
}

/// `rotate_xy_matrix`: add a rotation in the xy plane, in degrees.
fn rotate_xy_matrix(m: &mut Mat, phi: f32) {
    let (s, c) = (phi * PI / 180.0).sin_cos();
    for row in m.iter_mut() {
        let (u, v) = (row[0], row[1]);
        row[0] = c * u + s * v;
        row[1] = -s * u + c * v;
    }
}

fn mult_matrix_vector(m: &Mat, v: [f32; 4]) -> [f32; 4] {
    let mut o = [0.0f32; 4];
    for (i, out) in o.iter_mut().enumerate() {
        *out = (0..4).map(|j| m[i][j] * v[j]).sum();
    }
    o
}

fn normalize(v: &mut [f32; 4]) {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n != 0.0 {
        for c in v.iter_mut().take(3) {
            *c /= n;
        }
    }
}

/// `rnd_rot_matrix`: a rotation spread evenly over all of them, which is what
/// makes the face colours look unrelated to each other.
fn rnd_rot_matrix() -> Mat {
    let theta = frand(2.0 * std::f64::consts::PI) as f32;
    let phi = frand(2.0 * std::f64::consts::PI) as f32;
    let z = frand(2.0) as f32;
    let r = z.sqrt();
    let vx = r * phi.sin();
    let vy = r * phi.cos();
    let vz = (2.0 - z).sqrt();
    let (st, ct) = theta.sin_cos();
    let sx = vx * ct - vy * st;
    let sy = vx * st + vy * ct;
    let mut m = IDENTITY;
    m[0] = [vx * sx - ct, vx * sy - st, vx * vz, 0.0];
    m[1] = [vy * sx + st, vy * sy - ct, vy * vz, 0.0];
    m[2] = [vz * sx, vz * sy, 1.0 - z, 0.0];
    m
}

const EASING_QUINTIC: i32 = 0;
const EASING_ACCEL: i32 = 1;
const EASING_DECEL: i32 = 2;

/// `ease`, for values of `t` between nought and `max`.
fn ease(t: f32, max: f32, easing: i32) -> f32 {
    let s = t / max;
    let e = match easing {
        EASING_QUINTIC => ((6.0 * s - 15.0) * s + 10.0) * s * s * s,
        EASING_ACCEL => s * s * (2.0 - s),
        EASING_DECEL => s * (1.0 + s * (1.0 - s)),
        _ => 0.0,
    };
    max * e
}

/// One entry of a solid's face adjacency graph: two faces that touch, and
/// which of each one's edges they touch along.
#[derive(Clone, Copy)]
struct Edge {
    src: usize,
    dst: usize,
    src_edge: i32,
    dst_edge: i32,
}

const fn e(src: usize, dst: usize, src_edge: i32, dst_edge: i32) -> Edge {
    Edge {
        src,
        dst,
        src_edge,
        dst_edge,
    }
}

#[rustfmt::skip]
const TETRAHEDRON_EDGES: &[Edge] = &[
    e(0,1,0,0), e(0,2,2,0), e(0,3,1,0), e(1,2,1,2), e(1,3,2,1), e(2,3,1,2),
];

#[rustfmt::skip]
const HEXAHEDRON_EDGES: &[Edge] = &[
    e(0,1,0,0), e(0,2,1,0), e(0,3,2,0), e(0,4,3,0), e(1,2,3,1), e(1,4,1,3),
    e(1,5,2,0), e(2,3,3,1), e(2,5,2,3), e(3,4,3,1), e(3,5,2,2), e(4,5,2,1),
];

#[rustfmt::skip]
const OCTAHEDRON_EDGES: &[Edge] = &[
    e(0,1,0,0), e(0,2,1,0), e(0,3,2,0), e(1,4,1,2), e(1,5,2,1), e(2,5,1,0),
    e(2,6,2,0), e(3,4,2,0), e(3,6,1,2), e(4,7,1,1), e(5,7,2,0), e(6,7,1,2),
];

#[rustfmt::skip]
const DODECAHEDRON_EDGES: &[Edge] = &[
    e(0,1,0,0), e(0,2,1,4), e(0,3,2,0), e(0,4,3,3), e(0,5,4,1), e(1,2,4,0),
    e(1,5,1,0), e(1,6,2,0), e(1,7,3,0), e(2,3,3,1), e(2,7,1,4), e(2,8,2,2),
    e(3,4,4,4), e(3,8,2,1), e(3,9,3,0), e(4,5,2,2), e(4,9,0,4), e(4,10,1,4),
    e(5,6,4,1), e(5,10,3,3), e(6,7,4,1), e(6,10,2,2), e(6,11,3,3), e(7,8,3,3),
    e(7,11,2,2), e(8,9,0,1), e(8,11,4,1), e(9,10,3,0), e(9,11,2,0), e(10,11,1,4),
];

#[rustfmt::skip]
const ICOSAHEDRON_EDGES: &[Edge] = &[
    e(0,1,2,0), e(0,2,0,0), e(0,3,1,0), e(1,4,2,0), e(1,9,1,0), e(2,5,1,2),
    e(2,6,2,1), e(3,7,1,0), e(3,8,2,0), e(4,5,2,0), e(4,10,1,0), e(5,15,1,2),
    e(6,7,0,1), e(6,14,2,1), e(7,13,2,0), e(8,9,2,1), e(8,12,1,2), e(9,11,2,1),
    e(10,11,1,0), e(10,18,2,0), e(11,17,2,1), e(12,13,0,2), e(12,17,1,2),
    e(13,16,1,0), e(14,15,2,1), e(14,16,0,1), e(15,18,0,2), e(16,19,2,0),
    e(17,19,0,2), e(18,19,1,1),
];

/// Everything about one of the five solids: its faces' adjacency, the shape
/// of a face, how far a fold turns, and how far away to stand.
struct Solid {
    edges: &'static [Edge],
    faces: usize,
    verts: &'static [[f32; 4]],
    max_angle: f32,
    eye: f32,
}

#[rustfmt::skip]
const SOLIDS: [Solid; 5] = [
    Solid {
        edges: TETRAHEDRON_EDGES, faces: 4,
        verts: &[
            [ 1.0,      0.0, -SQRT_2_4, 1.0],
            [-0.5,  SQRT_3_2, -SQRT_2_4, 1.0],
            [-0.5, -SQRT_3_2, -SQRT_2_4, 1.0],
        ],
        max_angle: 109.471_22, eye: 6.0,
    },
    Solid {
        edges: HEXAHEDRON_EDGES, faces: 6,
        verts: &[
            [ SQRT_2_2, -SQRT_2_2, -SQRT_2_2, 1.0],
            [ SQRT_2_2,  SQRT_2_2, -SQRT_2_2, 1.0],
            [-SQRT_2_2,  SQRT_2_2, -SQRT_2_2, 1.0],
            [-SQRT_2_2, -SQRT_2_2, -SQRT_2_2, 1.0],
        ],
        max_angle: 90.0, eye: 6.0,
    },
    Solid {
        edges: OCTAHEDRON_EDGES, faces: 8,
        verts: &[
            [ 1.0,      0.0, -SQRT_2_2, 1.0],
            [-0.5,  SQRT_3_2, -SQRT_2_2, 1.0],
            [-0.5, -SQRT_3_2, -SQRT_2_2, 1.0],
        ],
        max_angle: 70.528_78, eye: 6.0,
    },
    Solid {
        edges: DODECAHEDRON_EDGES, faces: 12,
        verts: &[
            [    1.0,     0.0, -DODECA_IN_RAD, 1.0],
            [ COS_72,  SIN_72, -DODECA_IN_RAD, 1.0],
            [-COS_36,  SIN_36, -DODECA_IN_RAD, 1.0],
            [-COS_36, -SIN_36, -DODECA_IN_RAD, 1.0],
            [ COS_72, -SIN_72, -DODECA_IN_RAD, 1.0],
        ],
        max_angle: 63.434_95, eye: 12.0,
    },
    Solid {
        edges: ICOSAHEDRON_EDGES, faces: 20,
        verts: &[
            [ 1.0,      0.0, -ICOSA_IN_RAD, 1.0],
            [-0.5,  SQRT_3_2, -ICOSA_IN_RAD, 1.0],
            [-0.5, -SQRT_3_2, -ICOSA_IN_RAD, 1.0],
        ],
        max_angle: 41.810_315, eye: 12.0,
    },
];

/// One face of the net, hanging off its parent by an edge.
#[derive(Clone, Copy)]
struct Node {
    next: Option<usize>,
    child: Option<usize>,
    /// Which face of the solid this is, so that a face keeps its colour
    /// across different nets of the same solid.
    polygon_index: usize,
    edge_parent: i32,
    edge_self: i32,
    /// Where this face sits relative to its parent when the net is flat.
    unfold_pose: Mat,
    /// Which of the fold angles turns this face, or none for the root.
    fold_angle_index: i32,
    /// Where it sits relative to its parent as the fold stands.
    fold_pose: Mat,
}

/// `create_random_polyhedron_unfolding`: give every edge of the face graph a
/// random weight, take the minimum spanning tree, and hang the faces off one
/// another in the order a depth-first walk finds them.
fn random_unfolding(solid: &Solid) -> Vec<Node> {
    let n = solid.faces;
    let mut edges: Vec<(f32, Edge)> = solid
        .edges
        .iter()
        .map(|&e| (frand(1.0) as f32, e))
        .collect();
    edges.sort_by(|a, b| a.0.total_cmp(&b.0));

    // Kruskal, over a disjoint-set forest.
    let mut parent: Vec<usize> = (0..n).collect();
    let mut rank = vec![0usize; n];
    fn root(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut mst: Vec<Edge> = Vec::with_capacity(n - 1);
    for (_, edge) in edges {
        if mst.len() == n - 1 {
            break;
        }
        let (a, b) = (root(&mut parent, edge.src), root(&mut parent, edge.dst));
        if a == b {
            continue;
        }
        mst.push(edge);
        match rank[a].cmp(&rank[b]) {
            std::cmp::Ordering::Less => parent[a] = b,
            std::cmp::Ordering::Greater => parent[b] = a,
            std::cmp::Ordering::Equal => {
                parent[b] = a;
                rank[a] += 1;
            }
        }
    }

    // The chosen edges as an adjacency list, both ways round, in the order
    // upstream builds it: each new neighbour goes on the front of the list.
    let mut adj: Vec<Vec<(usize, i32, i32)>> = vec![Vec::new(); n];
    for edge in &mst {
        adj[edge.dst].insert(0, (edge.src, edge.dst_edge, edge.src_edge));
        adj[edge.src].insert(0, (edge.dst, edge.src_edge, edge.dst_edge));
    }

    // And the depth-first walk that turns it into a tree.
    let mut nodes: Vec<Node> = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    nodes.push(Node {
        next: None,
        child: None,
        polygon_index: 0,
        edge_parent: -1,
        edge_self: -1,
        unfold_pose: IDENTITY,
        fold_angle_index: -1,
        fold_pose: IDENTITY,
    });
    seen[0] = true;
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        let face = nodes[i].polygon_index;
        let mut last: Option<usize> = None;
        for &(v, edge_parent, edge_self) in &adj[face] {
            if seen[v] {
                continue;
            }
            seen[v] = true;
            nodes.push(Node {
                next: None,
                child: None,
                polygon_index: v,
                edge_parent,
                edge_self,
                unfold_pose: IDENTITY,
                fold_angle_index: -1,
                fold_pose: IDENTITY,
            });
            let k = nodes.len() - 1;
            match last {
                None => nodes[i].child = Some(k),
                Some(l) => nodes[l].next = Some(k),
            }
            last = Some(k);
            stack.push(k);
        }
    }
    nodes
}

/// `determine_unfolding_poses`: where each face sits relative to its parent
/// once the net is flat, and which fold angle turns it.
fn determine_unfolding_poses(nodes: &mut [Node], verts: &[[f32; 4]]) {
    let num = verts.len();
    let mut ai = 0;
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        let node = nodes[i];
        if node.edge_parent >= 0 && node.edge_self >= 0 {
            nodes[i].fold_angle_index = ai;
            ai += 1;
            let p1 = node.edge_parent as usize;
            let p2 = (p1 + 1) % num;
            let s1 = node.edge_self as usize;
            let s2 = (s1 + 1) % num;

            /* We have to take into account that, since both polygons must be
            oriented counterclockwise, the edge in the transformed polygon
            must be taken in the opposite direction to compute the correct
            rotation angle. */
            let mut vp = [0.0f32; 4];
            let mut vs = [0.0f32; 4];
            for k in 0..3 {
                vp[k] = verts[p2][k] - verts[p1][k];
                vs[k] = verts[s1][k] - verts[s2][k];
            }
            normalize(&mut vp);
            normalize(&mut vs);
            let cz = vs[0] * vp[1] - vs[1] * vp[0];
            let dp = vs[0] * vp[0] + vs[1] * vp[1] + vs[2] * vp[2];
            let phi = (180.0 / PI) * cz.atan2(dp);

            /* The midpoint of the edge in the parent polygon, doubled;
            everything lies in one plane when flat, so the z is dropped. */
            let t = [
                verts[p1][0] + verts[p2][0],
                verts[p1][1] + verts[p2][1],
                0.0,
            ];
            let mut m = IDENTITY;
            translate_matrix(&mut m, t);
            rotate_xy_matrix(&mut m, phi);
            nodes[i].unfold_pose = m;
        } else {
            nodes[i].fold_angle_index = -1;
            nodes[i].unfold_pose = IDENTITY;
        }
        if let Some(c) = node.child {
            stack.push(c);
        }
        if let Some(nx) = node.next {
            stack.push(nx);
        }
    }
}

/// `compute_fold_pose`: turn a face about the edge it hangs from.
fn compute_fold_pose(node: &mut Node, verts: &[[f32; 4]], angles: &[f32], max_angle: f32) {
    let num = verts.len();
    if node.fold_angle_index >= 0 {
        let phi = angles[node.fold_angle_index as usize];
        let fold_angle = ease(phi, max_angle, EASING_QUINTIC);
        let i1 = node.edge_self as usize;
        let i2 = (i1 + 1) % num;

        /* The midpoint of the edge of the polygon that is adjacent to the
        parent polygon. */
        let t1 = mult_matrix_vector(&node.unfold_pose, verts[i1]);
        let t2 = mult_matrix_vector(&node.unfold_pose, verts[i2]);
        let t = [
            0.5 * (t1[0] + t2[0]),
            0.5 * (t1[1] + t2[1]),
            0.5 * (t1[2] + t2[2]),
        ];
        let mt = [-t[0], -t[1], -t[2]];

        /* The direction that stays fixed during the rotation. */
        let mut c = [t2[0] - t1[0], t2[1] - t1[1], t2[2] - t1[2], 0.0];
        normalize(&mut c);

        /* The plane the rotation happens in: the translation flattened, and
        the z direction. */
        let mut a = [t[0], t[1], 0.0, 0.0];
        normalize(&mut a);
        let b = [0.0f32, 0.0, 1.0];

        let mut mats = IDENTITY;
        let mut matst = IDENTITY;
        for i in 0..3 {
            mats[i][0] = a[i];
            mats[i][1] = b[i];
            mats[i][2] = c[i];
            matst[0][i] = a[i];
            matst[1][i] = b[i];
            matst[2][i] = c[i];
        }

        let mut matr = IDENTITY;
        translate_matrix(&mut matr, t);
        mult_matrix(&mut matr, &mats);
        rotate_xy_matrix(&mut matr, fold_angle);
        mult_matrix(&mut matr, &matst);
        translate_matrix(&mut matr, mt);
        mult_matrix(&mut node.fold_pose, &matr);
    }
    let up = node.unfold_pose;
    mult_matrix(&mut node.fold_pose, &up);
}

/// The animation states, in the order they run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Anim {
    Init,
    Appear,
    Disappear,
    UnfoldJnt,
    FoldJnt,
    UnfoldSep,
    FoldSep,
}

struct Folding {
    trackball: Trackball,
    rotate: bool,
    num_foldings: i32,

    solid: usize,
    nodes: Vec<Node>,
    /// One colour per face of the net, in tree order.
    colors: Vec<[f32; 4]>,
    color_matrix: Mat,

    angle: Vec<f32>,
    max_angle: f32,
    delta_angle: f32,
    fold_angle: i32,
    num_fold_angles: usize,

    alpha: f32,
    beta: f32,
    delta: f32,
    delta_delta: f32,
    eye: f32,

    anim_state: Anim,
    anim_step: i32,
    anim_num_steps: i32,
    anim_remaining: i32,
    poly_pos: [f32; 3],
    spoly_pos: [f32; 3],
    dpoly_pos: [f32; 3],
}

impl Folding {
    /// Walk the tree in the order upstream does, handing each face its
    /// folded pose. The parent's pose is copied into a child before it is
    /// pushed, and into a sibling from the pose the parent had on entry.
    fn walk(&mut self, angles: &[f32], mut visit: impl FnMut(usize, &Mat)) {
        let verts = SOLIDS[self.solid].verts;
        let max_angle = self.max_angle;
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            if self.nodes[i].edge_parent < 0 || self.nodes[i].edge_self < 0 {
                self.nodes[i].fold_pose = IDENTITY;
            }
            let matp = self.nodes[i].fold_pose;
            let mut node = self.nodes[i];
            compute_fold_pose(&mut node, verts, angles, max_angle);
            self.nodes[i] = node;
            visit(i, &node.fold_pose);

            if let Some(c) = node.child {
                self.nodes[c].fold_pose = node.fold_pose;
                stack.push(c);
            }
            if let Some(nx) = node.next {
                self.nodes[nx].fold_pose = matp;
                stack.push(nx);
            }
        }
    }

    /// `determine_polygon_color_data`: a face's colour comes from where the
    /// middle of it lands, on the closed solid, under a random rotation.
    fn determine_colors(&mut self) {
        let verts = SOLIDS[self.solid].verts;
        let angles = vec![self.max_angle; self.angle.len()];
        let cm = self.color_matrix;
        let mut out = vec![[0.0f32; 4]; self.nodes.len()];
        self.walk(&angles, |i, pose| {
            let mut c = [0.0f32, 0.0, 0.0, 1.0];
            for v in verts {
                let tv = mult_matrix_vector(pose, *v);
                for k in 0..3 {
                    c[k] += tv[k];
                }
            }
            let mut cr = mult_matrix_vector(&cm, c);
            normalize(&mut cr);
            out[i] = [
                0.5 * (cr[0] + 1.0),
                0.5 * (cr[1] + 1.0),
                0.5 * (cr[2] + 1.0),
                1.0,
            ];
        });
        self.colors = out;
    }

    /// `init_polygon_unfolding`: a fresh random net of the current solid.
    fn init_unfolding(&mut self) {
        let s = &SOLIDS[self.solid];
        self.max_angle = s.max_angle;
        self.delta_angle = s.max_angle / 30.0;
        self.num_fold_angles = s.faces - 1;
        self.eye = s.eye;
        self.nodes = random_unfolding(s);
        determine_unfolding_poses(&mut self.nodes, s.verts);
        self.determine_colors();
    }
}

fn init(g: &mut Gl) -> Box<dyn Hack3d> {
    // Upstream picks the solid, the net, the colours and everything else in
    // the first frame's `Init` state, so there is nothing to set up here but
    // the knobs. `solid` starts outside the range so that the first pick,
    // which refuses to repeat the last solid, can be any of them.
    let foldings = g.res.string("foldings").to_string();
    Box::new(Folding {
        trackball: Trackball::new(),
        rotate: g.res.bool("rotate"),
        num_foldings: foldings.parse().unwrap_or(-1),
        solid: SOLIDS.len(),
        nodes: Vec::new(),
        colors: Vec::new(),
        color_matrix: IDENTITY,
        angle: vec![0.0; 19],
        max_angle: 90.0,
        delta_angle: 3.0,
        fold_angle: 0,
        num_fold_angles: 0,
        alpha: 300.0,
        beta: 0.0,
        delta: 0.0,
        delta_delta: 0.5,
        eye: 6.0,
        anim_state: Anim::Init,
        anim_step: 0,
        anim_num_steps: 180,
        anim_remaining: 0,
        poly_pos: [0.0; 3],
        spoly_pos: [0.0; 3],
        dpoly_pos: [0.0; 3],
    })
}

impl Hack3d for Folding {
    fn reshape(&mut self, _g: &mut Gl, _width: i32, _height: i32) {
        // Upstream keeps only the aspect ratio here and sets the projection
        // when it draws.
    }

    fn event(&mut self, g: &mut Gl, event: &XEvent) -> bool {
        self.trackball.event(event, g.width(), g.height())
    }

    fn draw(&mut self, g: &mut Gl) -> u32 {
        let aspect = g.width() as f32 / g.height() as f32;
        if !self.trackball.button_down() {
            self.step(aspect);
        }

        g.glx.viewport(0, 0, g.width(), g.height());
        g.glx.clear();
        g.glx.matrix_mode_projection();
        g.glx.load_identity();
        if aspect >= 1.0 {
            g.glx.perspective(45.0, aspect, 0.1, 30.0);
        } else {
            let fovy = 360.0 / PI * ((45.0 * PI / 360.0).tan() / aspect).atan();
            g.glx.perspective(fovy, aspect, 0.1, 30.0);
        }
        g.glx.matrix_mode_modelview();
        g.glx.load_identity();

        /* In the unfolded state all visible triangles face one way and in
        the folded state the other, so nothing may be culled. */
        g.glx.cull_face(false);
        g.glx.depth_test(true);
        g.glx.blend(Blend::Off);
        g.glx.front_face_cw(false);
        g.glx.lighting(true);
        g.glx.light_enable(0, true);
        g.glx.light_position(0, 1.0, 1.0, 1.0, 0.0);
        g.glx.light_ambient(0, [0.3, 0.3, 0.3, 1.0]);
        g.glx.light_diffuse(0, [0.7, 0.7, 0.7, 1.0]);
        g.glx.light_specular(0, [0.75, 0.75, 0.75, 1.0]);
        g.glx.material_specular([1.0, 1.0, 1.0, 1.0]);
        g.glx.material_shininess(30.0);
        g.glx.color_material(true);

        g.glx
            .look_at([0.0, 0.0, self.eye], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        g.glx
            .translate(self.poly_pos[0], self.poly_pos[1], self.poly_pos[2]);
        let m = self.trackball.matrix();
        g.glx.mult_matrix(m);
        g.glx.rotate(self.alpha, 1.0, 0.0, 0.0);
        g.glx.rotate(self.beta, 0.0, 1.0, 0.0);
        g.glx.rotate(self.delta, 0.0, 0.0, 1.0);

        if !self.nodes.is_empty() {
            let verts = SOLIDS[self.solid].verts;
            let normal = [0.0f32, 0.0, 1.0, 0.0];
            let angles = self.angle.clone();
            let colors = self.colors.clone();
            /// One face ready to draw: its colour, its normal, and its
            /// corners, all in the pose the fold has put it in.
            type Face = ([f32; 4], [f32; 4], Vec<[f32; 4]>);
            let mut runs: Vec<Face> = Vec::with_capacity(self.nodes.len());
            self.walk(&angles, |i, pose| {
                let tn = mult_matrix_vector(pose, normal);
                let vs = verts.iter().map(|v| mult_matrix_vector(pose, *v)).collect();
                runs.push((colors[i], tn, vs));
            });
            for (c, n, vs) in runs {
                g.glx.color4f(c[0], c[1], c[2], c[3]);
                g.glx.normal3f(n[0], n[1], n[2]);
                g.glx.begin(Shape::Polygon);
                for v in vs {
                    g.glx.vertex3f(v[0], v[1], v[2]);
                }
                g.glx.end();
            }
        }

        g.res.int("delay") as u32
    }
}

impl Folding {
    /// `display_platonicfolding`'s state machine.
    fn step(&mut self, aspect: f32) {
        if self.anim_state == Anim::Init {
            /* Whether the north or the south pole faces upwards, which only
            the earth colouring shows, and which way the fold goes. */
            let north_up = frand(1.0) < 0.5;
            self.alpha = if frand(1.0) < 0.5 { 300.0 } else { 120.0 };
            self.beta = 0.0;
            self.delta = frand(360.0) as f32;
            self.delta_delta = if north_up { 0.5 } else { -0.5 };

            /* A random solid, never the same one twice running. */
            let mut poly = self.solid;
            while poly == self.solid {
                poly = frand(5.0).floor() as usize;
            }
            self.solid = poly;
            self.color_matrix = rnd_rot_matrix();
            self.init_unfolding();

            self.anim_remaining = if self.num_foldings > 0 {
                self.num_foldings
            } else {
                match poly {
                    0 => 3 + frand(3.0).floor() as i32,
                    1 | 2 => 4 + frand(5.0).floor() as i32,
                    _ => 6 + frand(7.0).floor() as i32,
                }
            };

            for i in 0..self.angle.len() {
                self.angle[i] = if i < self.num_fold_angles {
                    self.max_angle
                } else {
                    0.0
                };
            }
            self.fold_angle = 0;

            /* Where it comes in from and goes out to. */
            let d = self.eye * (2.0 / 3.0);
            if aspect >= 1.0 {
                self.spoly_pos = [0.0, -d, 0.0];
                self.dpoly_pos = [0.0, d, 0.0];
            } else {
                self.spoly_pos = [-d, 0.0, 0.0];
                self.dpoly_pos = [d, 0.0, 0.0];
            }
            self.poly_pos = self.spoly_pos;
            self.anim_num_steps = 180;
            self.anim_step = 0;
            self.anim_state = Anim::Appear;
        }

        match self.anim_state {
            Anim::Init => {}
            Anim::Appear | Anim::Disappear => {
                let t = self.anim_step as f32 / self.anim_num_steps as f32;
                let t = if self.anim_state == Anim::Appear {
                    ease(t, 1.0, EASING_DECEL)
                } else {
                    1.0 + ease(t, 1.0, EASING_ACCEL)
                };
                for k in 0..3 {
                    self.poly_pos[k] = self.spoly_pos[k] + t * self.dpoly_pos[k];
                }
                self.spin();
                self.anim_step += 1;
                if self.anim_step > self.anim_num_steps {
                    self.anim_state = if self.anim_state == Anim::Appear {
                        self.next_unfold()
                    } else {
                        Anim::Init
                    };
                }
            }
            Anim::UnfoldJnt | Anim::FoldJnt => {
                let out = self.anim_state == Anim::UnfoldJnt;
                let mut change = false;
                for i in 0..self.num_fold_angles {
                    if out {
                        self.angle[i] -= self.delta_angle / 3.0;
                        if self.angle[i] < 0.0 {
                            self.angle[i] = 0.0;
                            change = true;
                        }
                    } else {
                        self.angle[i] += self.delta_angle / 3.0;
                        if self.angle[i] > self.max_angle {
                            self.angle[i] = self.max_angle;
                            change = true;
                        }
                    }
                }
                self.spin();
                if change {
                    self.anim_state = if out { Anim::FoldJnt } else { self.finished() };
                }
            }
            Anim::UnfoldSep | Anim::FoldSep => {
                let out = self.anim_state == Anim::UnfoldSep;
                let mut change = false;
                let i = self.fold_angle as usize;
                if out {
                    self.angle[i] -= self.delta_angle;
                    if self.angle[i] < 0.0 {
                        self.angle[i] = 0.0;
                        self.fold_angle += 1;
                        if self.fold_angle >= self.num_fold_angles as i32 {
                            self.fold_angle = self.num_fold_angles as i32 - 1;
                            change = true;
                        }
                    }
                } else {
                    self.angle[i] += self.delta_angle;
                    if self.angle[i] > self.max_angle {
                        self.angle[i] = self.max_angle;
                        self.fold_angle -= 1;
                        if self.fold_angle < 0 {
                            self.fold_angle = 0;
                            change = true;
                        }
                    }
                }
                self.spin();
                if change {
                    self.anim_state = if out { Anim::FoldSep } else { self.finished() };
                }
            }
        }
    }

    fn spin(&mut self) {
        if self.rotate {
            self.delta += self.delta_delta;
        }
    }

    /// Separate unfolding is used in one fourth of the cases.
    fn next_unfold(&self) -> Anim {
        if frand(1.0) < 0.25 {
            Anim::UnfoldSep
        } else {
            Anim::UnfoldJnt
        }
    }

    /// One more net of the same solid, or away it goes.
    fn finished(&mut self) -> Anim {
        self.anim_remaining -= 1;
        if self.anim_remaining > 0 {
            self.init_unfolding();
            self.next_unfold()
        } else {
            self.anim_num_steps = 180;
            self.anim_step = 0;
            Anim::Disappear
        }
    }
}

const DEFAULTS: &[&str] = &[
    "*delay:       25000",
    "*showFPS:     False",
    "*rotate:      True",
    "*foldings:    random",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "25000").inverted(),
    Opt::slider("foldings", "Foldings", -1.0, 20.0, 1.0, 0, "-1"),
    Opt::boolean("rotate", "Rotate", "true"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "platonicfolding",
    label: "Platonic Folding",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Carsten Steger",
        year: "2025",
        video: Some("https://www.youtube.com/watch?v=4TH0rrz2Pbc"),
        blurb: "The unfolding and folding of the Platonic solids. Every \
                unfolding of a Platonic solid lies flat without overlapping \
                itself; the dodecahedron and icosahedron have over five \
                million of them each.",
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

    #[test]
    fn a_net_is_a_spanning_tree_of_the_faces() {
        // Every face appears exactly once, joined to the rest by an edge the
        // two of them share, and only the root has no parent.
        for (si, s) in SOLIDS.iter().enumerate() {
            for _ in 0..20 {
                let nodes = random_unfolding(s);
                assert_eq!(nodes.len(), s.faces, "solid {si}");
                let mut seen = vec![false; s.faces];
                let mut roots = 0;
                for n in &nodes {
                    assert!(!seen[n.polygon_index], "solid {si}: a face twice");
                    seen[n.polygon_index] = true;
                    if n.edge_parent < 0 {
                        roots += 1;
                    } else {
                        assert!((n.edge_parent as usize) < s.verts.len());
                        assert!((n.edge_self as usize) < s.verts.len());
                    }
                }
                assert_eq!(roots, 1, "solid {si}: {roots} roots");
                assert!(seen.iter().all(|&b| b));
            }
        }
    }

    #[test]
    fn the_faces_of_a_closed_solid_are_all_the_same_distance_out() {
        // Folded all the way, the middle of every face has to be the same
        // distance from the centre: that is what makes it a Platonic solid,
        // and it is the strongest check there is on the fold arithmetic.
        for (si, _) in SOLIDS.iter().enumerate() {
            let mut f = bare();
            f.solid = si;
            f.init_unfolding();
            let angles = vec![f.max_angle; f.angle.len()];
            let verts = SOLIDS[si].verts;
            let mut radii = Vec::new();
            f.walk(&angles, |_, pose| {
                let mut c = [0.0f32; 3];
                for v in verts {
                    let tv = mult_matrix_vector(pose, *v);
                    for k in 0..3 {
                        c[k] += tv[k] / verts.len() as f32;
                    }
                }
                radii.push((c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt());
            });
            assert_eq!(radii.len(), SOLIDS[si].faces);
            let lo = radii.iter().copied().fold(f32::MAX, f32::min);
            let hi = radii.iter().copied().fold(0.0f32, f32::max);
            assert!(hi - lo < 1e-3, "solid {si}: radii {lo} to {hi}");
            assert!(lo > 0.3, "solid {si}: the faces collapsed to {lo}");
        }
    }

    #[test]
    fn the_flat_net_lies_in_one_plane() {
        // Unfolded all the way, every vertex has to have the same z.
        for (si, _) in SOLIDS.iter().enumerate() {
            let mut f = bare();
            f.solid = si;
            f.init_unfolding();
            let angles = vec![0.0f32; f.angle.len()];
            let verts = SOLIDS[si].verts;
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            f.walk(&angles, |_, pose| {
                for v in verts {
                    let tv = mult_matrix_vector(pose, *v);
                    lo = lo.min(tv[2]);
                    hi = hi.max(tv[2]);
                }
            });
            assert!(hi - lo < 1e-3, "solid {si}: the net is {} deep", hi - lo);
        }
    }

    fn bare() -> Folding {
        Folding {
            trackball: Trackball::new(),
            rotate: false,
            num_foldings: -1,
            solid: 0,
            nodes: Vec::new(),
            colors: Vec::new(),
            color_matrix: IDENTITY,
            angle: vec![0.0; 19],
            max_angle: 90.0,
            delta_angle: 3.0,
            fold_angle: 0,
            num_fold_angles: 0,
            alpha: 300.0,
            beta: 0.0,
            delta: 0.0,
            delta_delta: 0.5,
            eye: 6.0,
            anim_state: Anim::Init,
            anim_step: 0,
            anim_num_steps: 180,
            anim_remaining: 0,
            poly_pos: [0.0; 3],
            spoly_pos: [0.0; 3],
            dpoly_pos: [0.0; 3],
        }
    }

    #[test]
    fn it_shows_every_solid_and_keeps_folding() {
        let mut r = start(StartArgs::new(640, 480, "", 20260812));
        let mut faces = std::collections::BTreeSet::new();
        for i in 0..6000 {
            r.step();
            let f = r.frame();
            assert!(!f.batches.is_empty(), "nothing drawn on frame {i}");
            faces.insert(f.batches.len());
        }
        // Four, six, eight, twelve or twenty polygons, depending on which
        // solid is up.
        assert!(faces.len() > 2, "only {faces:?} ever drawn");
        assert!(
            faces.iter().all(|&n| [4, 6, 8, 12, 20].contains(&n)),
            "{faces:?}"
        );
    }
}
