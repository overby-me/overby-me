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

use wasm_bindgen::JsCast;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlContextAttributes,
    WebGlProgram, WebGlUniformLocation, WebGlVertexArrayObject,
};
use xscreensaver::SaverDef;
use xscreensaver::runtime::Runner3d;
use xscreensaver::runtime::XEvent;
use xscreensaver::runtime::gl::{Frame, Primitive};

/// Position, colour and normal, in the order [`Frame`] holds them.
const FLOATS_PER_VERTEX: usize = 10;

const VERTEX_SHADER: &str = r"#version 300 es
in vec3 a_pos;
in vec4 a_color;
in vec3 a_normal;
uniform mat4 u_mvp;
uniform mat4 u_modelview;
uniform float u_point_size;
out vec4 v_color;
out vec3 v_normal;
out vec3 v_eye;
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
}
";

/// The fixed-function lighting equation, for one light and no attenuation,
/// which is all any of these savers asks for. Shading is per fragment rather
/// than per vertex as OpenGL 1.3 did it; on the low-polygon shapes these draw
/// that is a visible improvement and never a difference in what is depicted.
const FRAGMENT_SHADER: &str = r"#version 300 es
precision highp float;
in vec4 v_color;
in vec3 v_normal;
in vec3 v_eye;
uniform bool u_lighting;
uniform vec4 u_light_position;
uniform vec4 u_light_ambient;
uniform vec4 u_light_diffuse;
uniform vec4 u_light_specular;
uniform vec4 u_material_diffuse;
uniform vec4 u_material_specular;
uniform float u_shininess;
uniform vec4 u_scene_ambient;
out vec4 frag_color;
void main() {
  if (! u_lighting) {
    frag_color = v_color;
    return;
  }
  vec3 n = normalize (v_normal);
  // A w of zero is a light infinitely far away, so its position is a
  // direction; otherwise it is a place, and the direction is from here to it.
  vec3 l = normalize (u_light_position.w == 0.0
                      ? u_light_position.xyz
                      : u_light_position.xyz - v_eye);
  // Two-sided: a back face is lit by its own side. OpenGL would cull or shade
  // it separately, and the savers that leave culling off want to see both.
  if (dot (n, normalize (-v_eye)) < 0.0) n = -n;

  vec3 c = u_material_diffuse.rgb * (u_scene_ambient.rgb + u_light_ambient.rgb);
  c += u_material_diffuse.rgb * u_light_diffuse.rgb * max (dot (n, l), 0.0);
  if (u_shininess > 0.0) {
    vec3 h = normalize (l + normalize (-v_eye));
    c += u_material_specular.rgb * u_light_specular.rgb
       * pow (max (dot (n, h), 0.0), u_shininess);
  }
  frag_color = vec4 (c, u_material_diffuse.a);
}
";

/// Where every uniform of the one program lives, looked up once.
struct Uniforms {
    mvp: Option<WebGlUniformLocation>,
    modelview: Option<WebGlUniformLocation>,
    point_size: Option<WebGlUniformLocation>,
    lighting: Option<WebGlUniformLocation>,
    light_position: Option<WebGlUniformLocation>,
    light_ambient: Option<WebGlUniformLocation>,
    light_diffuse: Option<WebGlUniformLocation>,
    light_specular: Option<WebGlUniformLocation>,
    material_diffuse: Option<WebGlUniformLocation>,
    material_specular: Option<WebGlUniformLocation>,
    shininess: Option<WebGlUniformLocation>,
    scene_ambient: Option<WebGlUniformLocation>,
}

impl Uniforms {
    fn of(gl: &Gl, program: &WebGlProgram) -> Self {
        let at = |name: &str| gl.get_uniform_location(program, name);
        Uniforms {
            mvp: at("u_mvp"),
            modelview: at("u_modelview"),
            point_size: at("u_point_size"),
            lighting: at("u_lighting"),
            light_position: at("u_light_position"),
            light_ambient: at("u_light_ambient"),
            light_diffuse: at("u_light_diffuse"),
            light_specular: at("u_light_specular"),
            material_diffuse: at("u_material_diffuse"),
            material_specular: at("u_material_specular"),
            shininess: at("u_shininess"),
            scene_ambient: at("u_scene_ambient"),
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
}

impl Gl3dEngine {
    pub fn new(canvas: &HtmlCanvasElement, runner: Runner3d) -> Option<Self> {
        // Depth is on: these are 3D. Antialiasing is left at the browser's
        // default, unlike the Shadertoy engine, because nothing here blits.
        let options = WebGlContextAttributes::new();
        options.set_depth(true);
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
        if position < 0 || color < 0 || normal < 0 {
            log::error!("gl3d: the shader is missing an attribute");
            return None;
        }

        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        let stride = (FLOATS_PER_VERTEX * 4) as i32;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&vbo));
        for (location, size, offset) in [(position, 3, 0), (color, 4, 12), (normal, 3, 28)] {
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
        })
    }

    pub fn def(&self) -> &'static SaverDef {
        self.runner.def()
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
            runner, scratch, ..
        } = self;
        let frame: &Frame = runner.frame();

        let [vx, vy, vw, vh] = frame.viewport;
        gl.viewport(vx, vy, vw.max(1), vh.max(1));

        if let Some([r, g, b, a]) = frame.clear {
            gl.clear_color(r, g, b, a);
            gl.clear(Gl::COLOR_BUFFER_BIT | Gl::DEPTH_BUFFER_BIT);
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

        gl.use_program(Some(&self.program));
        let u = &self.uniforms;
        // OpenGL's own default scene ambient, which no saver here changes.
        gl.uniform4f(u.scene_ambient.as_ref(), 0.2, 0.2, 0.2, 1.0);
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
            gl.uniform_matrix4fv_with_f32_array(u.mvp.as_ref(), false, &batch.mvp.0);
            gl.uniform_matrix4fv_with_f32_array(u.modelview.as_ref(), false, &batch.modelview.0);
            gl.uniform1i(u.lighting.as_ref(), i32::from(batch.lighting));
            if batch.lighting {
                let (l, m) = (&batch.light, &batch.material);
                gl.uniform4fv_with_f32_array(u.light_position.as_ref(), &l.position);
                gl.uniform4fv_with_f32_array(u.light_ambient.as_ref(), &l.ambient);
                gl.uniform4fv_with_f32_array(u.light_diffuse.as_ref(), &l.diffuse);
                gl.uniform4fv_with_f32_array(u.light_specular.as_ref(), &l.specular);
                gl.uniform4fv_with_f32_array(u.material_diffuse.as_ref(), &m.ambient_diffuse);
                gl.uniform4fv_with_f32_array(u.material_specular.as_ref(), &m.specular);
                gl.uniform1f(u.shininess.as_ref(), m.shininess);
            }
            gl.uniform1f(u.point_size.as_ref(), batch.point_size);
            // Line width above 1 is not portable in WebGL and is quietly
            // ignored by every browser that matters, so it is not set here
            // either; the savers that ask for a thick line get a thin one.
            gl.draw_arrays(
                mode(batch.primitive),
                batch.first as i32,
                batch.count as i32,
            );
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
