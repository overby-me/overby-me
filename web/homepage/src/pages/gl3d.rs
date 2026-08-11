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
uniform mat4 u_mvp;
uniform float u_point_size;
out vec4 v_color;
void main() {
  gl_Position = u_mvp * vec4 (a_pos, 1.0);
  gl_PointSize = u_point_size;
  v_color = a_color;
}
";

const FRAGMENT_SHADER: &str = r"#version 300 es
precision highp float;
in vec4 v_color;
out vec4 frag_color;
void main() {
  frag_color = v_color;
}
";

pub struct Gl3dEngine {
    gl: Gl,
    runner: Runner3d,
    program: WebGlProgram,
    mvp: Option<WebGlUniformLocation>,
    point_size: Option<WebGlUniformLocation>,
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
        if position < 0 || color < 0 {
            log::error!("gl3d: the shader is missing an attribute");
            return None;
        }

        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        let stride = (FLOATS_PER_VERTEX * 4) as i32;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&vbo));
        gl.vertex_attrib_pointer_with_i32(position as u32, 3, Gl::FLOAT, false, stride, 0);
        gl.enable_vertex_attrib_array(position as u32);
        gl.vertex_attrib_pointer_with_i32(color as u32, 4, Gl::FLOAT, false, stride, 12);
        gl.enable_vertex_attrib_array(color as u32);
        gl.bind_vertex_array(None);

        Some(Gl3dEngine {
            mvp: gl.get_uniform_location(&program, "u_mvp"),
            point_size: gl.get_uniform_location(&program, "u_point_size"),
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
        for batch in &frame.batches {
            if batch.depth_test {
                gl.enable(Gl::DEPTH_TEST);
            } else {
                gl.disable(Gl::DEPTH_TEST);
            }
            gl.uniform_matrix4fv_with_f32_array(self.mvp.as_ref(), false, &batch.mvp.0);
            gl.uniform1f(self.point_size.as_ref(), batch.point_size);
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
