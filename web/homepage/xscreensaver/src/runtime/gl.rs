//! Immediate-mode OpenGL, in the shape upstream's savers are written against.
//!
//! ```text
//! jwzgles, Copyright © 2012-2018 Jamie Zawinski <jwz@jwz.org>
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
//! The 136 OpenGL savers are written against OpenGL 1.3: a matrix stack, a
//! fixed-function pipeline, and `glBegin`/`glVertex`/`glEnd`. None of that
//! exists in OpenGL ES 2 and later, and so none of it exists in WebGL. Upstream
//! hit this first, on iOS and Android, and answered it with `jwzgles.c`, which
//! implements the old calls on top of the new ones. This is the same answer.
//!
//! It is not the same implementation. `jwzgles.c` *is* a GL binding: it makes
//! GL calls as it goes. This records instead. A saver's frame comes out as a
//! [`Frame`] of vertex data and the state each batch of it needs, and the host
//! hands that to WebGL2. Two things fall out of the indirection. The savers stay
//! testable with no browser and no GPU, which is what the whole 2D tier is built
//! on. And the batching is free: `glBegin`/`glEnd` around three vertices is one
//! entry in a vertex buffer here, rather than the driver round trip it was.
//!
//! ## The matrix at `glBegin`
//!
//! A vertex is transformed by the modelview matrix as it is given, so in
//! principle the matrix could change between two `glVertex` calls. In practice
//! it cannot: changing it inside a `glBegin` block is an error in OpenGL, and
//! nothing does. So a batch captures the matrices once, when the block opens,
//! and the host multiplies out on the GPU.
//!
//! ## Display lists
//!
//! A list records *commands*, not results, because `glCallList` runs it under
//! whatever matrix is current at the time. So the list is a little command
//! stream, replayed on call, which is what `glNewList` means and what
//! `jwzgles.c` does too.

use std::f64::consts::PI;

/// A 4x4 matrix in OpenGL's column-major order: `m[col * 4 + row]`.
///
/// The layout is not an implementation detail. It is the order `glMultMatrixf`
/// takes, the order `quat_to_rotmatrix` writes, and the order a GLSL `mat4`
/// uniform wants, so keeping it means never having to think about which way
/// round a matrix from upstream is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f32; 16]);

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    /// `self * other`, in the order `glMultMatrix` composes: the new matrix is
    /// applied first, closest to the vertex.
    #[must_use]
    pub fn mul(&self, other: &Mat4) -> Mat4 {
        let (a, b) = (&self.0, &other.0);
        let mut m = [0.0; 16];
        for col in 0..4 {
            for row in 0..4 {
                m[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
            }
        }
        Mat4(m)
    }

    /// The point transformed by this matrix, with w divided out.
    #[must_use]
    pub fn transform(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        let mut o = [0.0f32; 4];
        for (row, out) in o.iter_mut().enumerate() {
            *out = m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        }
        let w = o[3];
        if w != 0.0 && w != 1.0 {
            for v in o.iter_mut().take(3) {
                *v /= w;
            }
        }
        [o[0], o[1], o[2]]
    }

    fn translate(x: f32, y: f32, z: f32) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.0[12] = x;
        m.0[13] = y;
        m.0[14] = z;
        m
    }

    fn scale(x: f32, y: f32, z: f32) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.0[0] = x;
        m.0[5] = y;
        m.0[10] = z;
        m
    }

    /// `glRotatef`: `angle` in degrees about the given axis.
    fn rotate(angle: f32, x: f32, y: f32, z: f32) -> Mat4 {
        let len = (x * x + y * y + z * z).sqrt();
        if len == 0.0 {
            return Mat4::IDENTITY;
        }
        let (x, y, z) = (x / len, y / len, z / len);
        let r = angle * (PI as f32) / 180.0;
        let (s, c) = (r.sin(), r.cos());
        let t = 1.0 - c;
        Mat4([
            t * x * x + c,
            t * x * y + s * z,
            t * x * z - s * y,
            0.0,
            t * x * y - s * z,
            t * y * y + c,
            t * y * z + s * x,
            0.0,
            t * x * z + s * y,
            t * y * z - s * x,
            t * z * z + c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    /// `glFrustum`.
    fn frustum(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Mat4 {
        let mut m = Mat4([0.0; 16]);
        m.0[0] = 2.0 * n / (r - l);
        m.0[5] = 2.0 * n / (t - b);
        m.0[8] = (r + l) / (r - l);
        m.0[9] = (t + b) / (t - b);
        m.0[10] = -(f + n) / (f - n);
        m.0[11] = -1.0;
        m.0[14] = -2.0 * f * n / (f - n);
        m
    }

    /// `glOrtho`.
    fn ortho(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.0[0] = 2.0 / (r - l);
        m.0[5] = 2.0 / (t - b);
        m.0[10] = -2.0 / (f - n);
        m.0[12] = -(r + l) / (r - l);
        m.0[13] = -(t + b) / (t - b);
        m.0[14] = -(f + n) / (f - n);
        m
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 {
        return v;
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

/// What a batch of vertices makes. `GL_QUADS`, `GL_QUAD_STRIP` and
/// `GL_POLYGON` are absent on purpose: OpenGL ES has no such primitives, so
/// they are cut into triangles as the block closes, exactly as `jwzgles.c`
/// does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Points,
    Lines,
    LineStrip,
    LineLoop,
    Triangles,
    TriangleStrip,
    TriangleFan,
}

/// What a saver asks `glBegin` for, before the quads are cut up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Points,
    Lines,
    LineStrip,
    LineLoop,
    Triangles,
    TriangleStrip,
    TriangleFan,
    Quads,
    QuadStrip,
    Polygon,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub color: [f32; 4],
    pub normal: [f32; 3],
}

/// `glBlendFunc`, as the two pairs the savers actually pass it. More become
/// variants when one of them needs a third.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// `glDisable (GL_BLEND)`.
    Off,
    /// `GL_ONE, GL_ONE`: everything adds up, so where things overlap they get
    /// brighter and eventually white. What makes overlapping translucent
    /// shapes glow.
    Add,
    /// `GL_SRC_ALPHA, GL_ONE`: adds, but scaled by the alpha, so a
    /// translucent thing glows without a solid one blowing out.
    AlphaAdd,
    /// `GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA`: ordinary transparency.
    Alpha,
}

/// How many lights a saver can turn on. OpenGL guarantees eight; the savers
/// here use one or two, and this grows when one wants more.
pub const MAX_LIGHTS: usize = 2;

/// One of `GL_LIGHT0` and friends.
///
/// The position is in *eye* space: `glLightfv(GL_LIGHT0, GL_POSITION, ..)`
/// transforms what it is given by the modelview matrix current at the time of
/// the call, which is how a saver pins a light to the scene rather than to the
/// object it is about to rotate. A `w` of 0 means the light is infinitely far
/// away and only its direction matters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: [f32; 4],
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
}

impl Default for Light {
    /// OpenGL's own defaults for `GL_LIGHT0`.
    fn default() -> Self {
        Light {
            position: [0.0, 0.0, 1.0, 0.0],
            ambient: [0.0, 0.0, 0.0, 1.0],
            diffuse: [1.0, 1.0, 1.0, 1.0],
            specular: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// `glMaterialfv (GL_FRONT, ..)`. Ambient and diffuse are one field because
/// `GL_AMBIENT_AND_DIFFUSE` is what the savers set, nearly without exception.
///
/// The back colour is separate because a handful of savers turn culling off and
/// paint the inside of a surface a different colour from the outside, which is
/// the whole of what they look like. Everything else follows the front, since
/// setting one sets both unless a saver asks otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub ambient_diffuse: [f32; 4],
    pub back_ambient_diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub shininess: f32,
}

impl Default for Material {
    /// OpenGL's own defaults.
    fn default() -> Self {
        Material {
            ambient_diffuse: [0.8, 0.8, 0.8, 1.0],
            back_ambient_diffuse: [0.8, 0.8, 0.8, 1.0],
            specular: [0.0, 0.0, 0.0, 1.0],
            shininess: 0.0,
        }
    }
}

/// One `glBegin`/`glEnd` block: what to draw, where its vertices are, and the
/// state that was current when the block opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub primitive: Primitive,
    /// Index of the first vertex in [`Frame::vertices`].
    pub first: usize,
    pub count: usize,
    /// Projection times modelview, ready for a `mat4` uniform.
    pub mvp: Mat4,
    /// The modelview on its own, which lighting needs: the shading is worked
    /// out in eye space, where the light is.
    pub modelview: Mat4,
    pub lighting: bool,
    /// Which of the lights are on.
    pub light_enabled: [bool; MAX_LIGHTS],
    pub lights: [Light; MAX_LIGHTS],
    pub material: Material,
    /// `glEnable(GL_CULL_FACE)`: throw away the back of every face.
    pub cull_face: bool,
    /// Which winding is the front of a face, `glFrontFace`.
    pub front_face_cw: bool,
    /// Clear the depth buffer before drawing this batch, which is what a
    /// mid-frame `glClear(GL_DEPTH_BUFFER_BIT)` means: everything after it
    /// draws over everything before it, whatever the distances say.
    pub clear_depth_first: bool,
    pub blend: Blend,
    pub point_size: f32,
    pub line_width: f32,
    /// `glEnable(GL_DEPTH_TEST)`. Off for the savers that draw a flat scene
    /// and want it in the order they drew it.
    pub depth_test: bool,
}

impl Batch {
    /// Would these two draw identically but for their vertices? Everything on
    /// a batch except where its vertices are.
    fn same_state(&self, other: &Batch) -> bool {
        self.primitive == other.primitive
            && self.mvp == other.mvp
            && self.modelview == other.modelview
            && self.point_size == other.point_size
            && self.line_width == other.line_width
            && self.depth_test == other.depth_test
            && self.lighting == other.lighting
            && self.lights == other.lights
            && self.light_enabled == other.light_enabled
            && self.material == other.material
            && self.cull_face == other.cull_face
            && self.front_face_cw == other.front_face_cw
            && !other.clear_depth_first
            && self.blend == other.blend
    }
}

/// Everything a frame draws.
#[derive(Clone, Debug)]
pub struct Frame {
    pub vertices: Vec<Vertex>,
    pub batches: Vec<Batch>,
    /// What to clear to before any of it, if the saver asked for a clear.
    pub clear: Option<[f32; 4]>,
    pub viewport: [i32; 4],
}

/// The calls a display list can hold.
///
/// Only the ones that are legal inside `glNewList` and that a saver actually
/// records. A call that is not here executes immediately even while a list is
/// being compiled, which is what `jwzgles.c` does for the calls it does not
/// know how to record.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Begin(Shape),
    End,
    Vertex([f32; 3]),
    Color([f32; 4]),
    Normal([f32; 3]),
    PointSize(f32),
    LineWidth(f32),
    PushMatrix,
    PopMatrix,
    Translate([f32; 3]),
    Rotate([f32; 4]),
    Scale([f32; 3]),
    LoadIdentity,
    CallList(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MatrixMode {
    Modelview,
    Projection,
}

/// The GL context a saver draws into.
pub struct Glx {
    modelview: Vec<Mat4>,
    projection: Vec<Mat4>,
    mode: MatrixMode,

    color: [f32; 4],
    normal: [f32; 3],
    point_size: f32,
    depth_test: bool,
    lighting: bool,
    lights: [Light; MAX_LIGHTS],
    light_enabled: [bool; MAX_LIGHTS],
    material: Material,
    cull_face: bool,
    front_face_cw: bool,
    clear_color: [f32; 4],
    clear_depth_pending: bool,
    blend: Blend,
    line_width: f32,

    /// The block in progress, and the vertices it has so far.
    shape: Option<Shape>,
    pending: Vec<Vertex>,

    frame: Frame,

    /// `glGenLists` hands out indices into this; `None` is a list that has been
    /// reserved but not yet compiled.
    lists: Vec<Option<Vec<Cmd>>>,
    compiling: Option<usize>,
    /// `glNewList(.., GL_COMPILE_AND_EXECUTE)` records and draws at once.
    compile_and_execute: bool,
}

impl Glx {
    /// A context in OpenGL's documented initial state: identity matrices, white
    /// current colour, lighting off.
    ///
    /// No `Default`, deliberately. Clippy asks for one and the house lint
    /// refuses it, and the house lint is closer to right: a `Glx` is a device,
    /// not a value, and nothing should be conjuring one implicitly.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Glx {
            modelview: vec![Mat4::IDENTITY],
            projection: vec![Mat4::IDENTITY],
            mode: MatrixMode::Modelview,
            color: [1.0, 1.0, 1.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            point_size: 1.0,
            depth_test: true,
            lighting: false,
            lights: [Light::default(); MAX_LIGHTS],
            light_enabled: [false; MAX_LIGHTS],
            material: Material::default(),
            cull_face: false,
            front_face_cw: false,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            clear_depth_pending: false,
            blend: Blend::Off,
            line_width: 1.0,
            shape: None,
            pending: Vec::new(),
            frame: Frame {
                vertices: Vec::new(),
                batches: Vec::new(),
                clear: None,
                // Replaced by the first `start_frame`, and by any `glViewport`
                // the saver makes after that.
                viewport: [0, 0, 0, 0],
            },
            lists: Vec::new(),
            compiling: None,
            compile_and_execute: false,
        }
    }

    /// Throw away the last frame's geometry and start collecting the next.
    pub fn start_frame(&mut self, width: i32, height: i32) {
        self.frame.vertices.clear();
        self.frame.batches.clear();
        self.frame.clear = None;
        if self.frame.viewport == [0, 0, 0, 0] {
            self.frame.viewport = [0, 0, width, height];
        }
    }

    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /* Matrices */

    pub fn matrix_mode_projection(&mut self) {
        self.mode = MatrixMode::Projection;
    }

    pub fn matrix_mode_modelview(&mut self) {
        self.mode = MatrixMode::Modelview;
    }

    fn stack(&mut self) -> &mut Vec<Mat4> {
        match self.mode {
            MatrixMode::Modelview => &mut self.modelview,
            MatrixMode::Projection => &mut self.projection,
        }
    }

    fn top(&mut self) -> &mut Mat4 {
        let stack = self.stack();
        if stack.is_empty() {
            stack.push(Mat4::IDENTITY);
        }
        let last = stack.len() - 1;
        &mut stack[last]
    }

    pub fn load_identity(&mut self) {
        self.record_or(Cmd::LoadIdentity, |g| *g.top() = Mat4::IDENTITY);
    }

    pub fn push_matrix(&mut self) {
        self.record_or(Cmd::PushMatrix, |g| {
            let top = *g.top();
            g.stack().push(top);
        });
    }

    pub fn pop_matrix(&mut self) {
        self.record_or(Cmd::PopMatrix, |g| {
            let stack = g.stack();
            if stack.len() > 1 {
                stack.pop();
            }
        });
    }

    pub fn translate(&mut self, x: f32, y: f32, z: f32) {
        self.record_or(Cmd::Translate([x, y, z]), |g| {
            g.mult(Mat4::translate(x, y, z));
        });
    }

    pub fn rotate(&mut self, angle: f32, x: f32, y: f32, z: f32) {
        self.record_or(Cmd::Rotate([angle, x, y, z]), |g| {
            g.mult(Mat4::rotate(angle, x, y, z));
        });
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) {
        self.record_or(Cmd::Scale([x, y, z]), |g| {
            g.mult(Mat4::scale(x, y, z));
        });
    }

    /// `glMultMatrixf`.
    pub fn mult_matrix(&mut self, m: Mat4) {
        self.mult(m);
    }

    fn mult(&mut self, m: Mat4) {
        let top = *self.top();
        *self.top() = top.mul(&m);
    }

    pub fn frustum(&mut self, l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) {
        self.mult(Mat4::frustum(l, r, b, t, n, f));
    }

    pub fn ortho(&mut self, l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) {
        self.mult(Mat4::ortho(l, r, b, t, n, f));
    }

    /// `gluPerspective`.
    pub fn perspective(&mut self, fovy: f32, aspect: f32, near: f32, far: f32) {
        let t = near * (fovy * (PI as f32) / 360.0).tan();
        let r = t * aspect;
        self.frustum(-r, r, -t, t, near, far);
    }

    /// `gluLookAt`: put the eye somewhere and point it at something.
    pub fn look_at(&mut self, eye: [f32; 3], centre: [f32; 3], up: [f32; 3]) {
        let f = normalize([centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]]);
        let s = normalize(cross(f, normalize(up)));
        let u = cross(s, f);
        let m = Mat4([
            s[0], u[0], -f[0], 0.0, //
            s[1], u[1], -f[1], 0.0, //
            s[2], u[2], -f[2], 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ]);
        self.mult(m);
        self.translate(-eye[0], -eye[1], -eye[2]);
    }

    /// The modelview matrix, for the savers that read it back.
    pub fn modelview(&self) -> Mat4 {
        self.modelview.last().copied().unwrap_or(Mat4::IDENTITY)
    }

    /* State */

    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.frame.viewport = [x, y, w, h];
    }

    /// `glClearColor`. Context state, not something that happens: a saver sets
    /// it once when it starts and clears with it every frame after that.
    pub fn clear_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.clear_color = [r, g, b, a];
    }

    /// `glClear`. The colour is whatever `glClearColor` last set, defaulting to
    /// black, which is what a screen saver wants anyway.
    pub fn clear(&mut self) {
        if self.frame.clear.is_none() {
            self.frame.clear = Some(self.clear_color);
        }
    }

    /// `glClear (GL_DEPTH_BUFFER_BIT)` partway through a frame, which is how a
    /// saver says "whatever I draw next goes on top of all of that". It
    /// attaches to the next block rather than happening now, because the
    /// ordering is the whole point of it.
    pub fn clear_depth(&mut self) {
        self.clear_depth_pending = true;
    }

    pub fn point_size(&mut self, size: f32) {
        self.record_or(Cmd::PointSize(size), |g| g.point_size = size);
    }

    pub fn line_width(&mut self, width: f32) {
        self.record_or(Cmd::LineWidth(width), |g| g.line_width = width);
    }

    /// `glEnable(GL_DEPTH_TEST)` / `glDisable(GL_DEPTH_TEST)`. A saver drawing
    /// a flat scene turns it off so its polygons stack in the order it drew
    /// them rather than by how far away they are.
    pub fn depth_test(&mut self, on: bool) {
        self.depth_test = on;
    }

    /// `glBlendFunc`, and the enable that goes with it.
    pub fn blend(&mut self, blend: Blend) {
        self.blend = blend;
    }

    /// `glFrontFace`: true for `GL_CW`, false for `GL_CCW`, which is the
    /// default. A saver whose faces are wound clockwise says so rather than
    /// having its outsides culled.
    pub fn front_face_cw(&mut self, cw: bool) {
        self.front_face_cw = cw;
    }

    /// `glEnable(GL_CULL_FACE)`.
    pub fn cull_face(&mut self, on: bool) {
        self.cull_face = on;
    }

    /// `glEnable(GL_LIGHTING)`. With it on the vertex colours are ignored and
    /// the material is what is shaded, which is OpenGL's rule and not a
    /// simplification: a saver that wants a lit object sets a material for it.
    pub fn lighting(&mut self, on: bool) {
        self.lighting = on;
    }

    /// `glEnable` of one of the lights. Turning lighting on is separate: a saver does
    /// both, and so must a port.
    pub fn light_enable(&mut self, n: usize, on: bool) {
        if n < MAX_LIGHTS {
            self.light_enabled[n] = on;
        }
    }

    /// `glLightfv` of a light's `GL_POSITION`.
    ///
    /// The position is taken through the modelview matrix as it stands now,
    /// which is what fixes the light to the scene rather than to whatever the
    /// saver is about to rotate. `w` of 0 makes it directional.
    pub fn light_position(&mut self, n: usize, x: f32, y: f32, z: f32, w: f32) {
        let m = self.modelview().0;
        let mut o = [0.0f32; 4];
        for (row, out) in o.iter_mut().enumerate() {
            *out = m[row] * x + m[4 + row] * y + m[8 + row] * z + m[12 + row] * w;
        }
        if let Some(light) = self.lights.get_mut(n) {
            light.position = o;
        }
    }

    pub fn light_ambient(&mut self, n: usize, rgba: [f32; 4]) {
        if let Some(light) = self.lights.get_mut(n) {
            light.ambient = rgba;
        }
    }

    pub fn light_diffuse(&mut self, n: usize, rgba: [f32; 4]) {
        if let Some(light) = self.lights.get_mut(n) {
            light.diffuse = rgba;
        }
    }

    pub fn light_specular(&mut self, n: usize, rgba: [f32; 4]) {
        if let Some(light) = self.lights.get_mut(n) {
            light.specular = rgba;
        }
    }

    /// `glMaterialfv (GL_FRONT_AND_BACK, GL_AMBIENT_AND_DIFFUSE, ..)`.
    ///
    /// Upstream usually writes `GL_FRONT` here, which is the same thing in
    /// practice: those savers are drawing with culling on, so there is no back
    /// face to have a colour. Setting both is what makes the ones that leave
    /// culling off look right without each of them saying so.
    pub fn material_ambient_diffuse(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.ambient_diffuse = rgba;
        self.material.back_ambient_diffuse = rgba;
    }

    /// `glMaterialfv (GL_BACK, GL_AMBIENT_AND_DIFFUSE, ..)`: the inside of a
    /// surface, for a saver that wants it a different colour from the outside.
    /// Set it after the front, which sets both.
    pub fn material_back_ambient_diffuse(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.back_ambient_diffuse = rgba;
    }

    pub fn material_specular(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.specular = rgba;
    }

    pub fn material_shininess(&mut self, shininess: f32) {
        self.split_block();
        self.material.shininess = shininess;
    }

    /// Close the run of vertices so far into a batch and carry on with the
    /// same block.
    ///
    /// `glMaterial` is one of the few calls OpenGL allows between `glBegin`
    /// and `glEnd`, and savers use it: `cityflow` draws eight hundred boxes as
    /// one long run of quads, changing the material between each. A batch
    /// carries one material, so the run has to be cut where the material
    /// changes.
    ///
    /// Only where the vertices are independent primitives, and only on a
    /// primitive boundary: cutting a strip or a fan in half would lose the
    /// triangles that straddle the cut. Everywhere else the state is simply
    /// updated, which is what it did before, and is right for the savers that
    /// set a material before opening a block rather than inside one.
    fn split_block(&mut self) {
        let Some(shape) = self.shape else { return };
        if self.pending.is_empty() {
            return;
        }
        let n = match shape {
            Shape::Points => 1,
            Shape::Lines => 2,
            Shape::Triangles => 3,
            Shape::Quads => 4,
            _ => return,
        };
        if !self.pending.len().is_multiple_of(n) {
            return;
        }
        self.flush();
        self.shape = Some(shape);
    }

    /* Vertices */

    pub fn color3f(&mut self, r: f32, g: f32, b: f32) {
        self.color4f(r, g, b, 1.0);
    }

    pub fn color4f(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.record_or(Cmd::Color([r, g, b, a]), |gl| gl.color = [r, g, b, a]);
    }

    pub fn normal3f(&mut self, x: f32, y: f32, z: f32) {
        self.record_or(Cmd::Normal([x, y, z]), |g| g.normal = [x, y, z]);
    }

    pub fn begin(&mut self, shape: Shape) {
        self.record_or(Cmd::Begin(shape), |g| {
            g.shape = Some(shape);
            g.pending.clear();
        });
    }

    pub fn vertex3f(&mut self, x: f32, y: f32, z: f32) {
        self.record_or(Cmd::Vertex([x, y, z]), |g| {
            let v = Vertex {
                pos: [x, y, z],
                color: g.color,
                normal: g.normal,
            };
            g.pending.push(v);
        });
    }

    pub fn end(&mut self) {
        self.record_or(Cmd::End, Glx::flush);
    }

    /// Close the block in progress and turn it into a batch, cutting quads into
    /// triangles on the way.
    fn flush(&mut self) {
        let Some(shape) = self.shape.take() else {
            return;
        };
        let verts = std::mem::take(&mut self.pending);
        if verts.is_empty() {
            return;
        }
        let first = self.frame.vertices.len();
        let primitive = match shape {
            Shape::Points => Primitive::Points,
            Shape::Lines => Primitive::Lines,
            Shape::LineStrip => Primitive::LineStrip,
            Shape::LineLoop => Primitive::LineLoop,
            Shape::Triangles => Primitive::Triangles,
            Shape::TriangleStrip => Primitive::TriangleStrip,
            Shape::TriangleFan | Shape::Polygon => Primitive::TriangleFan,
            Shape::Quads | Shape::QuadStrip => Primitive::Triangles,
        };
        match shape {
            // Two triangles a quad. The winding of the second matches the
            // first, so a face stays front-facing.
            Shape::Quads => {
                for q in verts.chunks_exact(4) {
                    for i in [0, 1, 2, 0, 2, 3] {
                        self.frame.vertices.push(q[i]);
                    }
                }
            }
            // A quad strip's vertices come in pairs down the two edges, so a
            // quad is (n, n+1, n+3, n+2) rather than four in a row.
            Shape::QuadStrip => {
                let mut i = 0;
                while i + 3 < verts.len() {
                    for j in [i, i + 1, i + 3, i, i + 3, i + 2] {
                        self.frame.vertices.push(verts[j]);
                    }
                    i += 2;
                }
            }
            _ => self.frame.vertices.extend_from_slice(&verts),
        }
        let count = self.frame.vertices.len() - first;
        if count == 0 {
            return;
        }
        let mvp = self.projection().mul(&self.modelview());
        let batch = Batch {
            primitive,
            first,
            count,
            mvp,
            point_size: self.point_size,
            depth_test: self.depth_test,
            modelview: self.modelview(),
            lighting: self.lighting,
            lights: self.lights,
            light_enabled: self.light_enabled,
            material: self.material,
            cull_face: self.cull_face,
            front_face_cw: self.front_face_cw,
            clear_depth_first: std::mem::take(&mut self.clear_depth_pending),
            blend: self.blend,
            line_width: self.line_width,
        };

        // Run of blocks with nothing between them but more vertices: fold them
        // into one. A saver drawing a cube as forty-eight separate quads is
        // forty-eight `glBegin` blocks and, without this, forty-eight draw
        // calls; `cubestorm` draws eight hundred such cubes a frame.
        //
        // Only for the primitives where concatenation means what it looks
        // like. Two triangle strips joined end to end are not one longer
        // strip: the join would grow a pair of triangles that nobody asked
        // for. Points, lines and triangles have no such seam.
        let mergeable = matches!(
            primitive,
            Primitive::Points | Primitive::Lines | Primitive::Triangles
        );
        if mergeable
            && let Some(last) = self.frame.batches.last_mut()
            && last.first + last.count == batch.first
            && last.same_state(&batch)
        {
            last.count += batch.count;
            return;
        }
        self.frame.batches.push(batch);
    }

    fn projection(&self) -> Mat4 {
        self.projection.last().copied().unwrap_or(Mat4::IDENTITY)
    }

    /* Display lists */

    /// `glGenLists`. Returns the first of `n` consecutive names, or 0 if none
    /// were asked for, which is the value OpenGL reserves for "no list".
    pub fn gen_lists(&mut self, n: usize) -> u32 {
        if n == 0 {
            return 0;
        }
        let first = self.lists.len() + 1;
        for _ in 0..n {
            self.lists.push(None);
        }
        first as u32
    }

    /// `glNewList(list, GL_COMPILE)`.
    pub fn new_list(&mut self, list: u32) {
        self.new_list_mode(list, false);
    }

    /// `glNewList(list, GL_COMPILE_AND_EXECUTE)`.
    pub fn new_list_and_execute(&mut self, list: u32) {
        self.new_list_mode(list, true);
    }

    fn new_list_mode(&mut self, list: u32, execute: bool) {
        let Some(i) = self.list_index(list) else {
            return;
        };
        self.lists[i] = Some(Vec::new());
        self.compiling = Some(i);
        self.compile_and_execute = execute;
    }

    pub fn end_list(&mut self) {
        self.compiling = None;
        self.compile_and_execute = false;
    }

    pub fn is_list(&self, list: u32) -> bool {
        self.list_index(list)
            .is_some_and(|i| self.lists[i].is_some())
    }

    pub fn delete_lists(&mut self, list: u32, n: usize) {
        for k in 0..n {
            if let Some(i) = self.list_index(list + k as u32) {
                self.lists[i] = None;
            }
        }
    }

    fn list_index(&self, list: u32) -> Option<usize> {
        if list == 0 {
            return None;
        }
        let i = list as usize - 1;
        (i < self.lists.len()).then_some(i)
    }

    /// `glCallList`. Replays the list under whatever matrix is current now,
    /// which is the whole point of a list holding commands rather than results.
    pub fn call_list(&mut self, list: u32) {
        if self.compiling.is_some() {
            self.record(Cmd::CallList(list));
            if !self.compile_and_execute {
                return;
            }
        }
        let Some(i) = self.list_index(list) else {
            return;
        };
        let Some(cmds) = self.lists[i].clone() else {
            return;
        };
        // Suspended while the list runs, or a list that calls another would
        // record its callee's commands into itself.
        let compiling = self.compiling.take();
        for cmd in cmds {
            self.run(cmd);
        }
        self.compiling = compiling;
    }

    /// Either append to the list being compiled or do the thing.
    fn record_or(&mut self, cmd: Cmd, run: impl FnOnce(&mut Glx)) {
        if self.compiling.is_some() {
            self.record(cmd);
            if !self.compile_and_execute {
                return;
            }
        }
        run(self);
    }

    fn record(&mut self, cmd: Cmd) {
        if let Some(i) = self.compiling
            && let Some(cmds) = &mut self.lists[i]
        {
            cmds.push(cmd);
        }
    }

    /// Replay one recorded command.
    fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Begin(s) => {
                self.shape = Some(s);
                self.pending.clear();
            }
            Cmd::End => self.flush(),
            Cmd::Vertex([x, y, z]) => self.vertex3f(x, y, z),
            Cmd::Color(c) => self.color = c,
            Cmd::Normal(n) => self.normal = n,
            Cmd::PointSize(s) => self.point_size = s,
            Cmd::LineWidth(w) => self.line_width = w,
            Cmd::PushMatrix => self.push_matrix(),
            Cmd::PopMatrix => self.pop_matrix(),
            Cmd::Translate([x, y, z]) => self.mult(Mat4::translate(x, y, z)),
            Cmd::Rotate([a, x, y, z]) => self.mult(Mat4::rotate(a, x, y, z)),
            Cmd::Scale([x, y, z]) => self.mult(Mat4::scale(x, y, z)),
            Cmd::LoadIdentity => *self.top() = Mat4::IDENTITY,
            Cmd::CallList(l) => self.call_list(l),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clear colour is state, and state outlives the frame it was set in.
    /// A saver sets it once when it starts and clears with it for ever after;
    /// forgetting that is a black background on the one saver that has one.
    #[test]
    fn the_clear_colour_survives_the_frame_it_was_set_in() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        g.clear_color(0.5, 0.6, 0.7, 1.0);
        // Setting it is not clearing with it.
        assert_eq!(g.frame().clear, None);
        g.clear();
        assert_eq!(g.frame().clear, Some([0.5, 0.6, 0.7, 1.0]));

        for _ in 0..3 {
            g.start_frame(100, 100);
            assert_eq!(g.frame().clear, None, "a new frame starts uncleared");
            g.clear();
            assert_eq!(g.frame().clear, Some([0.5, 0.6, 0.7, 1.0]));
        }
    }

    #[test]
    fn a_matrix_times_the_identity_is_itself() {
        let m = Mat4::rotate(37.0, 1.0, 2.0, 3.0);
        assert_eq!(m.mul(&Mat4::IDENTITY), m);
        assert_eq!(Mat4::IDENTITY.mul(&m), m);
    }

    /// The order matters: `glTranslate` then `glScale` scales in the translated
    /// frame, not the other way round.
    #[test]
    fn transforms_compose_in_gl_order() {
        let mut g = Glx::new();
        g.translate(10.0, 0.0, 0.0);
        g.scale(2.0, 2.0, 2.0);
        let p = g.modelview().transform([1.0, 0.0, 0.0]);
        assert_eq!(p, [12.0, 0.0, 0.0]);
    }

    #[test]
    fn a_rotation_turns_the_axes_into_each_other() {
        let m = Mat4::rotate(90.0, 0.0, 0.0, 1.0);
        let p = m.transform([1.0, 0.0, 0.0]);
        assert!((p[0]).abs() < 1e-6 && (p[1] - 1.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn the_matrix_stack_restores_what_it_saved() {
        let mut g = Glx::new();
        g.translate(1.0, 2.0, 3.0);
        let saved = g.modelview();
        g.push_matrix();
        g.rotate(90.0, 0.0, 1.0, 0.0);
        g.scale(3.0, 3.0, 3.0);
        g.pop_matrix();
        assert_eq!(g.modelview(), saved);
    }

    #[test]
    fn a_block_of_points_becomes_one_batch() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        g.begin(Shape::Points);
        g.color3f(1.0, 0.0, 0.0);
        g.vertex3f(0.0, 0.0, 0.0);
        g.vertex3f(1.0, 1.0, 1.0);
        g.end();
        let f = g.frame();
        assert_eq!(f.batches.len(), 1);
        assert_eq!(f.batches[0].primitive, Primitive::Points);
        assert_eq!(f.batches[0].count, 2);
        assert_eq!(f.vertices[0].color, [1.0, 0.0, 0.0, 1.0]);
    }

    /// A colour set between vertices applies from there on, and not backwards.
    #[test]
    fn colour_is_per_vertex_and_sticky() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        g.begin(Shape::Points);
        g.color3f(1.0, 0.0, 0.0);
        g.vertex3f(0.0, 0.0, 0.0);
        g.color3f(0.0, 1.0, 0.0);
        g.vertex3f(1.0, 0.0, 0.0);
        g.vertex3f(2.0, 0.0, 0.0);
        g.end();
        let v = &g.frame().vertices;
        assert_eq!(v[0].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(v[1].color, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(v[2].color, [0.0, 1.0, 0.0, 1.0]);
    }

    /// There are no quads in OpenGL ES, so a quad has to arrive as two
    /// triangles that cover it and wind the same way.
    #[test]
    fn quads_are_cut_into_triangles() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        g.begin(Shape::Quads);
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            g.vertex3f(x, y, 0.0);
        }
        g.end();
        let f = g.frame();
        assert_eq!(f.batches[0].primitive, Primitive::Triangles);
        assert_eq!(f.batches[0].count, 6);
        let p: Vec<[f32; 3]> = f.vertices.iter().map(|v| v.pos).collect();
        assert_eq!(p[0], [0.0, 0.0, 0.0]);
        assert_eq!(p[2], [1.0, 1.0, 0.0]);
        assert_eq!(p[3], [0.0, 0.0, 0.0]);
        assert_eq!(p[5], [0.0, 1.0, 0.0]);
    }

    /// A quad strip's vertices zigzag, so its quads are not four in a row.
    #[test]
    fn a_quad_strip_zigzags() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        g.begin(Shape::QuadStrip);
        for i in 0..6 {
            g.vertex3f(i as f32, (i % 2) as f32, 0.0);
        }
        g.end();
        // Six vertices is two quads, so four triangles.
        assert_eq!(g.frame().batches[0].count, 12);
    }

    /// The one that makes lists worth having: the same list drawn twice under
    /// different matrices lands in two places.
    #[test]
    fn a_list_is_replayed_under_the_current_matrix() {
        let mut g = Glx::new();
        let list = g.gen_lists(1);
        g.new_list(list);
        g.begin(Shape::Points);
        g.vertex3f(0.0, 0.0, 0.0);
        g.end();
        g.end_list();

        g.start_frame(100, 100);
        assert!(g.frame().batches.is_empty(), "compiling must not draw");

        g.translate(5.0, 0.0, 0.0);
        g.call_list(list);
        g.translate(5.0, 0.0, 0.0);
        g.call_list(list);

        let f = g.frame();
        assert_eq!(f.batches.len(), 2);
        assert_eq!(f.batches[0].mvp.transform([0.0, 0.0, 0.0])[0], 5.0);
        assert_eq!(f.batches[1].mvp.transform([0.0, 0.0, 0.0])[0], 10.0);
    }

    /// A list is a stream of commands, so a matrix change inside one happens
    /// when it is called, not when it is compiled.
    #[test]
    fn a_list_can_move_the_matrix_itself() {
        let mut g = Glx::new();
        let list = g.gen_lists(1);
        g.new_list(list);
        g.push_matrix();
        g.translate(1.0, 0.0, 0.0);
        g.begin(Shape::Points);
        g.vertex3f(0.0, 0.0, 0.0);
        g.end();
        g.pop_matrix();
        g.end_list();

        g.start_frame(100, 100);
        g.call_list(list);
        assert_eq!(g.frame().batches[0].mvp.transform([0.0; 3])[0], 1.0);
        // And the pop inside it left the caller's matrix alone.
        assert_eq!(g.modelview(), Mat4::IDENTITY);
    }

    #[test]
    fn perspective_puts_the_near_plane_where_it_was_asked_to() {
        let mut g = Glx::new();
        g.matrix_mode_projection();
        g.load_identity();
        g.perspective(30.0, 1.0, 1.0, 100.0);
        // A point on the near plane comes out at the front of the clip volume.
        let z = g.projection().transform([0.0, 0.0, -1.0])[2];
        assert!((z + 1.0).abs() < 1e-5, "{z}");
        let z = g.projection().transform([0.0, 0.0, -100.0])[2];
        assert!((z - 1.0).abs() < 1e-4, "{z}");
    }

    /// Consecutive blocks of the same thing under the same state are one draw
    /// call, which is what makes a saver that draws a cube as forty-eight
    /// separate quads affordable.
    #[test]
    fn adjacent_blocks_of_triangles_are_folded_together() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        for i in 0..10 {
            g.begin(Shape::Quads);
            for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                g.vertex3f(x + i as f32, y, 0.0);
            }
            g.end();
        }
        let f = g.frame();
        assert_eq!(f.batches.len(), 1, "ten quads should be one batch");
        assert_eq!(f.batches[0].count, 60);
        assert_eq!(f.vertices.len(), 60);
    }

    /// But not across a state change, or the second lot would be drawn with
    /// the first lot's colours.
    #[test]
    fn a_state_change_breaks_the_run() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        for i in 0..3 {
            g.material_ambient_diffuse([i as f32 / 3.0, 0.0, 0.0, 1.0]);
            g.begin(Shape::Triangles);
            for k in 0..3 {
                g.vertex3f(k as f32, 0.0, 0.0);
            }
            g.end();
        }
        assert_eq!(g.frame().batches.len(), 3);
    }

    /// And never for a strip or a fan: joining two of those end to end would
    /// invent triangles across the seam.
    #[test]
    fn strips_are_never_folded_together() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        for i in 0..4 {
            g.begin(Shape::TriangleStrip);
            for k in 0..3 {
                g.vertex3f(k as f32, i as f32, 0.0);
            }
            g.end();
        }
        assert_eq!(g.frame().batches.len(), 4);
    }

    #[test]
    fn a_frame_starts_empty_every_time() {
        let mut g = Glx::new();
        for _ in 0..3 {
            g.start_frame(64, 48);
            g.begin(Shape::Triangles);
            for i in 0..3 {
                g.vertex3f(i as f32, 0.0, 0.0);
            }
            g.end();
            assert_eq!(g.frame().batches.len(), 1);
            assert_eq!(g.frame().vertices.len(), 3);
        }
    }
}
