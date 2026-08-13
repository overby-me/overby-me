//! The WebGL2 half of the OpenGL savers.
//!
//! [`xscreensaver::runtime::gl`] takes a saver's `glBegin`/`glVertex`/`glEnd`
//! and hands back a [`Frame`]: one vertex buffer, and a list of batches saying
//! which run of it to draw, as what, and under which matrix. This uploads that
//! and draws it, which is all there is to it. Everything interesting happened
//! on the other side of the boundary, where it can be tested.
//!
//! There is one shader pair, because the fixed-function pipeline these savers
//! were written for had one. It will grow: lighting and texturing are both
//! fixed-function state, so both end up as branches in here, driven by what the
//! saver asked for.

use std::collections::HashMap;

use wasm_bindgen::JsCast;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlContextAttributes,
    WebGlFramebuffer, WebGlProgram, WebGlTexture, WebGlUniformLocation, WebGlVertexArrayObject,
};
use xscreensaver::SaverDef;
use xscreensaver::runtime::Runner3d;
use xscreensaver::runtime::XEvent;
use xscreensaver::runtime::XImage;
use xscreensaver::runtime::gl::{
    Blend, DepthFunc, Fog, Frame, MAX_LIGHTS, Primitive, StencilFunc, StencilOp, TexEnv,
};

/// Position, colour and normal, in the order [`Frame`] holds them.
const FLOATS_PER_VERTEX: usize = 12;

const VERTEX_SHADER: &str = r"#version 300 es
in vec3 a_pos;
in vec4 a_color;
in vec3 a_normal;
in vec2 a_uv;
uniform mat4 u_mvp;
uniform mat4 u_modelview;
uniform float u_point_size;
uniform bool u_texgen_sphere;
out vec4 v_color;
out vec3 v_normal;
out vec3 v_eye;
out vec2 v_uv;
void main() {
  gl_Position = u_mvp * vec4 (a_pos, 1.0);
  gl_PointSize = u_point_size;
  v_color = a_color;
  vec4 eye = u_modelview * vec4 (a_pos, 1.0);
  v_eye = eye.xyz / eye.w;
  // The upper 3x3 of the modelview, which is the normal matrix as long as the
  // scaling is uniform. Every saver here scales uniformly, and OpenGL's own
  // GL_NORMALIZE, which they all enable, is the normalize below.
  v_normal = mat3 (u_modelview) * a_normal;

  // GL_SPHERE_MAP: the texture is treated as a photograph taken in a mirrored
  // ball, and where a vertex reads from it is where the view reflects off the
  // surface. Fixed-function GL works this out per vertex, so this does too.
  if (u_texgen_sphere) {
    vec3 u = normalize (v_eye);
    vec3 n = normalize (v_normal);
    vec3 r = u - 2.0 * n * dot (n, u);
    float m = 2.0 * sqrt (r.x*r.x + r.y*r.y + (r.z + 1.0)*(r.z + 1.0));
    v_uv = vec2 (r.x / m + 0.5, r.y / m + 0.5);
  } else {
    v_uv = a_uv;
  }
}
";

/// The fixed-function lighting equation, for as many lights as any of these
/// savers turns on, with distance attenuation. Shading is per fragment rather
/// than per vertex as OpenGL 1.3 did it; on the low-polygon shapes these draw
/// that is a visible improvement and never a difference in what is depicted.
const FRAGMENT_SHADER: &str = r"#version 300 es
precision highp float;
in vec4 v_color;
in vec3 v_normal;
in vec3 v_eye;
in vec2 v_uv;
#define LIGHTS 3
uniform bool u_lighting;
uniform bool u_light_on[LIGHTS];
uniform vec4 u_light_position[LIGHTS];
uniform vec4 u_light_ambient[LIGHTS];
uniform vec4 u_light_diffuse[LIGHTS];
uniform vec4 u_light_specular[LIGHTS];
// Constant, linear and quadratic terms of 1 / (c + l*d + q*d*d).
uniform vec3 u_light_attenuation[LIGHTS];
uniform vec4 u_material_diffuse;
uniform vec4 u_material_diffuse_back;
uniform vec4 u_material_ambient;
uniform vec4 u_material_ambient_back;
uniform vec4 u_material_specular;
uniform vec4 u_material_emission;
uniform float u_shininess;
uniform vec4 u_scene_ambient;
uniform bool u_color_material;
uniform bool u_textured;
uniform sampler2D u_tex;
// 0 is GL_MODULATE, 1 is GL_ADD, 2 is GL_REPLACE.
uniform int u_tex_env;
uniform bool u_fog;
// 0 is GL_EXP2, 1 is GL_LINEAR, 2 is GL_EXP.
uniform int u_fog_mode;
uniform float u_fog_density;
uniform float u_fog_start;
uniform float u_fog_end;
uniform vec4 u_fog_color;
// GL_ALPHA_TEST with GL_GEQUAL: anything below this is thrown away rather
// than blended. Zero when the test is off, which nothing can fall below.
uniform float u_alpha_ref;
out vec4 frag_color;

// The three texture environments these savers ask for. GL_MODULATE, the
// default, multiplies the texture into whatever colour came out of the
// lighting; GL_ADD sums the colours and multiplies the alphas; GL_REPLACE
// discards the colour and shows the texture alone.
vec4 textured (vec4 c) {
  if (! u_textured) return c;
  vec4 t = texture (u_tex, v_uv);
  if (u_tex_env == 2) return t;
  if (u_tex_env == 1) return vec4 (c.rgb + t.rgb, c.a * t.a);
  return c * t;
}

// The two fog modes these savers ask for. GL_EXP2 leaves exp(-(density*d)^2)
// of the colour at a distance; GL_LINEAR ramps from all of it to none of it
// between two distances.
vec4 fogged (vec4 c) {
  if (! u_fog) return c;
  float z = length (v_eye);
  float f;
  if (u_fog_mode == 1) {
    f = (u_fog_end - z) / (u_fog_end - u_fog_start);
  } else if (u_fog_mode == 2) {
    f = exp (-u_fog_density * z);
  } else {
    float d = u_fog_density * z;
    f = exp (-d * d);
  }
  return vec4 (mix (u_fog_color.rgb, c.rgb, clamp (f, 0.0, 1.0)), c.a);
}

void main() {
  if (! u_lighting) {
    frag_color = fogged (textured (v_color));
    if (frag_color.a < u_alpha_ref) discard;
    return;
  }
  vec3 n = normalize (v_normal);
  vec3 eye = normalize (-v_eye);
  // Two-sided: a back face is lit by its own side. OpenGL would cull or shade
  // it separately, and the savers that leave culling off want to see both.
  if (dot (n, eye) < 0.0) n = -n;
  // Which side of the surface this is, by winding rather than by normal, since
  // that is what GL_FRONT and GL_BACK mean. The two are the same colour unless
  // the saver asked for a different one inside.
  // GL_COLOR_MATERIAL: the colour comes from the vertex rather than the
  // material, which is the only way a lit surface can be more than one colour.
  vec4 diffuse = u_color_material
               ? v_color
               : (gl_FrontFacing ? u_material_diffuse : u_material_diffuse_back);

  // What the ambient light lands on is its own material colour in GL. It is
  // the same as the diffuse for nearly every saver, and deliberately is not
  // for the few that set only GL_DIFFUSE under a strong scene ambient.
  vec3 ambient = u_color_material
               ? v_color.rgb
               : (gl_FrontFacing ? u_material_ambient.rgb : u_material_ambient_back.rgb);

  // GL_EMISSION: what the surface gives off itself, before anything lands
  // on it.
  vec3 c = u_material_emission.rgb + ambient * u_scene_ambient.rgb;
  for (int i = 0; i < LIGHTS; i++) {
    if (! u_light_on[i]) continue;
    vec4 p = u_light_position[i];
    // A w of zero is a light infinitely far away, so its position is a
    // direction. Otherwise it is a homogeneous point, and w has to be divided
    // out before the direction to it means anything.
    vec3 to = (p.w == 0.0 ? p.xyz : p.xyz / p.w - v_eye);
    vec3 l = normalize (to);
    // A light infinitely far away does not attenuate; a positional one falls
    // off with the distance to the fragment.
    vec3 k = u_light_attenuation[i];
    float att = (p.w == 0.0)
              ? 1.0
              : 1.0 / max (k.x + length (to) * (k.y + length (to) * k.z), 1e-6);
    vec3 lc = vec3 (0.0);
    lc += ambient * u_light_ambient[i].rgb;
    lc += diffuse.rgb * u_light_diffuse[i].rgb * max (dot (n, l), 0.0);
    if (u_shininess > 0.0) {
      vec3 h = normalize (l + eye);
      lc += u_material_specular.rgb * u_light_specular[i].rgb
          * pow (max (dot (n, h), 0.0), u_shininess);
    }
    c += lc * att;
  }
  frag_color = fogged (textured (vec4 (c, diffuse.a)));
  if (frag_color.a < u_alpha_ref) discard;
}
";

/// Where every uniform of the one program lives, looked up once.
struct Uniforms {
    mvp: Option<WebGlUniformLocation>,
    modelview: Option<WebGlUniformLocation>,
    point_size: Option<WebGlUniformLocation>,
    lighting: Option<WebGlUniformLocation>,
    /// One location per light, since GLSL arrays are addressed by element.
    light_on: Vec<Option<WebGlUniformLocation>>,
    light_position: Vec<Option<WebGlUniformLocation>>,
    light_ambient: Vec<Option<WebGlUniformLocation>>,
    light_attenuation: Vec<Option<WebGlUniformLocation>>,
    light_diffuse: Vec<Option<WebGlUniformLocation>>,
    light_specular: Vec<Option<WebGlUniformLocation>>,
    material_diffuse: Option<WebGlUniformLocation>,
    material_diffuse_back: Option<WebGlUniformLocation>,
    material_ambient: Option<WebGlUniformLocation>,
    material_ambient_back: Option<WebGlUniformLocation>,
    material_specular: Option<WebGlUniformLocation>,
    material_emission: Option<WebGlUniformLocation>,
    shininess: Option<WebGlUniformLocation>,
    scene_ambient: Option<WebGlUniformLocation>,
    color_material: Option<WebGlUniformLocation>,
    textured: Option<WebGlUniformLocation>,
    texgen_sphere: Option<WebGlUniformLocation>,
    tex: Option<WebGlUniformLocation>,
    tex_env: Option<WebGlUniformLocation>,
    alpha_ref: Option<WebGlUniformLocation>,
    fog_mode: Option<WebGlUniformLocation>,
    fog_start: Option<WebGlUniformLocation>,
    fog_end: Option<WebGlUniformLocation>,
    fog: Option<WebGlUniformLocation>,
    fog_density: Option<WebGlUniformLocation>,
    fog_color: Option<WebGlUniformLocation>,
}

impl Uniforms {
    fn of(gl: &Gl, program: &WebGlProgram) -> Self {
        let at = |name: &str| gl.get_uniform_location(program, name);
        Uniforms {
            mvp: at("u_mvp"),
            modelview: at("u_modelview"),
            point_size: at("u_point_size"),
            lighting: at("u_lighting"),
            light_on: (0..MAX_LIGHTS)
                .map(|i| at(&format!("u_light_on[{i}]")))
                .collect(),
            light_position: (0..MAX_LIGHTS)
                .map(|i| at(&format!("u_light_position[{i}]")))
                .collect(),
            light_attenuation: (0..MAX_LIGHTS)
                .map(|i| at(&format!("u_light_attenuation[{i}]")))
                .collect(),
            light_ambient: (0..MAX_LIGHTS)
                .map(|i| at(&format!("u_light_ambient[{i}]")))
                .collect(),
            light_diffuse: (0..MAX_LIGHTS)
                .map(|i| at(&format!("u_light_diffuse[{i}]")))
                .collect(),
            light_specular: (0..MAX_LIGHTS)
                .map(|i| at(&format!("u_light_specular[{i}]")))
                .collect(),
            material_diffuse: at("u_material_diffuse"),
            material_diffuse_back: at("u_material_diffuse_back"),
            material_ambient: at("u_material_ambient"),
            material_ambient_back: at("u_material_ambient_back"),
            material_specular: at("u_material_specular"),
            material_emission: at("u_material_emission"),
            shininess: at("u_shininess"),
            scene_ambient: at("u_scene_ambient"),
            color_material: at("u_color_material"),
            textured: at("u_textured"),
            texgen_sphere: at("u_texgen_sphere"),
            tex: at("u_tex"),
            tex_env: at("u_tex_env"),
            alpha_ref: at("u_alpha_ref"),
            fog_mode: at("u_fog_mode"),
            fog_start: at("u_fog_start"),
            fog_end: at("u_fog_end"),
            fog: at("u_fog"),
            fog_density: at("u_fog_density"),
            fog_color: at("u_fog_color"),
        }
    }
}

pub struct Gl3dEngine {
    gl: Gl,
    runner: Runner3d,
    program: WebGlProgram,
    uniforms: Uniforms,
    vao: WebGlVertexArrayObject,
    vbo: WebGlBuffer,
    /// Vertices the buffer has room for, so a frame the same size as the last
    /// one is a sub-upload rather than a reallocation.
    capacity: usize,
    scratch: Vec<f32>,
    /// Textures the saver has built, uploaded once and kept. A saver makes
    /// them when it starts and refers to them by name from then on.
    textures: HashMap<u32, (WebGlTexture, u32)>,
    /// Somewhere to point a texture at when a saver asks for a screenshot of
    /// its own frame. Made on the first one; most savers never ask.
    copy_fbo: Option<WebGlFramebuffer>,
}

impl Gl3dEngine {
    pub fn new(canvas: &HtmlCanvasElement, runner: Runner3d) -> Option<Self> {
        // Depth is on: these are 3D. Stencil is on for the two chess savers,
        // which mask their reflections to the board with it. Antialiasing is
        // left at the browser's default, unlike the Shadertoy engine, and the
        // saver that wants a screenshot of its own frame resolves the
        // multisampling with a blit.
        let options = WebGlContextAttributes::new();
        options.set_depth(true);
        options.set_stencil(true);
        options.set_alpha(false);
        let gl: Gl = canvas
            .get_context_with_context_options("webgl2", &options)
            .ok()
            .flatten()?
            .dyn_into()
            .ok()?;

        let program = link(&gl, VERTEX_SHADER, FRAGMENT_SHADER)?;
        let position = gl.get_attrib_location(&program, "a_pos");
        let color = gl.get_attrib_location(&program, "a_color");
        let normal = gl.get_attrib_location(&program, "a_normal");
        let uv = gl.get_attrib_location(&program, "a_uv");
        if position < 0 || color < 0 || normal < 0 || uv < 0 {
            log::error!("gl3d: the shader is missing an attribute");
            return None;
        }

        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        let stride = (FLOATS_PER_VERTEX * 4) as i32;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&vbo));
        for (location, size, offset) in [
            (position, 3, 0),
            (color, 4, 12),
            (normal, 3, 28),
            (uv, 2, 40),
        ] {
            gl.vertex_attrib_pointer_with_i32(
                location as u32,
                size,
                Gl::FLOAT,
                false,
                stride,
                offset,
            );
            gl.enable_vertex_attrib_array(location as u32);
        }
        gl.bind_vertex_array(None);

        Some(Gl3dEngine {
            uniforms: Uniforms::of(&gl, &program),
            gl,
            runner,
            program,
            vao,
            vbo,
            capacity: 0,
            scratch: Vec::new(),
            textures: HashMap::new(),
            copy_fbo: None,
        })
    }

    pub fn def(&self) -> &'static SaverDef {
        self.runner.def()
    }

    /// Host side: what codepoint the saver is waiting for.
    pub fn take_glyph_request(&mut self) -> Option<(u32, i32)> {
        self.runner.take_glyph_request()
    }

    /// Host side: hand back a drawn glyph.
    pub fn deliver_glyph(&mut self, codepoint: u32, image: Option<XImage>) {
        self.runner.deliver_glyph(codepoint, image);
    }

    /// Host side: what map tiles the saver is waiting for.
    pub fn take_tile_requests(&mut self) -> Vec<(u64, String)> {
        self.runner.take_tile_requests()
    }

    /// Host side: hand back a fetched tile, or `None` if it could not be had.
    pub fn deliver_tile(&mut self, key: u64, image: Option<XImage>) {
        self.runner.deliver_tile(key, image);
    }

    /// Host side: has the saver asked for a picture? Eleven of the 3D savers
    /// do; `photopile` and `glslideshow` are nothing but photographs.
    pub fn take_image_request(&mut self) -> bool {
        self.runner.take_image_request()
    }

    /// Host side: does this saver work on a picture at all?
    pub fn hack_uses_images(&self) -> bool {
        self.runner.hack_uses_images()
    }

    /// Host side: hand the saver a decoded picture.
    pub fn deliver_image(&mut self, image: XImage, title: Option<String>) {
        self.runner.deliver_image(image, title);
    }

    /// The caption of the picture on screen, if the host gave one.
    pub fn image_title(&self) -> Option<String> {
        self.runner.image_title().map(str::to_string)
    }

    /// Host side: has the saver asked for words? `starwars` and `fliptext`
    /// are the 3D ones that read text.
    pub fn take_text_request(&mut self) -> bool {
        self.runner.take_text_request()
    }

    /// Host side: hand the saver some words.
    pub fn deliver_text(&mut self, s: &str) {
        self.runner.deliver_text(s);
    }

    /// Run a different saver, or the same one with different knobs, on the
    /// context we already have.
    pub fn restart(&mut self, runner: Runner3d) {
        self.runner = runner;
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.runner.resize(width, height);
    }

    pub fn event(&mut self, event: &XEvent) -> bool {
        self.runner.event(*event)
    }

    pub fn draw(&mut self, now: f64) {
        self.runner.tick(now);
        let gl = self.gl.clone();
        // Split the borrow: uploading reads the frame while `self.scratch` is
        // written, and both live on `self`.
        let Gl3dEngine {
            runner,
            scratch,
            textures,
            copy_fbo,
            ..
        } = self;
        let frame: &Frame = runner.frame();

        let [vx, vy, vw, vh] = frame.viewport;
        gl.viewport(vx, vy, vw.max(1), vh.max(1));

        if let Some([r, g, b, a]) = frame.clear {
            // A clear writes through the masks, so put them back first: the
            // last batch of the frame before may well have left one off.
            gl.color_mask(true, true, true, true);
            gl.depth_mask(true);
            gl.clear_color(r, g, b, a);
            gl.clear(Gl::COLOR_BUFFER_BIT | Gl::DEPTH_BUFFER_BIT | Gl::STENCIL_BUFFER_BIT);
        }
        if frame.batches.is_empty() {
            return;
        }

        scratch.clear();
        scratch.reserve(frame.vertices.len() * FLOATS_PER_VERTEX);
        for v in &frame.vertices {
            scratch.extend_from_slice(&v.pos);
            scratch.extend_from_slice(&v.color);
            scratch.extend_from_slice(&v.normal);
            scratch.extend_from_slice(&v.uv);
        }

        gl.bind_vertex_array(Some(&self.vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.vbo));
        unsafe {
            // SAFETY: the view borrows the wasm heap and is handed straight to
            // a buffer call, which copies before anything can grow the heap.
            let view = js_sys::Float32Array::view(scratch);
            if frame.vertices.len() > self.capacity {
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::DYNAMIC_DRAW);
                self.capacity = frame.vertices.len();
            } else {
                gl.buffer_sub_data_with_i32_and_array_buffer_view(Gl::ARRAY_BUFFER, 0, &view);
            }
        }

        // Upload any texture this frame refers to that we have not seen, or
        // that the saver has redrawn. Most build theirs once at startup and
        // never touch it again; `cubenetic` rebuilds its every frame, which is
        // what the generation counter is for.
        for id in frame
            .batches
            .iter()
            .flat_map(|b| [b.texture, b.copy_to_texture])
            .flatten()
        {
            let Some(t) = runner.texture(id) else {
                continue;
            };
            if textures.get(&id).is_some_and(|(_, g)| *g == t.generation) {
                continue;
            }
            let handle = match textures.get(&id) {
                Some((h, _)) => h.clone(),
                None => match gl.create_texture() {
                    Some(h) => h,
                    None => continue,
                },
            };
            gl.bind_texture(Gl::TEXTURE_2D, Some(&handle));
            // A texture with no bytes is one that is only ever copied into from
            // the screen. It is allocated without an alpha channel, because
            // the drawing buffer has none either and a blit out of a
            // multisampled buffer is only legal between identical formats.
            // Sampling it then gives an alpha of one, which is what upstream
            // gets from the GL_LUMINANCE it copies to.
            let format = if t.data.is_empty() { Gl::RGB } else { Gl::RGBA };
            let ok = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
                Gl::TEXTURE_2D,
                0,
                format as i32,
                t.width,
                t.height,
                0,
                format,
                Gl::UNSIGNED_BYTE,
                // No bytes means reserve the size and leave it black, which is
                // what a texture that is only ever copied into asks for. WebGL
                // guarantees the zero fill, so there is nothing to send.
                if t.data.is_empty() {
                    None
                } else {
                    Some(&t.data)
                },
            );
            if ok.is_err() {
                log::error!("gl3d: texture {id} would not upload");
                continue;
            }
            // The two parameters the savers disagree on. Everything else is
            // left at the default.
            let wrap = if t.clamp {
                Gl::CLAMP_TO_EDGE
            } else {
                Gl::REPEAT
            };
            let filter = if t.nearest { Gl::NEAREST } else { Gl::LINEAR };
            for (p, v) in [
                (Gl::TEXTURE_WRAP_S, wrap),
                (Gl::TEXTURE_WRAP_T, wrap),
                (Gl::TEXTURE_MIN_FILTER, filter),
                (Gl::TEXTURE_MAG_FILTER, filter),
            ] {
                gl.tex_parameteri(Gl::TEXTURE_2D, p, v as i32);
            }
            textures.insert(id, (handle, t.generation));
        }

        gl.use_program(Some(&self.program));
        let u = &self.uniforms;

        for batch in &frame.batches {
            if batch.depth_test {
                gl.enable(Gl::DEPTH_TEST);
            } else {
                gl.disable(Gl::DEPTH_TEST);
            }
            if batch.cull_face {
                gl.enable(Gl::CULL_FACE);
            } else {
                gl.disable(Gl::CULL_FACE);
            }
            gl.front_face(if batch.front_face_cw { Gl::CW } else { Gl::CCW });
            gl.depth_mask(batch.depth_mask);
            gl.depth_func(match batch.depth_func {
                DepthFunc::Less => Gl::LESS,
                DepthFunc::LessEqual => Gl::LEQUAL,
                DepthFunc::Equal => Gl::EQUAL,
            });
            let m = batch.color_mask;
            gl.color_mask(m[0], m[1], m[2], m[3]);
            match batch.stencil {
                Some(s) => {
                    gl.enable(Gl::STENCIL_TEST);
                    gl.stencil_func(
                        match s.func {
                            StencilFunc::Always => Gl::ALWAYS,
                            StencilFunc::Equal => Gl::EQUAL,
                            StencilFunc::NotEqual => Gl::NOTEQUAL,
                        },
                        s.reference,
                        !0,
                    );
                    gl.stencil_op(
                        Gl::KEEP,
                        Gl::KEEP,
                        match s.pass {
                            StencilOp::Keep => Gl::KEEP,
                            StencilOp::Replace => Gl::REPLACE,
                            StencilOp::Incr => Gl::INCR,
                        },
                    );
                }
                None => gl.disable(Gl::STENCIL_TEST),
            }
            match batch.polygon_offset {
                Some((factor, units)) => {
                    gl.enable(Gl::POLYGON_OFFSET_FILL);
                    gl.polygon_offset(factor, units);
                }
                None => gl.disable(Gl::POLYGON_OFFSET_FILL),
            }
            let [bx, by, bw, bh] = batch.viewport;
            gl.viewport(bx, by, bw.max(1), bh.max(1));
            let mut bits = 0;
            if batch.clear_color_first {
                bits |= Gl::COLOR_BUFFER_BIT | Gl::DEPTH_BUFFER_BIT;
            } else if batch.clear_depth_first {
                bits |= Gl::DEPTH_BUFFER_BIT;
            }
            if batch.clear_stencil_first {
                bits |= Gl::STENCIL_BUFFER_BIT;
            }
            if bits != 0 {
                gl.clear(bits);
            }
            match batch.blend {
                Blend::Off => gl.disable(Gl::BLEND),
                Blend::Add => {
                    gl.enable(Gl::BLEND);
                    gl.blend_func(Gl::ONE, Gl::ONE);
                }
                Blend::AlphaAdd => {
                    gl.enable(Gl::BLEND);
                    gl.blend_func(Gl::SRC_ALPHA, Gl::ONE);
                }
                Blend::Alpha => {
                    gl.enable(Gl::BLEND);
                    gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);
                }
                Blend::DstColorAlpha => {
                    gl.enable(Gl::BLEND);
                    gl.blend_func(Gl::DST_COLOR, Gl::SRC_ALPHA);
                }
                Blend::DstColorSrcColor => {
                    gl.enable(Gl::BLEND);
                    gl.blend_func(Gl::DST_COLOR, Gl::SRC_COLOR);
                }
                Blend::InverseDst => {
                    gl.enable(Gl::BLEND);
                    gl.blend_func(Gl::ONE_MINUS_DST_COLOR, Gl::ZERO);
                }
                Blend::ConstantFade(a) => {
                    gl.enable(Gl::BLEND);
                    gl.blend_color(0.0, 0.0, 0.0, a);
                    gl.blend_func(Gl::CONSTANT_ALPHA, Gl::ONE_MINUS_CONSTANT_ALPHA);
                }
                Blend::ConstantAdd(a) => {
                    gl.enable(Gl::BLEND);
                    gl.blend_color(0.0, 0.0, 0.0, a);
                    gl.blend_func(Gl::CONSTANT_ALPHA, Gl::ONE);
                }
                Blend::ConstantSubtract(a) => {
                    gl.enable(Gl::BLEND);
                    gl.blend_color(0.0, 0.0, 0.0, a);
                    gl.blend_func(Gl::CONSTANT_ALPHA, Gl::ONE);
                }
            }
            // The equation is separate state from the factors, and only one
            // batch in the collection ever changes it, so it is set back
            // rather than left for the next batch to find.
            gl.blend_equation(match batch.blend {
                Blend::ConstantSubtract(_) => Gl::FUNC_REVERSE_SUBTRACT,
                _ => Gl::FUNC_ADD,
            });
            gl.uniform_matrix4fv_with_f32_array(u.mvp.as_ref(), false, &batch.mvp.0);
            gl.uniform_matrix4fv_with_f32_array(u.modelview.as_ref(), false, &batch.modelview.0);
            match batch.texture.and_then(|id| textures.get(&id)) {
                Some((t, _)) => {
                    gl.uniform1i(u.textured.as_ref(), 1);
                    gl.active_texture(Gl::TEXTURE0);
                    gl.bind_texture(Gl::TEXTURE_2D, Some(t));
                    gl.uniform1i(u.tex.as_ref(), 0);
                }
                None => gl.uniform1i(u.textured.as_ref(), 0),
            }
            gl.uniform1i(u.texgen_sphere.as_ref(), i32::from(batch.tex_gen_sphere));
            gl.uniform1i(
                u.tex_env.as_ref(),
                match batch.tex_env {
                    TexEnv::Modulate => 0,
                    TexEnv::Add => 1,
                    TexEnv::Replace => 2,
                },
            );
            gl.uniform1f(u.alpha_ref.as_ref(), batch.alpha_test.unwrap_or(0.0));
            gl.uniform1i(u.fog.as_ref(), i32::from(batch.fog.is_some()));
            match batch.fog {
                Some(Fog::Exp2 { density, color }) => {
                    gl.uniform1i(u.fog_mode.as_ref(), 0);
                    gl.uniform1f(u.fog_density.as_ref(), density);
                    gl.uniform4fv_with_f32_array(u.fog_color.as_ref(), &color);
                }
                Some(Fog::Exp { density, color }) => {
                    gl.uniform1i(u.fog_mode.as_ref(), 2);
                    gl.uniform1f(u.fog_density.as_ref(), density);
                    gl.uniform4fv_with_f32_array(u.fog_color.as_ref(), &color);
                }
                Some(Fog::Linear { start, end, color }) => {
                    gl.uniform1i(u.fog_mode.as_ref(), 1);
                    gl.uniform1f(u.fog_start.as_ref(), start);
                    gl.uniform1f(u.fog_end.as_ref(), end);
                    gl.uniform4fv_with_f32_array(u.fog_color.as_ref(), &color);
                }
                None => {}
            }
            gl.uniform4fv_with_f32_array(u.scene_ambient.as_ref(), &batch.scene_ambient);
            gl.uniform1i(u.color_material.as_ref(), i32::from(batch.color_material));
            gl.uniform1i(u.lighting.as_ref(), i32::from(batch.lighting));
            if batch.lighting {
                let m = &batch.material;
                for i in 0..MAX_LIGHTS {
                    let on = batch.light_enabled[i];
                    gl.uniform1i(u.light_on[i].as_ref(), i32::from(on));
                    if !on {
                        continue;
                    }
                    let l = &batch.lights[i];
                    gl.uniform4fv_with_f32_array(u.light_position[i].as_ref(), &l.position);
                    gl.uniform4fv_with_f32_array(u.light_ambient[i].as_ref(), &l.ambient);
                    gl.uniform4fv_with_f32_array(u.light_diffuse[i].as_ref(), &l.diffuse);
                    gl.uniform4fv_with_f32_array(u.light_specular[i].as_ref(), &l.specular);
                    gl.uniform3fv_with_f32_array(u.light_attenuation[i].as_ref(), &l.attenuation);
                }
                gl.uniform4fv_with_f32_array(u.material_diffuse.as_ref(), &m.ambient_diffuse);
                gl.uniform4fv_with_f32_array(
                    u.material_diffuse_back.as_ref(),
                    &m.back_ambient_diffuse,
                );
                gl.uniform4fv_with_f32_array(u.material_ambient.as_ref(), &m.ambient);
                gl.uniform4fv_with_f32_array(u.material_ambient_back.as_ref(), &m.back_ambient);
                gl.uniform4fv_with_f32_array(u.material_specular.as_ref(), &m.specular);
                gl.uniform4fv_with_f32_array(u.material_emission.as_ref(), &m.emission);
                gl.uniform1f(u.shininess.as_ref(), m.shininess);
            }
            gl.uniform1f(u.point_size.as_ref(), batch.point_size);
            // Line width above 1 is not portable in WebGL and is quietly
            // ignored by every browser that matters, so it is not set here
            // either; the savers that ask for a thick line get a thin one.
            if batch.count > 0 {
                gl.draw_arrays(
                    mode(batch.primitive),
                    batch.first as i32,
                    batch.count as i32,
                );
            }
            // And now, if this batch was the saver asking for a screenshot,
            // take one. What is read is the drawing buffer as it stands, which
            // is everything above this line; the browser does not throw that
            // away until the frame is composited.
            //
            // By blit rather than glCopyTexSubImage2D, which is what the C
            // calls: the canvas is antialiased, so the default framebuffer is
            // multisampled, and a copy out of a multisampled buffer is an
            // error. A blit to a single-sampled one is exactly the resolve
            // that rule exists to force, and same-size with NEAREST is the
            // form of it that a multisampled source allows.
            if let Some(id) = batch.copy_to_texture
                && let (Some((handle, _)), Some(t)) = (textures.get(&id), runner.texture(id))
            {
                let fbo = match copy_fbo {
                    Some(f) => Some(&*f),
                    None => {
                        *copy_fbo = gl.create_framebuffer();
                        copy_fbo.as_ref()
                    }
                };
                let (w, h) = (
                    batch.viewport[2].min(t.width),
                    batch.viewport[3].min(t.height),
                );
                gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, fbo);
                gl.framebuffer_texture_2d(
                    Gl::DRAW_FRAMEBUFFER,
                    Gl::COLOR_ATTACHMENT0,
                    Gl::TEXTURE_2D,
                    Some(handle),
                    0,
                );
                gl.blit_framebuffer(0, 0, w, h, 0, 0, w, h, Gl::COLOR_BUFFER_BIT, Gl::NEAREST);
                gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, None);
            }
        }
        gl.bind_vertex_array(None);
        gl.use_program(None);
    }
}

fn mode(p: Primitive) -> u32 {
    match p {
        Primitive::Points => Gl::POINTS,
        Primitive::Lines => Gl::LINES,
        Primitive::LineStrip => Gl::LINE_STRIP,
        Primitive::LineLoop => Gl::LINE_LOOP,
        Primitive::Triangles => Gl::TRIANGLES,
        Primitive::TriangleStrip => Gl::TRIANGLE_STRIP,
        Primitive::TriangleFan => Gl::TRIANGLE_FAN,
    }
}

fn link(gl: &Gl, vertex: &str, fragment: &str) -> Option<WebGlProgram> {
    let program = gl.create_program()?;
    for (kind, source) in [(Gl::VERTEX_SHADER, vertex), (Gl::FRAGMENT_SHADER, fragment)] {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(&shader, source);
        gl.compile_shader(&shader);
        if !gl
            .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
            .as_bool()
            .unwrap_or(false)
        {
            log::error!(
                "gl3d: {}",
                gl.get_shader_info_log(&shader).unwrap_or_default()
            );
            return None;
        }
        gl.attach_shader(&program, &shader);
        gl.delete_shader(Some(&shader));
    }
    gl.link_program(&program);
    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Some(program)
    } else {
        log::error!(
            "gl3d: {}",
            gl.get_program_info_log(&program).unwrap_or_default()
        );
        None
    }
}
