//! Port of `hacks/celtic.c`.
//!
//! ```text
//! celtic, Copyright (c) 2006 Max Froumentin <max@lapin-bleu.net>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! A celtic pattern programme inspired by "Les Entrelacs Celtes", by
//! Christian Mercat, Dossier Pour La Science, no. 47, april/june 2005.
//! See <http://www.entrelacs.net/>
//! ```
//!
//! A knot is not drawn. A graph is drawn, and the knot is what falls out of it.
//!
//! Lay down a graph: a square grid, a triangular one, a wheel of orbits, or a
//! lattice of little four-pointed clusters. Then walk it by a rule that never
//! looks at the picture. Stand on an edge facing a node, take the next edge
//! round that node in the direction you are turning, cross it, and reverse your
//! turn. Keep going and you come back to where you started, having traced a
//! closed loop; start again on any edge-and-direction you have not used, and
//! when none are left the graph has been covered exactly twice, once per side of
//! every edge. Each of those loops is one strand of the knot.
//!
//! The loops are laid out as Bézier segments, one per node passed, bulging by an
//! amount taken from the angle turned through, which is what gives the strands
//! their plaited curve rather than a polygon.
//!
//! What makes it read as woven is not the strands but their shadow. A thicker
//! line in the background colour is drawn a little way ahead of each strand, so
//! wherever a strand is about to cross something already drawn it wipes a gap
//! first and then draws over it. Over and under fall out of the order the
//! drawing happens in, with no crossing ever computed.
//!
//! Upstream's `mono` is not on the panel; it reduces the whole thing to one
//! colour and the shadow trick still carries the picture.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::color::{XColor, make_smooth_colormap};
use crate::runtime::erase::{Eraser, erase_window};
use crate::runtime::{
    About, Dpy, Gc, Opt, Runner, SaverDef, Screenhack, StartArgs, XEvent, random,
    screenhack_event_helper,
};

const SQRT_3: f64 = 1.732_050_807_568_877_2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Clockwise = 0,
    Anticlockwise = 1,
}

impl Direction {
    fn flip(self) -> Self {
        match self {
            Direction::Clockwise => Direction::Anticlockwise,
            Direction::Anticlockwise => Direction::Clockwise,
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphType {
    Polar,
    Tgrid,
    Kennicott,
    Triangle,
}

struct Node {
    x: f64,
    y: f64,
    edges: Vec<usize>,
}

struct GEdge {
    node1: usize,
    node2: usize,
    angle1: f64,
    angle2: f64,
}

#[derive(Default)]
struct Graph {
    nodes: Vec<Node>,
    edges: Vec<GEdge>,
}

impl Graph {
    fn add_node(&mut self, x: f64, y: f64) -> usize {
        self.nodes.push(Node {
            x,
            y,
            edges: Vec::new(),
        });
        self.nodes.len() - 1
    }

    /// Upstream wraps a negative angle by adding 6.28 rather than a full turn,
    /// leaving it three thousandths of a radian short. That is kept rather than
    /// corrected: these angles are compared against each other to decide which
    /// edge the walk turns onto, so the imprecision can pick a different edge
    /// and therefore a different knot.
    #[allow(clippy::approx_constant)]
    fn add_edge(&mut self, n1: usize, n2: usize) {
        let (a, b) = (&self.nodes[n1], &self.nodes[n2]);
        let mut angle1 = (b.y - a.y).atan2(b.x - a.x);
        if angle1 < 0.0 {
            angle1 += 6.28;
        }
        let mut angle2 = (a.y - b.y).atan2(a.x - b.x);
        if angle2 < 0.0 {
            angle2 += 6.28;
        }
        self.edges.push(GEdge {
            node1: n1,
            node2: n2,
            angle1,
            angle2,
        });
        let e = self.edges.len() - 1;
        self.nodes[n1].edges.push(e);
        self.nodes[n2].edges.push(e);
    }

    /// The angle of edge `e` where it meets node `n`.
    fn edge_angle(&self, e: usize, n: usize) -> f64 {
        if self.edges[e].node1 == n {
            self.edges[e].angle1
        } else {
            self.edges[e].angle2
        }
    }

    fn other_node(&self, e: usize, n: usize) -> usize {
        if self.edges[e].node1 == n {
            self.edges[e].node2
        } else {
            self.edges[e].node1
        }
    }

    /// The angle swept from one edge to another around a node, going the given
    /// way.
    fn edge_angle_to(&self, e: usize, e2: usize, node: usize, dir: Direction) -> f64 {
        let a = if dir == Direction::Clockwise {
            self.edge_angle(e, node) - self.edge_angle(e2, node)
        } else {
            self.edge_angle(e2, node) - self.edge_angle(e, node)
        };
        if a < 0.0 {
            a + std::f64::consts::TAU
        } else {
            a
        }
    }

    /// The next edge round `n` after `e`, which is the one the walk turns onto.
    fn next_edge_around(&self, n: usize, e: usize, dir: Direction) -> usize {
        let mut minangle = 20.0;
        let mut next_edge = e;
        for &edge in &self.nodes[n].edges {
            if edge != e {
                let angle = self.edge_angle_to(e, edge, n, dir);
                if angle < minangle {
                    next_edge = edge;
                    minangle = angle;
                }
            }
        }
        next_edge
    }

    fn rotate(&mut self, angle: f64, cx: i32, cy: i32) {
        let (c, s) = ((angle as f32).cos() as f64, (angle as f32).sin() as f64);
        let (cx, cy) = (cx as f64, cy as f64);
        for n in &mut self.nodes {
            let (x, y) = (n.x, n.y);
            n.x = (x - cx) * c - (y - cy) * s + cx;
            n.y = (x - cx) * s + (y - cy) * c + cy;
        }
    }
}

/// A simple grid graph with its diagonals.
fn make_grid_graph(xmin: i32, ymin: i32, width: i32, height: i32, step: i32) -> Graph {
    let size = width.max(height);
    // Empirically there are two curves only if both are even, so round them.
    let nbcol = (2 + size / step) / 2 * 2;
    let nbrow = nbcol;

    let mut g = Graph::default();
    // Centre the grid.
    let xmin = xmin + (width - (nbcol - 1) * step) / 2;
    let ymin = ymin + (height - (nbrow - 1) * step) / 2;

    let mut grid = vec![0usize; (nbrow * nbcol) as usize];
    for row in 0..nbrow {
        for col in 0..nbcol {
            let x = (col * step + xmin) as f64;
            let y = (row * step + ymin) as f64;
            grid[(row + col * nbrow) as usize] = g.add_node(x, y);
        }
    }

    let at = |row: i32, col: i32| grid[(row + col * nbrow) as usize];
    let mut edges = Vec::new();
    for row in 0..nbrow {
        for col in 0..nbcol {
            if col != nbcol - 1 {
                edges.push((at(row, col), at(row, col + 1)));
            }
            if row != nbrow - 1 {
                edges.push((at(row, col), at(row + 1, col)));
            }
            if col != nbcol - 1 && row != nbrow - 1 {
                edges.push((at(row, col), at(row + 1, col + 1)));
                edges.push((at(row + 1, col), at(row, col + 1)));
            }
        }
    }
    for (a, b) in edges {
        g.add_edge(a, b);
    }
    g
}

/// A triangular lattice filling one big triangle.
fn make_triangle_graph(xmin: i32, ymin: i32, width: i32, height: i32, edge_size: i32) -> Graph {
    let l = width.min(height) as f64 / 2.0; // Circumradius of the triangle.
    let cx = xmin as f64 + width as f64 / 2.0;
    let cy = ymin as f64 + height as f64 / 2.0;
    // p2 is the bottom left vertex.
    let p2x = cx - l * SQRT_3 / 2.0;
    let p2y = cy + l / 2.0;
    let nsteps = (3.0 * l / (SQRT_3 * edge_size as f64)) as i32;
    if nsteps < 1 {
        return Graph::default();
    }

    let mut g = Graph::default();
    let w = nsteps + 1;
    let mut grid = vec![usize::MAX; (w * w) as usize];
    for row in 0..=nsteps {
        for col in 0..=nsteps {
            if row + col <= nsteps {
                let x = p2x
                    + col as f64 * l * SQRT_3 / nsteps as f64
                    + row as f64 * l * SQRT_3 / (2.0 * nsteps as f64);
                let y = p2y - row as f64 * 3.0 * l / (2.0 * nsteps as f64);
                grid[(col + row * w) as usize] = g.add_node(x, y);
            }
        }
    }

    // Upstream fills this grid indexed one way and joins it up indexed the
    // other, which transposes the lattice. Harmless, since the filled set is
    // symmetric, and kept so the figure comes out the same way round.
    let at = |row: i32, col: i32| grid[(row + col * w) as usize];
    let mut edges = Vec::new();
    for row in 0..nsteps {
        for col in 0..nsteps {
            if row + col < nsteps {
                edges.push((at(row, col), at(row, col + 1))); // Horizontal.
                edges.push((at(row, col), at(row + 1, col))); // Vertical.
                edges.push((at(row + 1, col), at(row, col + 1))); // Diagonal.
            }
        }
    }
    for (a, b) in edges {
        if a != usize::MAX && b != usize::MAX {
            g.add_edge(a, b);
        }
    }
    g
}

/// A wheel of orbits: a centre, rings of nodes around it, spokes outwards and
/// links along each ring.
fn make_polar_graph(xmin: i32, ymin: i32, width: i32, height: i32, nbp: i32, nbo: i32) -> Graph {
    let cx = (width / 2 + xmin) as f64;
    let cy = (height / 2 + ymin) as f64;
    let os = (width.min(height) / (2 * nbo)) as f64; // Orbit height.

    let mut g = Graph::default();
    let mut grid = vec![0usize; (1 + nbp * nbo) as usize];
    grid[0] = g.add_node(cx, cy);

    for o in 0..nbo {
        for p in 0..nbp {
            let a = p as f64 * std::f64::consts::TAU / nbp as f64;
            grid[(1 + o * nbp + p) as usize] = g.add_node(
                cx + (o + 1) as f64 * os * a.sin(),
                cy + (o + 1) as f64 * os * a.cos(),
            );
        }
    }

    let mut edges = Vec::new();
    for o in 0..nbo {
        for p in 0..nbp {
            let here = grid[(1 + o * nbp + p) as usize];
            if o == 0 {
                edges.push((here, grid[0])); // Link the first orbit to the centre.
            } else {
                edges.push((here, grid[(1 + (o - 1) * nbp + p) as usize]));
            }
            edges.push((here, grid[(1 + o * nbp + (p + 1) % nbp) as usize]));
        }
    }
    for (a, b) in edges {
        g.add_edge(a, b);
    }
    g
}

/// A grid of clusters shaped like a diamond with a cross through it, after one
/// of the motifs from the Kennicott bible.
fn make_kennicott_graph(
    xmin: i32,
    ymin: i32,
    width: i32,
    height: i32,
    step: i32,
    cluster_size: i32,
) -> Graph {
    let size = width.max(height);
    let nbcol = (1 + size / step) / 2 * 2;
    let nbrow = nbcol;

    let mut g = Graph::default();
    let xmin = xmin + (width - (nbcol - 1) * step) / 2;
    let ymin = ymin + (height - (nbrow - 1) * step) / 2;

    let mut grid = vec![0usize; (5 * nbrow * nbcol).max(1) as usize];
    let cs = cluster_size as f64;
    let mut edges = Vec::new();
    for row in 0..nbrow {
        for col in 0..nbcol {
            let ci = (5 * (row + col * nbrow)) as usize;
            let x = (col * step + xmin) as f64;
            let y = (row * step + ymin) as f64;

            grid[ci] = g.add_node(x, y);
            grid[ci + 1] = g.add_node(x + cs, y);
            grid[ci + 2] = g.add_node(x, y - cs);
            grid[ci + 3] = g.add_node(x - cs, y);
            grid[ci + 4] = g.add_node(x, y + cs);

            for k in 1..=4 {
                edges.push((grid[ci], grid[ci + k]));
            }
            edges.push((grid[ci + 1], grid[ci + 2]));
            edges.push((grid[ci + 2], grid[ci + 3]));
            edges.push((grid[ci + 3], grid[ci + 4]));
            edges.push((grid[ci + 4], grid[ci + 1]));
        }
    }

    // Join neighbouring clusters point to point.
    for row in 0..nbrow {
        for col in 0..nbcol {
            let ci = (5 * (row + col * nbrow)) as usize;
            if col != nbcol - 1 {
                edges.push((
                    grid[ci + 1],
                    grid[(5 * (row + (col + 1) * nbrow)) as usize + 3],
                ));
            }
            if row != nbrow - 1 {
                edges.push((
                    grid[ci + 4],
                    grid[(5 * (row + 1 + col * nbrow)) as usize + 2],
                ));
            }
        }
    }
    for (a, b) in edges {
        g.add_edge(a, b);
    }
    g
}

/// One cubic Bézier: the piece of a strand that passes one node.
#[derive(Clone, Copy)]
struct SplineSegment {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
    x4: f64,
    y4: f64,
}

struct Strand {
    segments: Vec<SplineSegment>,
    color: usize,
}

impl Strand {
    /// Where the strand is at `t`, which runs from nought to one over the whole
    /// closed loop.
    fn value_at(&self, t: f64) -> (f64, f64, usize) {
        let n = self.segments.len();
        let mut si = (t * n as f64).floor() as usize;
        if si >= n {
            si = n - 1;
        }
        let tt = t * n as f64 - si as f64;
        let s = &self.segments[si];
        let u = 1.0 - tt;
        let x = s.x1 * u * u * u
            + 3.0 * s.x2 * tt * u * u
            + 3.0 * s.x3 * tt * tt * u
            + s.x4 * tt * tt * tt;
        let y = s.y1 * u * u * u
            + 3.0 * s.y2 * tt * u * u
            + 3.0 * s.y3 * tt * tt * u
            + s.y4 * tt * tt * tt;
        (x, y, si)
    }
}

struct Celtic {
    ncolors: usize,
    colors: Vec<XColor>,
    gc: Gc,
    shadow_gc: Gc,
    gc_graph: Gc,

    show_graph: bool,
    graph: Option<Graph>,
    strands: Vec<Option<Strand>>,
    /// Which of the two sides of each edge the walk has already used.
    edge_couple: Vec<[bool; 2]>,

    width: i32,
    height: i32,
    delay: u32,
    delay2: u32,
    reset: bool,
    force_reset: bool,
    t: f64,
    eraser: Option<Eraser>,

    curve_width: i32,
    shadow_width: i32,
    shape1: f64,
    shape2: f64,
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let show_graph = d.res.bool("showGraph");
    let mut ncolors = d.res.int("ncolors").max(1) as usize;
    let foreground = d.res.pixel("foreground");
    let background = d.res.pixel("background");

    let mut colors: Vec<XColor> = if d.res.bool("mono") {
        ncolors = 1;
        vec![XColor::from_pixel(foreground)]
    } else {
        make_smooth_colormap(ncolors)
    };
    if colors.len() < 2 {
        ncolors = 1;
        colors = vec![XColor::from_pixel(foreground)];
    } else {
        colors[0] = XColor::from_pixel(foreground);
        colors[1] = XColor::from_pixel(background);
    }

    Box::new(Celtic {
        ncolors,
        gc: Gc::new(colors[0].pixel, background),
        shadow_gc: Gc::new(colors[colors.len().min(2) - 1].pixel, background),
        gc_graph: Gc::new(colors[0].pixel, background),
        colors,
        show_graph,
        graph: None,
        strands: Vec::new(),
        edge_couple: Vec::new(),
        width: d.width(),
        height: d.height(),
        delay: d.res.int("delay").max(0) as u32,
        delay2: (d.res.int("delay2").max(0) as u32).saturating_mul(1_000_000),
        reset: false,
        force_reset: false,
        t: 0.0,
        eraser: None,
        curve_width: 1,
        shadow_width: 1,
        shape1: 0.0,
        shape2: 0.0,
    })
}

impl Celtic {
    /// One Bézier through a node, from the middle of one edge to the middle of
    /// the next, bulging by how far the walk turned.
    fn spline_segment(
        &self,
        g: &Graph,
        node: usize,
        edge1: usize,
        edge2: usize,
        direction: Direction,
    ) -> SplineSegment {
        let mid = |e: usize| {
            let (a, b) = (&g.nodes[g.edges[e].node1], &g.nodes[g.edges[e].node2]);
            ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
        };
        let (x1, y1) = mid(edge1);
        let (x4, y4) = mid(edge2);
        let n = &g.nodes[node];

        let alpha = g.edge_angle_to(edge1, edge2, node, direction) * self.shape1;
        let beta = self.shape2;

        // One control point sticks out to the left of the way in and the other
        // to the right of the way out, which is what makes the strand bend
        // around the node rather than cut the corner.
        let sign = if direction == Direction::Anticlockwise {
            1.0
        } else {
            -1.0
        };
        let i1x = sign * alpha * (n.y - y1) + x1;
        let i1y = -sign * alpha * (n.x - x1) + y1;
        let i2x = -sign * alpha * (n.y - y4) + x4;
        let i2y = sign * alpha * (n.x - x4) + y4;
        let x2 = sign * beta * (y1 - i1y) + i1x;
        let y2 = -sign * beta * (x1 - i1x) + i1y;
        let x3 = -sign * beta * (y4 - i2y) + i2x;
        let y3 = sign * beta * (x4 - i2x) + i2y;

        SplineSegment {
            x1,
            y1,
            x2,
            y2,
            x3,
            y3,
            x4,
            y4,
        }
    }

    fn next_unfilled_couple(&self) -> Option<(usize, Direction)> {
        for (i, c) in self.edge_couple.iter().enumerate() {
            if !c[Direction::Clockwise.index()] {
                return Some((i, Direction::Clockwise));
            } else if !c[Direction::Anticlockwise.index()] {
                return Some((i, Direction::Anticlockwise));
            }
        }
        None
    }

    /// Walk the graph into closed loops. Each loop uses one side of each edge
    /// it passes, so the walk terminates when every side has been used once.
    fn make_curves(&mut self, g: &Graph) {
        self.strands.clear();
        self.edge_couple = vec![[false; 2]; g.edges.len()];

        while let Some((first_edge, first_direction)) = self.next_unfilled_couple() {
            let color = if self.ncolors > 2 {
                (random() as usize) % (self.ncolors - 2) + 2
            } else {
                0
            };
            let mut segments = Vec::new();

            let mut current_edge = first_edge;
            let first_node = g.edges[current_edge].node1;
            let mut current_node = first_node;
            let mut current_direction = first_direction;

            loop {
                self.edge_couple[current_edge][current_direction.index()] = true;
                let next_edge = g.next_edge_around(current_node, current_edge, current_direction);
                segments.push(self.spline_segment(
                    g,
                    current_node,
                    current_edge,
                    next_edge,
                    current_direction,
                ));

                // Cross the edge and turn the other way.
                current_edge = next_edge;
                current_node = g.other_node(next_edge, current_node);
                current_direction = current_direction.flip();

                if current_node == first_node
                    && current_edge == first_edge
                    && current_direction == first_direction
                {
                    break;
                }
            }

            // A two-segment loop is just a point.
            if segments.len() == 2 {
                self.strands.push(None);
            } else {
                self.strands.push(Some(Strand { segments, color }));
            }
        }
    }

    fn new_pattern(&mut self, d: &mut Dpy) {
        self.curve_width = (random() % 5) as i32 + 4;
        self.shadow_width = self.curve_width + 4;
        self.shape1 = (15 + random() % 15) as f64 / 10.0 - 1.0;
        self.shape2 = (15 + random() % 15) as f64 / 10.0 - 1.0;
        let mut edge_size = 10 * (random() % 5) as i32 + 20;
        let angle = (random() % 360) as f64 * std::f64::consts::TAU / 360.0;
        let mut margin = (random() % 8) as i32 * 100 - 600;
        let mut cluster_size = 0;
        let mut nb_orbits = 0;
        let mut nb_nodes_per_orbit = 0;

        let gtype = match random() % 4 {
            0 => {
                // Upstream's `random()%1*2-1.0` is always -1, so these two
                // shapes only ever come out negative for a square grid.
                self.shape1 = -((random() % 10) as f64 + 3.0) / 10.0;
                self.shape2 = -((random() % 10) as f64 + 3.0) / 10.0;
                edge_size = 10 * (random() % 5) as i32 + 50;
                GraphType::Tgrid
            }
            1 => {
                self.shape1 = (random() % 20) as f64 / 10.0 - 1.0;
                self.shape2 = (random() % 20) as f64 / 10.0 - 1.0;
                edge_size = 10 * (random() % 3) as i32 + 70;
                cluster_size = (edge_size as f64 / (3.0 + (random() % 10) as f64) - 1.0) as i32;
                GraphType::Kennicott
            }
            2 => {
                edge_size = 10 * (random() % 5) as i32 + 60;
                margin = (random() % 10) as i32 * 100 - 900;
                GraphType::Triangle
            }
            _ => {
                nb_orbits = 2 + (random() % 10) as i32;
                nb_nodes_per_orbit = 4 + (random() % 10) as i32;
                GraphType::Polar
            }
        };

        let (w, h) = (self.width - 2 * margin, self.height - 2 * margin);
        let edge_size = edge_size.max(1);
        let mut g = match gtype {
            GraphType::Tgrid => make_grid_graph(margin, margin, w, h, edge_size),
            GraphType::Kennicott => {
                make_kennicott_graph(margin, margin, w, h, edge_size, cluster_size.max(1))
            }
            GraphType::Triangle => make_triangle_graph(margin, margin, w, h, edge_size),
            GraphType::Polar => {
                make_polar_graph(margin, margin, w, h, nb_nodes_per_orbit, nb_orbits)
            }
        };

        g.rotate(angle, self.width / 2, self.height / 2);

        if self.show_graph {
            let fg = self.colors[0].pixel;
            self.gc_graph.set_foreground(fg);
            for n in &g.nodes {
                let (x, y) = (n.x.round() as i32 - 5, n.y.round() as i32 - 5);
                d.win().draw_arc(&self.gc_graph, x, y, 10, 10, 0, 360 * 64);
            }
            for e in &g.edges {
                let (a, b) = (&g.nodes[e.node1], &g.nodes[e.node2]);
                d.win().draw_line(
                    &self.gc_graph,
                    a.x as i32,
                    a.y as i32,
                    b.x as i32,
                    b.y as i32,
                );
            }
        }

        self.make_curves(&g);
        self.graph = Some(g);
        self.t = 0.0;
    }

    /// Draw a hundred steps' worth of every strand at once, with each strand's
    /// shadow running a little way ahead of it.
    fn animate(&mut self, d: &mut Dpy) {
        // Set the step (or the delay) as a function of the spline length, so
        // that drawing speed is constant, is upstream's outstanding to-do.
        let step = 0.0001;
        let shadow2 = (self.shadow_width * self.shadow_width) as f64;
        self.gc.set_line_width(self.curve_width);
        self.shadow_gc.set_line_width(self.shadow_width);

        let mut t = self.t;
        let single = self.strands.iter().filter(|s| s.is_some()).count() == 1;

        for _ in 0..100 {
            if t >= 1.0 {
                break;
            }
            for i in 0..self.strands.len() {
                let Some(s) = &self.strands[i] else { continue };
                let (x, y, segment) = s.value_at(t % 1.0);
                let (x2, y2, _) = s.value_at((t + step) % 1.0);

                // Look ahead for the shadow segment.
                let mut t2 = t + step;
                if t2 <= 1.0 {
                    let (mut x3, mut y3, _) = s.value_at(t2 % 1.0);
                    while t2 + step < 1.0 && (x3 - x2) * (x3 - x2) + (y3 - y2) * (y3 - y2) < shadow2
                    {
                        t2 += step;
                        let v = s.value_at(t2 % 1.0);
                        x3 = v.0;
                        y3 = v.1;
                    }
                    let (x4, y4, _) = s.value_at((t2 + step) % 1.0);
                    d.win().draw_line(
                        &self.shadow_gc,
                        x3.round() as i32,
                        y3.round() as i32,
                        x4.round() as i32,
                        y4.round() as i32,
                    );
                }

                let p = if single && self.ncolors > 3 {
                    self.colors[segment % (self.ncolors - 3) + 2].pixel
                } else {
                    self.colors[s.color.min(self.colors.len() - 1)].pixel
                };
                self.gc.set_foreground(p);
                d.win().draw_line(
                    &self.gc,
                    x.round() as i32,
                    y.round() as i32,
                    x2.round() as i32,
                    y2.round() as i32,
                );
            }
            t += step;
        }
        self.t = t;

        if t >= 1.0 {
            self.reset = true;

            // Redraw the tail to remove the shadow that spilled past the end.
            for i in 0..self.strands.len() {
                let Some(s) = &self.strands[i] else { continue };
                let mut offset = step;
                let p = self.colors[s.color.min(self.colors.len() - 1)].pixel;
                self.gc.set_foreground(p);
                let (x, y, _) = s.value_at(t % 1.0);
                let (mut x2, mut y2, _) = s.value_at((t - offset).rem_euclid(1.0));
                while (x2 - x) * (x2 - x) + (y2 - y) * (y2 - y) < shadow2 && offset < 1.0 {
                    offset += step;
                    let v = s.value_at((t - offset).rem_euclid(1.0));
                    x2 = v.0;
                    y2 = v.1;
                }
                d.win().draw_line(
                    &self.gc,
                    x.round() as i32,
                    y.round() as i32,
                    x2.round() as i32,
                    y2.round() as i32,
                );
            }
        }
    }
}

impl Screenhack for Celtic {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        if self.eraser.is_some() {
            self.eraser = erase_window(d, self.eraser.take());
            return 10000;
        }

        if self.reset || self.force_reset {
            let delay = if self.force_reset { 0 } else { self.delay2 };
            self.reset = false;
            self.force_reset = false;
            self.t = 1.0;
            self.graph = None;
            self.strands.clear();

            // Recolour each time.
            if self.ncolors > 2 {
                let fg = self.colors[0].pixel;
                let bg = self.colors[1].pixel;
                self.colors = make_smooth_colormap(self.ncolors);
                self.colors[0] = XColor::from_pixel(fg);
                self.colors[1] = XColor::from_pixel(bg);
                self.shadow_gc.set_foreground(bg);
            }

            self.eraser = erase_window(d, self.eraser.take());
            return delay;
        }

        if self.graph.is_none() {
            self.new_pattern(d);
        }

        self.animate(d);
        self.delay
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        if screenhack_event_helper(event) {
            self.force_reset = true;
            return true;
        }
        false
    }
}

const DEFAULTS: &[&str] = &[
    ".background: black",
    ".foreground: #333333",
    "*fpsSolid: true",
    "*ncolors: 20",
    "*delay: 10000",
    "*delay2: 5",
    "*showGraph: False",
    "*mono: False",
];

const OPTS: &[Opt] = &[
    Opt::slider("delay", "Frame rate", 0.0, 100_000.0, 1000.0, 0, "10000").inverted(),
    Opt::slider("delay2", "Linger", 0.0, 10.0, 1.0, 0, "5"),
    Opt::boolean("showGraph", "Draw graph", "False"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "celtic",
    label: "Celtic",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Max Froumentin",
        year: "2005",
        video: Some("https://www.youtube.com/watch?v=PnX60AAoTdw"),
        blurb: "Repeatedly draws random Celtic cross-stitch patterns.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
