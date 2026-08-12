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
    pub uv: [f32; 2],
}

/// A texture, as the savers build them: a block of RGBA bytes and the two
/// parameters they disagree on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Texture {
    pub width: i32,
    pub height: i32,
    /// `width * height * 4` bytes, or empty for a texture that was allocated
    /// but not filled: the size is reserved and the contents are black. That
    /// is what a texture only ever copied into from the screen wants, and it
    /// saves carrying a megabyte of zeroes across to the host.
    pub data: Vec<u8>,
    /// `GL_CLAMP_TO_EDGE` rather than `GL_REPEAT`, for a texture that is one
    /// picture rather than a tile.
    pub clamp: bool,
    /// `GL_NEAREST` rather than `GL_LINEAR`, for one that wants its pixels.
    pub nearest: bool,
    /// Bumped every time the image is replaced. Most savers upload once and
    /// never again; `cubenetic` rebuilds its texture every frame, and the host
    /// uses this to know which is which.
    pub generation: u32,
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
    /// `GL_DST_COLOR, GL_SRC_ALPHA`: the source is multiplied by what is
    /// already there, so it can only brighten where there is something to
    /// brighten. `lockward` flashes its blades with it, which is why a flash
    /// shows up on the spinner and not on the black around it.
    DstColorAlpha,
    /// `GL_DST_COLOR, GL_SRC_COLOR`: twice the product of the two, which is
    /// how `quasicrystal` tints a grey interference pattern without washing
    /// it out.
    DstColorSrcColor,
    /// `GL_ONE_MINUS_DST_COLOR, GL_ZERO`: replaces what is there with the
    /// source times its inverse, so it both inverts and darkens.
    InverseDst,
}

/// `GL_FOG`, in the two modes the savers ask for.
///
/// A saver reaches for fog when its scene runs off towards a horizon: without
/// it the far end of the geometry is drawn as brightly as the near end and
/// piles into an unreadable band. `gravitywell`'s grid is the case in point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fog {
    /// `GL_EXP`: what survives at a distance is `exp(-density * d)`.
    Exp { density: f32, color: [f32; 4] },
    /// `GL_EXP2`: what survives at a distance is `exp(-(density * d)^2)`.
    Exp2 { density: f32, color: [f32; 4] },
    /// `GL_LINEAR`: untouched up to `start`, gone by `end`.
    Linear {
        start: f32,
        end: f32,
        color: [f32; 4],
    },
}

/// `glTexEnvi (GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, ..)`: how a texel is
/// combined with the colour underneath it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TexEnv {
    /// `GL_MODULATE`, the default: the texture multiplies the colour.
    #[default]
    Modulate,
    /// `GL_ADD`: the colours add and the alphas multiply. `energystream` wants
    /// it so its flares pile up into white where they overlap.
    Add,
}

/// How many lights a saver can turn on. OpenGL guarantees eight; most of the
/// savers here use one or two and `bubble3d` uses three, and this grows when
/// one wants more. Every batch carries this many, so it is not free.
pub const MAX_LIGHTS: usize = 3;

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
    /// What the ambient light lands on, which is a separate colour in GL and
    /// almost never a separate colour in a saver: nearly all of them set
    /// `GL_AMBIENT_AND_DIFFUSE` and never think about it again. It matters for
    /// the few that set only `GL_DIFFUSE` and leave this at GL's dim grey, so
    /// that a strong scene ambient lifts their faces towards grey rather than
    /// towards their own colour.
    pub ambient: [f32; 4],
    pub back_ambient: [f32; 4],
    pub specular: [f32; 4],
    pub shininess: f32,
    /// `GL_EMISSION`: light the surface gives off itself, added on top of
    /// whatever falls on it. A saver reaches for it when something is meant to
    /// glow rather than be lit: `boxed`'s balls carry half their own colour
    /// this way, so they read as coloured even in the shadow of the box.
    pub emission: [f32; 4],
}

impl Default for Material {
    /// OpenGL's own defaults.
    fn default() -> Self {
        Material {
            ambient_diffuse: [0.8, 0.8, 0.8, 1.0],
            back_ambient_diffuse: [0.8, 0.8, 0.8, 1.0],
            ambient: [0.2, 0.2, 0.2, 1.0],
            back_ambient: [0.2, 0.2, 0.2, 1.0],
            specular: [0.0, 0.0, 0.0, 1.0],
            shininess: 0.0,
            emission: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// `glDepthFunc`. Only the three the savers ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DepthFunc {
    #[default]
    Less,
    LessEqual,
    /// Draw only where something is already at exactly this depth, which is
    /// how `molecule` shades its electron shells without them piling up.
    Equal,
}

/// `glStencilFunc`. Only the comparisons the savers ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StencilFunc {
    /// Draw whatever the buffer says, which is what a pass writing the mask
    /// wants: it is there to leave a mark, not to be masked itself.
    Always,
    Equal,
}

/// `glStencilFunc` with `glStencilOp`, cut down to what the savers ask for.
///
/// The stencil buffer is a per-pixel scribble pad the size of the screen, and
/// the chess savers use it for one thing: paint the board's tiles into it with
/// the colour mask off, then draw the pieces upside down under the board with
/// the test set to `Equal`, so a reflection appears on the tiles and nowhere
/// else. Neither of them needs an action for the failing cases, so a batch
/// carries a comparison, a reference value, and whether a fragment that passes
/// writes that value back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stencil {
    pub func: StencilFunc,
    pub reference: i32,
    /// `glStencilOp (GL_KEEP, GL_KEEP, GL_REPLACE)` rather than all `GL_KEEP`.
    pub write: bool,
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
    /// Clear the colour buffer too, which is what a *second* `glClear` in one
    /// frame means: what has been drawn so far was scaffolding, and the real
    /// picture starts here. `glblur` renders its scene small, copies it to a
    /// texture, and then wipes it to draw the blur instead.
    pub clear_color_first: bool,
    /// `glViewport`. Held per batch rather than per frame because a saver may
    /// shrink it, draw into the corner, and put it back; the frame carries the
    /// viewport it started with, for the clear.
    pub viewport: [i32; 4],
    pub blend: Blend,
    /// `glPolygonOffset`, and whether `GL_POLYGON_OFFSET_FILL` is on. A saver
    /// reaches for it when it draws two surfaces in the same place and needs
    /// one of them to win: the coplanar one is pushed back by a slope-scaled
    /// amount so it stops fighting for the depth buffer.
    pub polygon_offset: Option<(f32, f32)>,
    /// `glDepthMask`. Off for a translucent surface, so that its own faces
    /// blend with each other instead of the nearest one hiding the rest.
    pub depth_mask: bool,
    pub depth_func: DepthFunc,
    /// `glColorMask`, per channel. All false for a pass that is only there to
    /// fill the depth buffer, which is how a saver marks out where a later
    /// pass may draw. Some savers mask individual channels instead:
    /// `esper`'s flash writes only blue and alpha, which tints what is
    /// already on screen rather than covering it.
    pub color_mask: [bool; 4],
    pub point_size: f32,
    pub line_width: f32,
    /// `glEnable(GL_DEPTH_TEST)`. Off for the savers that draw a flat scene
    /// and want it in the order they drew it.
    pub depth_test: bool,
    /// `glLightModelfv (GL_LIGHT_MODEL_AMBIENT, ..)`: the light every surface
    /// gets whatever the lamps are doing.
    pub scene_ambient: [f32; 4],
    /// `glEnable(GL_FOG)`, and what it fades to.
    pub fog: Option<Fog>,
    /// `glAlphaFunc (GL_GEQUAL, ref)` with `glEnable (GL_ALPHA_TEST)`: throw
    /// the fragment away rather than blending it when its alpha comes out
    /// below `ref`. `GL_GEQUAL` is the only comparison the savers ask for.
    ///
    /// A saver reaches for it when a texture is a cut-out: `glforestfire`'s
    /// trees are quads whose background is transparent, and blending them
    /// would still write depth over the whole quad and punch a hole in the
    /// scene behind. Discarding writes nothing at all.
    pub alpha_test: Option<f32>,
    /// `glEnable (GL_STENCIL_TEST)` and the state that goes with it, or `None`
    /// for `glDisable`.
    pub stencil: Option<Stencil>,
    /// Which texture is bound, if `GL_TEXTURE_2D` is enabled.
    pub texture: Option<u32>,
    pub tex_env: TexEnv,
    /// `glEnable (GL_COLOR_MATERIAL)` with `GL_AMBIENT_AND_DIFFUSE`: a lit
    /// surface takes its colour from `glColor` rather than from the material.
    ///
    /// Most savers use this to say what `glMaterialfv` would have said and are
    /// ported as though they had; it matters where the colour varies *within* a
    /// block, since a material is one colour for the whole of it and a vertex
    /// colour is not.
    pub color_material: bool,
    /// `glEnable (GL_TEXTURE_GEN_S/T)` with `GL_SPHERE_MAP`: work the texture
    /// coordinates out from which way the surface faces rather than taking the
    /// ones the saver gave, so a texture of a room reflects off it.
    pub tex_gen_sphere: bool,
    /// `glCopyTexSubImage2D`: once this batch has been drawn, copy the screen
    /// into this texture. A batch carrying one usually has no vertices of its
    /// own and exists only to say where in the frame the copy happens.
    pub copy_to_texture: Option<u32>,
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
            && !other.clear_color_first
            && self.viewport == other.viewport
            && self.blend == other.blend
            && self.polygon_offset == other.polygon_offset
            && self.depth_mask == other.depth_mask
            && self.depth_func == other.depth_func
            && self.color_mask == other.color_mask
            && self.fog == other.fog
            && self.alpha_test == other.alpha_test
            && self.stencil == other.stencil
            && self.texture == other.texture
            && self.tex_env == other.tex_env
            && self.tex_gen_sphere == other.tex_gen_sphere
            && self.color_material == other.color_material
            && self.scene_ambient == other.scene_ambient
            // A copy has to happen where it was asked for, so a batch carrying
            // one never merges with its neighbours.
            && self.copy_to_texture.is_none()
            && other.copy_to_texture.is_none()
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
    TexCoord([f32; 2]),
    BindTexture(u32),
    Texturing(bool),
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
    scene_ambient: [f32; 4],
    fog: Option<Fog>,
    alpha_test: Option<f32>,
    stencil: Option<Stencil>,
    textures: Vec<Texture>,
    bound_texture: Option<u32>,
    texturing: bool,
    tex_gen_sphere: bool,
    color_material: bool,
    tex_env: TexEnv,
    uv: [f32; 2],
    clear_depth_pending: bool,
    clear_color_pending: bool,
    blend: Blend,
    polygon_offset: Option<(f32, f32)>,
    depth_mask: bool,
    depth_func: DepthFunc,
    color_mask: [bool; 4],
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
            scene_ambient: [0.2, 0.2, 0.2, 1.0],
            fog: None,
            alpha_test: None,
            stencil: None,
            textures: Vec::new(),
            bound_texture: None,
            texturing: false,
            tex_gen_sphere: false,
            color_material: false,
            tex_env: TexEnv::Modulate,
            uv: [0.0, 0.0],
            clear_depth_pending: false,
            clear_color_pending: false,
            blend: Blend::Off,
            polygon_offset: None,
            depth_mask: true,
            depth_func: DepthFunc::Less,
            color_mask: [true; 4],
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
        } else {
            // A second clear in one frame cannot be the frame's own; it wipes
            // what has been drawn so far, so it attaches to what comes next.
            self.clear_color_pending = true;
            self.clear_depth_pending = true;
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
    /// `glFogf (GL_FOG_DENSITY, ..)` and `glFogfv (GL_FOG_COLOR, ..)` with
    /// `glEnable (GL_FOG)`, or `None` for `glDisable`. Only `GL_EXP2` is
    /// implemented, which is the only mode any of these savers uses.
    pub fn fog(&mut self, fog: Option<Fog>) {
        self.fog = fog;
    }

    /// `glAlphaFunc (GL_GEQUAL, ref)` with `glEnable (GL_ALPHA_TEST)`, or
    /// `None` for `glDisable`.
    pub fn alpha_test(&mut self, reference: Option<f32>) {
        self.alpha_test = reference;
    }

    /// `glStencilFunc` and `glStencilOp` together, with `glEnable
    /// (GL_STENCIL_TEST)`, or `None` for `glDisable`.
    pub fn stencil(&mut self, stencil: Option<Stencil>) {
        self.stencil = stencil;
    }

    /// `glPolygonOffset`, with `None` for `glDisable(GL_POLYGON_OFFSET_FILL)`.
    pub fn polygon_offset(&mut self, offset: Option<(f32, f32)>) {
        self.polygon_offset = offset;
    }

    /// `glDepthMask`: whether what is drawn writes depth as well as reading it.
    pub fn depth_mask(&mut self, on: bool) {
        self.depth_mask = on;
    }

    pub fn depth_func(&mut self, f: DepthFunc) {
        self.depth_func = f;
    }

    /// `glColorMask`, all four channels at once, which is the only way the
    /// savers use it.
    pub fn color_mask(&mut self, on: bool) {
        self.color_mask = [on; 4];
    }

    /// `glColorMask` with the four channels given separately.
    pub fn color_mask_rgba(&mut self, m: [bool; 4]) {
        self.color_mask = m;
    }

    pub fn blend(&mut self, blend: Blend) {
        self.blend = blend;
    }

    /// `glFrontFace`: true for `GL_CW`, false for `GL_CCW`, which is the
    /// default. A saver whose faces are wound clockwise says so rather than
    /// having its outsides culled.
    pub fn front_face_cw(&mut self, cw: bool) {
        self.front_face_cw = cw;
    }

    /// `glGetIntegerv (GL_FRONT_FACE, ..)`, for code that has to set the
    /// winding for its own geometry and put the caller's back afterwards.
    pub fn front_face_cw_set(&self) -> bool {
        self.front_face_cw
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
        self.material.ambient = rgba;
        self.material.back_ambient = rgba;
    }

    /// `glMaterialfv (GL_FRONT_AND_BACK, GL_DIFFUSE, ..)`, and only the
    /// diffuse: the ambient colour stays where it was, which for a saver that
    /// never sets it is GL's dim grey.
    pub fn material_diffuse(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.ambient_diffuse = rgba;
        self.material.back_ambient_diffuse = rgba;
    }

    /// `glMaterialfv (GL_FRONT_AND_BACK, GL_AMBIENT, ..)`, and only the
    /// ambient: the diffuse stays where it was. `kallisti` is the saver that
    /// wants the two to be different colours, because gold is dark where the
    /// light does not reach it and bright where it does.
    pub fn material_ambient(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.ambient = rgba;
        self.material.back_ambient = rgba;
    }

    /// `glMaterialfv (GL_BACK, GL_AMBIENT_AND_DIFFUSE, ..)`: the inside of a
    /// surface, for a saver that wants it a different colour from the outside.
    /// Set it after the front, which sets both.
    pub fn material_back_ambient_diffuse(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.back_ambient_diffuse = rgba;
        self.material.back_ambient = rgba;
    }

    pub fn material_specular(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.specular = rgba;
    }

    pub fn material_shininess(&mut self, shininess: f32) {
        self.split_block();
        self.material.shininess = shininess;
    }

    /// `glMaterialfv (GL_FRONT_AND_BACK, GL_EMISSION, ..)`.
    pub fn material_emission(&mut self, rgba: [f32; 4]) {
        self.split_block();
        self.material.emission = rgba;
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

    /// `glTexCoord2f`. Like the colour and the normal, it rides on the next
    /// vertex rather than on the block.
    pub fn tex_coord2f(&mut self, s: f32, t: f32) {
        self.record_or(Cmd::TexCoord([s, t]), |g| g.uv = [s, t]);
    }

    /* Textures */

    /// `glGenTextures(1, ..)`. The name exists at once; it has no image
    /// until `tex_image_2d`, which is also OpenGL's rule.
    pub fn gen_texture(&mut self) -> u32 {
        self.textures.push(Texture::default());
        self.textures.len() as u32
    }

    /// `glBindTexture(GL_TEXTURE_2D, ..)`.
    pub fn bind_texture(&mut self, id: u32) {
        self.record_or(Cmd::BindTexture(id), |g| g.bound_texture = Some(id));
    }

    /// `glTexImage2D`, into whichever texture is bound. RGBA bytes, and
    /// nothing else: no mipmaps, no other formats, because no saver here asks
    /// for either.
    pub fn tex_image_2d(&mut self, width: i32, height: i32, data: Vec<u8>) {
        let Some(t) = self.bound_texture_mut() else {
            return;
        };
        t.width = width;
        t.height = height;
        t.data = data;
        t.generation += 1;
    }

    /// `glCopyTexSubImage2D (GL_TEXTURE_2D, 0, 0, 0, 0, 0, w, h)`: copy the
    /// screen, as far as it has been drawn, into the bound texture.
    ///
    /// This is how a saver keeps the previous frame. In the olden days one
    /// could draw into the front buffer and find one's pixels still there next
    /// time round; nothing guarantees that now, so a saver that piles frame on
    /// frame saves the result and draws it back at the top of the next one.
    /// `noof` accumulates its flowers this way.
    ///
    /// Recorded as a batch of its own, so the copy happens where it was asked
    /// for rather than at the end of the frame.
    pub fn copy_tex_sub_image_2d(&mut self) {
        let Some(id) = self.bound_texture else {
            return;
        };
        self.flush();
        let first = self.frame.vertices.len();
        let mut b = self.batch_state(Primitive::Points, first, 0);
        b.copy_to_texture = Some(id);
        self.frame.batches.push(b);
    }

    /// `glEnable (GL_TEXTURE_GEN_S)` and `GL_TEXTURE_GEN_T` with a mode of
    /// `GL_SPHERE_MAP`, which is the only one any saver here asks for.
    ///
    /// The texture coordinates stop coming from `glTexCoord` and are worked out
    /// per vertex from which way the surface faces, as though the texture were
    /// a photograph of the room taken in a mirrored ball. It is the cheap way
    /// to make something look shiny, and six of these savers use it.
    pub fn tex_gen_sphere(&mut self, on: bool) {
        self.tex_gen_sphere = on;
    }

    /// Whether the depth test, lighting and fog are on, so that something
    /// drawing an overlay can put them back the way it found them.
    pub fn depth_test_enabled(&self) -> bool {
        self.depth_test
    }

    pub fn lighting_enabled(&self) -> bool {
        self.lighting
    }

    pub fn fog_set(&self) -> Option<Fog> {
        self.fog
    }

    /// `glEnable (GL_COLOR_MATERIAL)`: see [`Batch::color_material`].
    pub fn color_material(&mut self, on: bool) {
        self.color_material = on;
    }

    /// `glTexParameteri (GL_TEXTURE_2D, GL_TEXTURE_WRAP_S/T, ..)`: clamp at
    /// the edges rather than repeat.
    pub fn tex_clamp(&mut self, clamp: bool) {
        if let Some(t) = self.bound_texture_mut() {
            t.clamp = clamp;
        }
    }

    /// `glTexParameteri (GL_TEXTURE_2D, GL_TEXTURE_MIN/MAG_FILTER, ..)`: take
    /// the nearest texel rather than blending the four around the point.
    pub fn tex_nearest(&mut self, nearest: bool) {
        if let Some(t) = self.bound_texture_mut() {
            t.nearest = nearest;
        }
    }

    fn bound_texture_mut(&mut self) -> Option<&mut Texture> {
        let id = self.bound_texture?;
        self.textures.get_mut(id as usize - 1)
    }

    /// `glLightModelfv (GL_LIGHT_MODEL_AMBIENT, ..)`. OpenGL defaults it to a
    /// fifth, which is what a saver that never mentions it gets.
    pub fn light_model_ambient(&mut self, rgba: [f32; 4]) {
        self.scene_ambient = rgba;
    }

    /// `glTexEnvi (GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, ..)`.
    pub fn tex_env(&mut self, env: TexEnv) {
        self.tex_env = env;
    }

    /// `glEnable`/`glDisable` of `GL_TEXTURE_2D`.
    pub fn texturing(&mut self, on: bool) {
        self.record_or(Cmd::Texturing(on), |g| g.texturing = on);
    }

    /// What a texture holds, for the host to upload. Returns `None` for a
    /// name that was generated but never given an image.
    #[must_use]
    pub fn texture(&self, id: u32) -> Option<&Texture> {
        let t = self.textures.get(id as usize - 1)?;
        (t.width > 0 && t.height > 0).then_some(t)
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
                uv: g.uv,
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
        let batch = self.batch_state(primitive, first, count);

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

    /// Everything a batch opening now would carry, bar its vertices. Taken
    /// rather than read, in the case of a pending depth clear: whichever batch
    /// is built first is the one that does the clearing.
    fn batch_state(&mut self, primitive: Primitive, first: usize, count: usize) -> Batch {
        Batch {
            primitive,
            first,
            count,
            mvp: self.projection().mul(&self.modelview()),
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
            clear_color_first: std::mem::take(&mut self.clear_color_pending),
            viewport: self.frame.viewport,
            blend: self.blend,
            polygon_offset: self.polygon_offset,
            depth_mask: self.depth_mask,
            depth_func: self.depth_func,
            color_mask: self.color_mask,
            line_width: self.line_width,
            scene_ambient: self.scene_ambient,
            fog: self.fog,
            alpha_test: self.alpha_test,
            stencil: self.stencil,
            texture: if self.texturing {
                self.bound_texture
            } else {
                None
            },
            tex_env: self.tex_env,
            tex_gen_sphere: self.tex_gen_sphere,
            color_material: self.color_material,
            copy_to_texture: None,
        }
    }

    /// `glGetFloatv (GL_MODELVIEW_MATRIX, ..)`. A few savers read the matrix
    /// back to find out where something they have just positioned ended up.
    pub fn modelview_matrix(&self) -> Mat4 {
        self.modelview()
    }

    fn projection(&self) -> Mat4 {
        self.projection.last().copied().unwrap_or(Mat4::IDENTITY)
    }

    /// `glGetFloatv (GL_PROJECTION_MATRIX, ..)`. With the modelview, this is
    /// what a saver needs to work out `gluProject` for itself: `winduprobot`
    /// depth sorts its robots that way.
    pub fn projection_matrix(&self) -> Mat4 {
        self.projection()
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
            Cmd::TexCoord(uv) => self.uv = uv,
            Cmd::BindTexture(id) => self.bound_texture = Some(id),
            Cmd::Texturing(on) => self.texturing = on,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screenshot has to happen where the saver asked for it, so it lands as
    /// a batch of its own between what it is meant to catch and what comes
    /// after, and neighbouring blocks never fold over it.
    #[test]
    fn a_screenshot_holds_its_place_in_the_frame() {
        let mut g = Glx::new();
        g.start_frame(100, 100);
        let tex = g.gen_texture();
        g.bind_texture(tex);
        g.tex_image_2d(128, 128, Vec::new());

        let dot = |g: &mut Glx| {
            g.begin(Shape::Points);
            g.vertex3f(0.0, 0.0, 0.0);
            g.end();
        };
        dot(&mut g);
        g.copy_tex_sub_image_2d();
        dot(&mut g);

        let b = &g.frame().batches;
        assert_eq!(b.len(), 3, "the two dots merged across the screenshot");
        assert_eq!(b[0].copy_to_texture, None);
        assert_eq!(b[1].copy_to_texture, Some(tex));
        assert_eq!(b[1].count, 0, "a screenshot draws nothing itself");
        assert_eq!(b[2].copy_to_texture, None);

        // Without one in the way the same two dots are one batch, which is
        // what the assertion above is worth checking against.
        g.start_frame(100, 100);
        dot(&mut g);
        dot(&mut g);
        assert_eq!(g.frame().batches.len(), 1);
    }

    /// A texture with no bytes is a size and nothing else, for one that is only
    /// ever copied into.
    #[test]
    fn a_texture_can_be_reserved_without_an_image() {
        let mut g = Glx::new();
        let tex = g.gen_texture();
        assert!(g.texture(tex).is_none(), "no size yet, so no texture yet");
        g.bind_texture(tex);
        g.tex_image_2d(64, 32, Vec::new());
        let t = g.texture(tex).expect("a reserved texture is a texture");
        assert_eq!((t.width, t.height), (64, 32));
        assert!(t.data.is_empty());
    }

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
