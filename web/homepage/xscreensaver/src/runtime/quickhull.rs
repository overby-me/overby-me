//! Port of `hacks/glx/quickhull.c`.
//!
//! ```text
//! quickhull, Copyright (c) 2016 Karim Naaji, karim.naaji@gmail.com
//! https://github.com/karimnaaji/3d-quickhull
//!
//! LICENCE:
//!  The MIT License (MIT)
//!
//!  Copyright (c) 2016 Karim Naaji, karim.naaji@gmail.com
//!
//!  Permission is hereby granted, free of charge, to any person obtaining a
//!  copy of this software and associated documentation files (the
//!  "Software"), to deal in the Software without restriction, including
//!  without limitation the rights to use, copy, modify, merge, publish,
//!  distribute, sublicense, and/or sell copies of the Software, and to permit
//!  persons to whom the Software is furnished to do so, subject to the
//!  following conditions:
//!
//!  The above copyright notice and this permission notice shall be included in
//!  all copies or substantial portions of the Software.
//!
//!  THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
//!  IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
//!  FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
//!  THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
//!  LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
//!  FROM, OUT OF OR IN CONNECTION WITH THE
//!
//! REFERENCES:
//!  [1] http://box2d.org/files/GDC2014/DirkGregorius_ImplementingQuickHull.pdf
//!  [2] http://www.cs.smith.edu/~orourke/books/compgeom.html
//!  [3] http://www.flipcode.com/archives/The_Half-Edge_Data_Structure.shtml
//!  [4] http://doc.cgal.org/latest/HalfedgeDS/index.html
//!  [5] http://thomasdiewald.com/blog/?p=1888
//!  [6] https://fgiesen.wordpress.com/2012/02/21/half-edge-based-mesh-representations-theory/
//!
//! HISTORY:
//!  - 25-Feb-2018: jwz: adapted for xscreensaver
//!  - 1.0.1 (2016-11-01): Various improvements over epsilon issues and
//!            degenerate faces
//!            Debug functionalities to test final results dynamically
//!            API to export hull meshes in OBJ files
//!  - 1.0   (2016-09-10): Initial
//! ```
//!
//! The convex hull of a cloud of points: the shape a rubber sheet would take
//! if it were shrunk onto them. `crumbler` is what wants it, since a Voronoi
//! chunk is a set of points and the piece you see is their hull.
//!
//! Quickhull starts from a tetrahedron made of six extreme points, hands every
//! remaining point to the first face that can see it, and then repeats one
//! step until no face has any points left: take the face's furthest point,
//! walk outwards from that face over every neighbour that can also see it, and
//! the boundary of that region is the *horizon*. Everything inside the horizon
//! is now buried, so it is thrown away and replaced with a fan of triangles
//! from the horizon up to the new point, and the buried faces' points are
//! handed on to whichever new face can see them.
//!
//! The mesh is half edges: each triangle owns three of them, each knows the
//! next and previous around its own face and the one facing it across the
//! shared edge, and walking a face's neighbours is following those links.
//!
//! Two things are done differently from the C. It allocates faces and edges
//! for `n * (n - 1)` triangles up front, which is gigabytes for the point
//! counts `crumbler` uses and only works because a system that overcommits
//! never gives out the pages; here they are vectors that grow to the few
//! thousand actually used. And its duplicate-point pass is a quadratic scan
//! that shuffles the array down on every removal, which is minutes of work at
//! the higher densities; here the same rule, keep a point unless an earlier
//! kept point is within epsilon of it on all three axes, is answered with a
//! grid of epsilon-sized cells. The C loop skips a point after each removal
//! and so leaves some near-duplicates in, which the grid does not, but the
//! clouds this is given are random and contain no duplicates at all.

/// A point, and also a direction: upstream's `qh_vertex_t` is both.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vertex {
    pub fn new(x: f64, y: f64, z: f64) -> Vertex {
        Vertex { x, y, z }
    }

    fn sub(self, b: Vertex) -> Vertex {
        Vertex::new(self.x - b.x, self.y - b.y, self.z - b.z)
    }

    fn add(self, b: Vertex) -> Vertex {
        Vertex::new(self.x + b.x, self.y + b.y, self.z + b.z)
    }

    fn multiply(self, v: f64) -> Vertex {
        Vertex::new(self.x * v, self.y * v, self.z * v)
    }

    fn length2(self) -> f64 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }

    fn dot(self, b: Vertex) -> f64 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }

    fn cross(self, b: Vertex) -> Vertex {
        Vertex::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }

    fn normalize(self) -> Vertex {
        self.multiply(1.0 / self.length2().sqrt())
    }

    fn equals_epsilon(self, b: Vertex, epsilon: f64) -> bool {
        (self.x - b.x).abs() <= epsilon
            && (self.y - b.y).abs() <= epsilon
            && (self.z - b.z).abs() <= epsilon
    }
}

/// One triangle of the hull, with the normal that face carries.
///
/// Upstream returns a `qh_mesh_t` of four parallel arrays, but its indices are
/// `0, 1, 2, 3, ...` and its normal indices are `0, 1, 2, ...`, so the mesh is
/// a plain soup of triangles with a normal each. That is what this is.
#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub normal: Vertex,
    pub vertices: [Vertex; 3],
}

const QH_FLT_MAX: f64 = 1e+37;
const QH_FLT_EPS: f64 = 1E-5;

#[derive(Clone, Copy, Debug)]
struct HalfEdge {
    /// Index of the opposite half edge.
    opposite_he: i64,
    /// Index of the next half edge.
    next_he: i64,
    /// Index of the previous half edge.
    previous_he: i64,
    /// Index of the current half edge.
    he: i64,
    /// Index of the next vertex.
    to_vertex: i64,
    /// Index of the ajacent face.
    adjacent_face: i64,
}

impl HalfEdge {
    fn new(he: i64) -> HalfEdge {
        HalfEdge {
            adjacent_face: -1,
            he,
            next_he: -1,
            opposite_he: -1,
            to_vertex: -1,
            previous_he: -1,
        }
    }
}

#[derive(Clone, Debug)]
struct Face {
    /// The points still to be assigned that this face can see.
    iset: Vec<i64>,
    normal: Vertex,
    centroid: Vertex,
    edges: [i64; 3],
    face: i64,
    sdist: f64,
    visitededges: i32,
}

struct Context {
    faces: Vec<Face>,
    edges: Vec<HalfEdge>,
    vertices: Vec<Vertex>,
    facestack: Vec<i64>,
    scratch: Vec<i64>,
    horizonedges: Vec<i64>,
    newhorizonedges: Vec<i64>,
    valid: Vec<bool>,
}

/// `qh__pop_stack`, which answers -1 rather than failing when it is empty.
fn pop_stack(stack: &mut Vec<i64>) -> i64 {
    stack.pop().unwrap_or(-1)
}

fn find_6eps(vertices: &[Vertex]) -> [i64; 6] {
    let mut minxy = QH_FLT_MAX;
    let mut minxz = QH_FLT_MAX;
    let mut minyz = QH_FLT_MAX;

    let mut maxxy = -QH_FLT_MAX;
    let mut maxxz = -QH_FLT_MAX;
    let mut maxyz = -QH_FLT_MAX;

    let mut eps = [0i64; 6];

    for (i, v) in vertices.iter().enumerate() {
        let i = i as i64;
        if v.z < minxy {
            eps[0] = i;
            minxy = v.z;
        }
        if v.y < minxz {
            eps[1] = i;
            minxz = v.y;
        }
        if v.x < minyz {
            eps[2] = i;
            minyz = v.x;
        }
        if v.z > maxxy {
            eps[3] = i;
            maxxy = v.z;
        }
        if v.y > maxxz {
            eps[4] = i;
            maxxz = v.y;
        }
        if v.x > maxyz {
            eps[5] = i;
            maxyz = v.x;
        }
    }
    eps
}

fn vertex_segment_length2(p: Vertex, a: Vertex, b: Vertex) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let dz = b.z - a.z;

    let d = dx * dx + dy * dy + dz * dz;

    let (mut x, mut y, mut z) = (a.x, a.y, a.z);

    if d != 0.0 {
        let t = ((p.x - a.x) * dx + (p.y - a.y) * dy + (p.z - a.z) * dz) / d;

        if t > 1.0 {
            x = b.x;
            y = b.y;
            z = b.z;
        } else if t > 0.0 {
            x += dx * t;
            y += dy * t;
            z += dz * t;
        }
    }

    let dx = p.x - x;
    let dy = p.y - y;
    let dz = p.z - z;

    dx * dx + dy * dy + dz * dz
}

fn find_2dps_6eps(vertices: &[Vertex], eps: &[i64; 6]) -> (usize, usize) {
    let mut max = -QH_FLT_MAX;
    let (mut ii, mut jj) = (0, 0);

    for i in 0..6 {
        for j in 0..6 {
            if i == j {
                continue;
            }
            let d = vertices[eps[i] as usize].sub(vertices[eps[j] as usize]);
            let d2 = d.length2();
            if d2 > max {
                ii = i;
                jj = j;
                max = d2;
            }
        }
    }
    (ii, jj)
}

fn dist_point_plane(v: Vertex, normal: Vertex, sdist: f64) -> f64 {
    (v.dot(normal) - sdist).abs()
}

impl Context {
    fn face_centroid(&self, vertices: [i64; 3]) -> Vertex {
        let mut centroid = Vertex::default();
        for v in vertices {
            centroid = centroid.add(self.vertices[v as usize]);
        }
        centroid.multiply(1.0 / 3.0)
    }

    fn next_edge(&mut self) -> usize {
        let he = self.edges.len();
        self.edges.push(HalfEdge::new(he as i64));
        he
    }

    fn next_face(&mut self) -> usize {
        let n = self.faces.len();
        self.faces.push(Face {
            iset: Vec::new(),
            normal: Vertex::default(),
            centroid: Vertex::default(),
            edges: [-1; 3],
            face: n as i64,
            sdist: 0.0,
            visitededges: 0,
        });
        self.valid.push(true);
        n
    }

    fn edge_vec3(&self, edge: usize) -> Vertex {
        let prevhe = self.edges[self.edges[edge].previous_he as usize];
        let v0 = self.vertices[prevhe.to_vertex as usize];
        let v1 = self.vertices[self.edges[edge].to_vertex as usize];
        v1.sub(v0).normalize()
    }

    fn face_init(&mut self, face: usize, vertices: [i64; 3]) {
        let e0 = self.next_edge();
        let e1 = self.next_edge();
        let e2 = self.next_edge();

        self.edges[e2].to_vertex = vertices[0];
        self.edges[e0].to_vertex = vertices[1];
        self.edges[e1].to_vertex = vertices[2];

        self.edges[e0].next_he = e1 as i64;
        self.edges[e2].previous_he = e1 as i64;
        self.faces[face].edges[1] = e1 as i64;

        self.edges[e1].next_he = e2 as i64;
        self.edges[e0].previous_he = e2 as i64;
        self.faces[face].edges[2] = e2 as i64;
        let v1 = self.edge_vec3(e2);

        self.edges[e2].next_he = e0 as i64;
        self.edges[e1].previous_he = e0 as i64;
        self.faces[face].edges[0] = e0 as i64;
        let v0 = self.edge_vec3(e0);

        let f = self.faces[face].face;
        self.edges[e2].adjacent_face = f;
        self.edges[e1].adjacent_face = f;
        self.edges[e0].adjacent_face = f;

        let v1 = v1.multiply(-1.0);
        let normal = v0.cross(v1).normalize();

        let centroid = self.face_centroid(vertices);
        let face = &mut self.faces[face];
        face.centroid = centroid;
        face.sdist = normal.dot(centroid);
        face.normal = normal;
        face.iset.clear();
        face.visitededges = 0;
    }

    fn tetrahedron_basis(&self) -> [i64; 3] {
        let eps = find_6eps(&self.vertices);
        let (j, k) = find_2dps_6eps(&self.vertices, &eps);

        let mut max = -QH_FLT_MAX;
        let mut l = 0;
        for i in 0..6 {
            if i == j || i == k {
                continue;
            }
            let d2 = vertex_segment_length2(
                self.vertices[eps[i] as usize],
                self.vertices[eps[j] as usize],
                self.vertices[eps[k] as usize],
            );
            if d2 > max {
                max = d2;
                l = i;
            }
        }

        [eps[j], eps[k], eps[l]]
    }

    /// The index *into `indices`* of the point furthest from the plane, or
    /// the vertex index itself when `indices` is `None`.
    fn furthest_point_from_plane(
        &self,
        indices: Option<&[i64]>,
        nindices: usize,
        normal: Vertex,
        sdist: f64,
    ) -> i64 {
        let mut j = -1;
        let mut max = -QH_FLT_MAX;

        for i in 0..nindices {
            let index = match indices {
                Some(ix) => ix[i],
                None => i as i64,
            };
            let dist = dist_point_plane(self.vertices[index as usize], normal, sdist);
            if dist > max {
                j = i as i64;
                max = dist;
            }
        }
        j
    }

    fn face_can_see_vertex(&self, face: usize, v: Vertex) -> bool {
        let tov = v.sub(self.faces[face].centroid);
        tov.dot(self.faces[face].normal) > 0.0
    }

    /// As above, but a point within epsilon of the plane counts as seen, and
    /// is *nudged* out along the normal so that it stays seen. Upstream really
    /// does move the caller's point, and the caller really is the context's
    /// own vertex array.
    fn face_can_see_vertex_epsilon(&mut self, face: usize, vertex: usize, epsilon: f64) -> bool {
        let tov = self.vertices[vertex].sub(self.faces[face].centroid);
        let dot = tov.dot(self.faces[face].normal);

        if dot > epsilon {
            return true;
        }
        let dot = dot.abs();
        if dot <= epsilon && dot >= 0.0 {
            /* allow epsilon degeneration along the face normal */
            let n = self.faces[face].normal.multiply(epsilon);
            self.vertices[vertex] = self.vertices[vertex].add(n);
            return true;
        }
        false
    }

    fn build_hull(&mut self, epsilon: f64) {
        let mut topface = pop_stack(&mut self.facestack);

        while topface != -1 {
            let tf = topface as usize;
            if !self.valid[tf] || self.faces[tf].iset.is_empty() {
                topface = pop_stack(&mut self.facestack);
                continue;
            }

            let fvi = self.furthest_point_from_plane(
                Some(&self.faces[tf].iset),
                self.faces[tf].iset.len(),
                self.faces[tf].normal,
                self.faces[tf].sdist,
            );
            let fv = self.vertices[self.faces[tf].iset[fvi as usize] as usize];

            /* Reset visited flag for faces */
            for f in &mut self.faces {
                f.visitededges = 0;
            }

            /* Find horizon edge */
            {
                let mut tovisit = topface;

                /* Release scratch */
                self.scratch.clear();

                while tovisit != -1 {
                    let tv = tovisit as usize;
                    if self.faces[tv].visitededges >= 3 {
                        self.valid[tv] = false;
                        tovisit = pop_stack(&mut self.scratch);
                        continue;
                    }

                    let edgeindex = self.faces[tv].edges[self.faces[tv].visitededges as usize];
                    self.faces[tv].visitededges += 1;

                    let edge = self.edges[edgeindex as usize];
                    let oppedge = self.edges[edge.opposite_he as usize];
                    let adjface = oppedge.adjacent_face;

                    if !self.valid[adjface as usize] {
                        continue;
                    }

                    if !self.face_can_see_vertex(adjface as usize, fv) {
                        self.horizonedges.push(edge.he);
                    } else {
                        self.valid[tv] = false;
                        self.scratch.push(adjface);
                    }
                }
            }

            let apex = self.faces[tf].iset[fvi as usize];
            let mut reversed = false;

            /* Sort horizon edges in CCW order */
            {
                let mut triangle = [Vertex::default(); 3];
                let mut vindex = 0;

                for i in 0..self.horizonedges.len() {
                    let he0 = self.horizonedges[i];
                    let he0vert = self.edges[he0 as usize].to_vertex;
                    let phe0 = self.edges[he0 as usize].previous_he;
                    let phe0vert = self.edges[phe0 as usize].to_vertex;

                    for j in i + 2..self.horizonedges.len() {
                        let he1 = self.horizonedges[j];
                        let he1vert = self.edges[he1 as usize].to_vertex;
                        let phe1 = self.edges[he1 as usize].previous_he;
                        let phe1vert = self.edges[phe1 as usize].to_vertex;

                        if phe1vert == he0vert || phe0vert == he1vert {
                            self.horizonedges.swap(j, i + 1);
                            break;
                        }
                    }

                    if vindex < 3 {
                        triangle[vindex] =
                            self.vertices[self.edges[he0 as usize].to_vertex as usize];
                        vindex += 1;
                    }
                }

                if vindex == 3 {
                    /* Detect first triangle face ordering */
                    let v0 = triangle[0].sub(triangle[1]);
                    let v1 = triangle[2].sub(triangle[1]);

                    let n = v0.cross(v1);

                    /* Get the vector to the apex */
                    let toapex = triangle[0].sub(self.vertices[apex as usize]);

                    reversed = n.dot(toapex) < 0.0;
                }
            }

            /* Create new faces */
            {
                let mut top = pop_stack(&mut self.horizonedges);
                let mut last = pop_stack(&mut self.horizonedges);
                let first = top;
                let mut looped = false;

                /* Release scratch */
                self.scratch.clear();

                while !looped {
                    if last == -1 {
                        looped = true;
                        last = first;
                    }

                    let (prevhe, nexthe) = if reversed { (top, last) } else { (last, top) };

                    let verts = [
                        self.edges[prevhe as usize].to_vertex,
                        self.edges[nexthe as usize].to_vertex,
                        apex,
                    ];

                    self.valid[self.edges[nexthe as usize].adjacent_face as usize] = false;

                    let oppedge = self.edges[nexthe as usize].opposite_he;
                    let newface = self.next_face();

                    self.face_init(newface, verts);

                    let e0 = self.faces[newface].edges[0];
                    self.edges[oppedge as usize].opposite_he = self.edges[e0 as usize].he;
                    self.edges[e0 as usize].opposite_he = self.edges[oppedge as usize].he;

                    self.scratch.push(self.faces[newface].face);
                    self.newhorizonedges.push(e0);

                    top = last;
                    last = pop_stack(&mut self.horizonedges);
                }
            }

            /* Attach point sets to newly created faces */
            for k in 0..self.faces.len() {
                if self.valid[k] || self.faces[k].iset.is_empty() {
                    continue;
                }

                if self.faces[k].visitededges == 3 {
                    self.valid[k] = false;
                }

                let iset = std::mem::take(&mut self.faces[k].iset);
                for &vertex in &iset {
                    let mut dface = None;

                    for j in 0..self.scratch.len() {
                        let newface = self.scratch[j] as usize;
                        let e = self.faces[newface].edges;
                        if self.edges[e[0] as usize].to_vertex == vertex
                            || self.edges[e[1] as usize].to_vertex == vertex
                            || self.edges[e[2] as usize].to_vertex == vertex
                        {
                            continue;
                        }

                        if self.face_can_see_vertex_epsilon(newface, vertex as usize, epsilon) {
                            dface = Some(newface);
                            break;
                        }
                    }

                    if let Some(dface) = dface {
                        self.faces[dface].iset.push(vertex);
                    }
                }
                // The set is emptied either way: upstream sets its size to
                // zero after handing on whichever of its points it could.
            }

            /* Link new faces together */
            {
                for i in 0..self.newhorizonedges.len() {
                    let ii = if reversed {
                        if i == 0 {
                            self.newhorizonedges.len() - 1
                        } else {
                            i - 1
                        }
                    } else {
                        (i + 1) % self.newhorizonedges.len()
                    };

                    let phe0 = self.edges[self.newhorizonedges[i] as usize].previous_he;
                    let nhe1 = self.edges[self.newhorizonedges[ii] as usize].next_he;

                    self.edges[phe0 as usize].opposite_he = self.edges[nhe1 as usize].he;
                    self.edges[nhe1 as usize].opposite_he = self.edges[phe0 as usize].he;
                }

                self.newhorizonedges.clear();
            }

            /* Push new face to stack */
            {
                for i in 0..self.scratch.len() {
                    let face = self.scratch[i] as usize;
                    if !self.faces[face].iset.is_empty() {
                        self.facestack.push(self.faces[face].face);
                    }
                }

                /* Release scratch */
                self.scratch.clear();
            }

            topface = pop_stack(&mut self.facestack);
        }
    }

    fn build_tetrahedron(&mut self, epsilon: f64) {
        /* Get the initial tetrahedron basis (first face) */
        let mut vertices = self.tetrahedron_basis();

        /* Find apex from the tetrahedron basis */
        let apex;
        {
            let v0 = self.vertices[vertices[1] as usize].sub(self.vertices[vertices[0] as usize]);
            let v1 = self.vertices[vertices[2] as usize].sub(self.vertices[vertices[0] as usize]);

            let normal = v0.cross(v1).normalize();

            let centroid = self.face_centroid(vertices);
            let sdist = normal.dot(centroid);

            apex = self.furthest_point_from_plane(None, self.vertices.len(), normal, sdist);
            let vapex = self.vertices[apex as usize].sub(centroid);

            /* Whether the face is looking towards the apex */
            if vapex.dot(normal) > 0.0 {
                vertices.swap(1, 2);
            }
        }

        let f0 = self.next_face();
        self.face_init(f0, vertices);

        /* Build faces from the tetrahedron basis to the apex */
        for i in 0..3 {
            let edgeindex = self.faces[f0].edges[i];
            let edge = self.edges[edgeindex as usize];
            let prevedge = self.edges[edge.previous_he as usize];

            let facevertices = [edge.to_vertex, prevedge.to_vertex, apex];

            let face = self.next_face();
            self.face_init(face, facevertices);

            let e0 = self.faces[face].edges[0];
            self.edges[edgeindex as usize].opposite_he = self.edges[e0 as usize].he;
            self.edges[e0 as usize].opposite_he = self.edges[edgeindex as usize].he;
        }

        /* Attach half edges to faces tied to the apex */
        for i in 0..3 {
            let j = (i + 2) % 3;

            let e1 = self.faces[f0 + i + 1].edges[1];
            let e2 = self.faces[f0 + j + 1].edges[2];

            self.edges[e1 as usize].opposite_he = self.edges[e2 as usize].he;
            self.edges[e2 as usize].opposite_he = self.edges[e1 as usize].he;
        }

        /* Create initial point set; every point is */
        /* attached to the first face it can see */
        for i in 0..self.vertices.len() {
            let iv = i as i64;
            if vertices[0] == iv || vertices[1] == iv || vertices[2] == iv {
                continue;
            }

            let mut dface = None;
            for j in 0..4 {
                if self.face_can_see_vertex_epsilon(j, i, epsilon) {
                    dface = Some(j);
                    break;
                }
            }

            if let Some(dface) = dface {
                let mut valid = true;
                for j in 0..3 {
                    let e = self.faces[dface].edges[j];
                    if iv == self.edges[e as usize].to_vertex {
                        valid = false;
                        break;
                    }
                }
                if !valid {
                    continue;
                }
                self.faces[dface].iset.push(iv);
            }
        }

        /* Add initial tetrahedron faces to the face stack */
        for i in 0..4 {
            self.valid[i] = true;
            self.facestack.push(i as i64);
        }
    }

    /// `qh__remove_vertex_duplicates`, by a grid of epsilon-sized cells
    /// rather than by a quadratic scan. See the module note.
    fn remove_vertex_duplicates(&mut self, epsilon: f64) {
        use std::collections::HashMap;

        for v in &mut self.vertices {
            // Upstream's `if (v->x == 0) v->x = 0;`, which is not a no-op:
            // it turns a negative zero into a positive one.
            if v.x == 0.0 {
                v.x = 0.0;
            }
            if v.y == 0.0 {
                v.y = 0.0;
            }
            if v.z == 0.0 {
                v.z = 0.0;
            }
        }

        // A cell an epsilon across, so two points within epsilon on every
        // axis are always in the same cell or one beside it. An epsilon of
        // zero, which happens when every point is on the y and z axes, makes
        // the test exact equality, and any cell size answers that.
        let size = if epsilon > 0.0 && epsilon.is_finite() {
            epsilon
        } else {
            1.0
        };
        let cell = |v: Vertex| {
            (
                (v.x / size).floor() as i64,
                (v.y / size).floor() as i64,
                (v.z / size).floor() as i64,
            )
        };

        let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        let mut kept: Vec<Vertex> = Vec::with_capacity(self.vertices.len());

        for i in 0..self.vertices.len() {
            let v = self.vertices[i];
            let (cx, cy, cz) = cell(v);
            let mut duplicate = false;
            'search: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(bucket) = grid.get(&(cx + dx, cy + dy, cz + dz)) else {
                            continue;
                        };
                        for &j in bucket {
                            if kept[j].equals_epsilon(v, epsilon) {
                                duplicate = true;
                                break 'search;
                            }
                        }
                    }
                }
            }
            if !duplicate {
                grid.entry((cx, cy, cz)).or_default().push(kept.len());
                kept.push(v);
            }
        }

        self.vertices = kept;
    }
}

fn compute_epsilon(vertices: &[Vertex]) -> f64 {
    let mut maxxi = -QH_FLT_MAX;
    let mut maxyi = -QH_FLT_MAX;

    for v in vertices {
        let fxi = v.x.abs();
        let fyi = v.y.abs();
        if fxi > maxxi {
            maxxi = fxi;
        }
        if fyi > maxyi {
            maxyi = fyi;
        }
    }

    2.0 * (maxxi + maxyi) * QH_FLT_EPS
}

/// `qh_quickhull3d`: the convex hull of a cloud of points, as triangles.
///
/// Empty if there are too few points to make one, which upstream leaves to
/// the caller's own check.
pub fn quickhull3d(vertices: &[Vertex]) -> Vec<Triangle> {
    if vertices.len() < 4 {
        return Vec::new();
    }

    let epsilon = compute_epsilon(vertices);

    let mut context = Context {
        faces: Vec::new(),
        edges: Vec::new(),
        vertices: vertices.to_vec(),
        facestack: Vec::new(),
        scratch: Vec::new(),
        horizonedges: Vec::new(),
        newhorizonedges: Vec::new(),
        valid: Vec::new(),
    };

    context.remove_vertex_duplicates(epsilon);
    if context.vertices.len() < 4 {
        return Vec::new();
    }

    /* Build the initial tetrahedron */
    context.build_tetrahedron(epsilon);

    /* Build the convex hull */
    context.build_hull(epsilon);

    let mut out = Vec::new();
    for i in 0..context.faces.len() {
        if !context.valid[i] {
            continue;
        }
        let e = context.faces[i].edges;
        out.push(Triangle {
            normal: context.faces[i].normal,
            vertices: [
                context.vertices[context.edges[e[0] as usize].to_vertex as usize],
                context.vertices[context.edges[e[1] as usize].to_vertex as usize],
                context.vertices[context.edges[e[2] as usize].to_vertex as usize],
            ],
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{frand, ya_rand_init};

    /// Is every point on or inside every face's plane? That is what being the
    /// convex hull means, and it is the property upstream's own debug build
    /// checks with `qh__test_hull`.
    ///
    /// The slack is not slop. A point within epsilon of a face counts as
    /// outside it and is nudged out along the normal so that it stays so, and
    /// the hull is built from the nudged copy, so a corner of the answer can
    /// sit an epsilon beyond the point it came from. Epsilon here is
    /// `2 * (max|x| + max|y|) * 1e-5`, which is 4e-5 for a unit cube.
    fn everything_is_inside(hull: &[Triangle], points: &[Vertex], slack: f64) {
        assert!(!hull.is_empty());
        for t in hull {
            let sdist = t.normal.dot(t.vertices[0]);
            for p in points {
                let d = t.normal.dot(*p) - sdist;
                assert!(d <= slack, "point {p:?} is {d} outside a face");
            }
        }
    }

    #[test]
    fn the_hull_of_a_cube_is_the_cube() {
        let mut points = Vec::new();
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0f64, 1.0] {
                    points.push(Vertex::new(x, y, z));
                }
            }
        }
        let hull = quickhull3d(&points);
        // Six square sides, two triangles each.
        assert_eq!(hull.len(), 12, "{}", hull.len());
        everything_is_inside(&hull, &points, 1e-4);

        // Every triangle has area, and its stored normal is the one its own
        // winding gives, pointing outwards.
        for t in &hull {
            let n = t.vertices[1]
                .sub(t.vertices[0])
                .cross(t.vertices[2].sub(t.vertices[0]));
            assert!(n.length2() > 1e-9);
            assert!(n.normalize().dot(t.normal) > 0.99, "{:?}", t.normal);
            let mid = t.vertices[0].add(t.vertices[1]).add(t.vertices[2]);
            assert!(mid.dot(t.normal) > 0.0, "normal points inwards");
        }
    }

    #[test]
    fn interior_points_do_not_change_the_hull() {
        ya_rand_init(20260812);
        let mut points = Vec::new();
        for x in [-1.0, 1.0] {
            for y in [-1.0, 1.0] {
                for z in [-1.0f64, 1.0] {
                    points.push(Vertex::new(x, y, z));
                }
            }
        }
        let corners = points.clone();
        for _ in 0..400 {
            points.push(Vertex::new(
                (frand(1.6) - 0.8) as f64,
                (frand(1.6) - 0.8) as f64,
                (frand(1.6) - 0.8) as f64,
            ));
        }
        let hull = quickhull3d(&points);
        everything_is_inside(&hull, &points, 1e-4);
        // The corners are still corners: each is on at least three faces,
        // give or take the epsilon nudge.
        for c in &corners {
            let on = hull
                .iter()
                .filter(|t| t.vertices.iter().any(|v| v.sub(*c).length2() < 1e-8))
                .count();
            assert!(on >= 3, "{c:?} is on {on} faces");
        }
    }

    #[test]
    fn a_ball_of_points_becomes_a_closed_shell() {
        ya_rand_init(20260812);
        let mut points = Vec::new();
        while points.len() < 2000 {
            let v = Vertex::new(
                (0.5 - frand(1.0)) as f64,
                (0.5 - frand(1.0)) as f64,
                (0.5 - frand(1.0)) as f64,
            );
            if v.length2() < 0.25 {
                points.push(v);
            }
        }
        let hull = quickhull3d(&points);
        assert!(hull.len() > 50, "{}", hull.len());
        everything_is_inside(&hull, &points, 1e-4);

        // A closed surface: every edge is walked once in each direction.
        let key = |a: Vertex, b: Vertex| {
            let f = |v: Vertex| (v.x.to_bits(), v.y.to_bits(), v.z.to_bits());
            (f(a), f(b))
        };
        let mut edges = std::collections::HashMap::new();
        for t in &hull {
            for i in 0..3 {
                let (a, b) = (t.vertices[i], t.vertices[(i + 1) % 3]);
                *edges.entry(key(a, b)).or_insert(0) += 1;
            }
        }
        let unpaired = edges
            .iter()
            .filter(|&(&(a, b), _)| edges.get(&(b, a)) != Some(&1))
            .count();
        // Upstream's own comment says this hull is sometimes not closed, so
        // this is a bound on how ragged rather than a demand that it be
        // perfect.
        assert!(
            unpaired * 20 < edges.len(),
            "{unpaired} unpaired of {}",
            edges.len()
        );
    }
}
