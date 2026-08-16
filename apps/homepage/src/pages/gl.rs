//! The WebGL2 half of the screensaver stage: the Shadertoy savers.
//!
//! [`xscreensaver::shadertoy`] works out *what* to draw and this draws it.
//! A Shadertoy program is a fragment shader run over two triangles that cover
//! the viewport, so there is no geometry, no depth buffer and no state to keep
//! beyond the textures each pass renders into.
//!
//! Those textures are why this is not a single draw call. A program may have up
//! to five passes; each renders into its own texture and can read any pass's
//! texture, including its own from the frame before, which is how the ones with
//! motion blur work. Each pass therefore has *two* textures and alternates
//! between them: it renders into the back one and then makes it the front, so a
//! pass reads the latest finished output of everything before it in the chain
//! and last frame's output of itself. Reading and writing one texture in a
//! single draw, which is what upstream does, is undefined in OpenGL and refused
//! outright by WebGL2.

use wasm_bindgen::JsCast;
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as Gl, WebGlBuffer, WebGlContextAttributes,
    WebGlFramebuffer, WebGlProgram, WebGlTexture, WebGlUniformLocation, WebGlVertexArrayObject,
};
use xscreensaver::SaverDef;
use xscreensaver::runtime::XEvent;
use xscreensaver::shadertoy::{self, MAX_CHANNELS, Shadertoy, Variant};

/// One pass: its compiled program, where its uniforms are, and the two
/// textures it alternates between.
struct Pass {
    program: WebGlProgram,
    position: u32,
    resolution: Option<WebGlUniformLocation>,
    time: Option<WebGlUniformLocation>,
    time_delta: Option<WebGlUniformLocation>,
    frame_rate: Option<WebGlUniformLocation>,
    frame: Option<WebGlUniformLocation>,
    mouse: Option<WebGlUniformLocation>,
    date: Option<WebGlUniformLocation>,
    channel: [Option<WebGlUniformLocation>; MAX_CHANNELS],
    texture: [WebGlTexture; 2],
    fbo: [WebGlFramebuffer; 2],
    /// Which of the two holds the finished output.
    front: usize,
}

pub struct GlEngine {
    gl: Gl,
    st: Shadertoy,
    vao: WebGlVertexArrayObject,
    vbo: WebGlBuffer,
    passes: Vec<Pass>,
    /// The size the textures were made at, which is the window times the
    /// resolution knob.
    texture_size: (i32, i32),
}

impl GlEngine {
    /// Take over a canvas. `None` if the browser will not give us WebGL2, which
    /// is the one thing this cannot work around.
    pub fn new(canvas: &HtmlCanvasElement, st: Shadertoy) -> Option<Self> {
        // Upstream's `*forceSingleSample: True`, and it is not cosmetic: the
        // last thing a frame does is blit the finished pass onto the canvas,
        // and blitting into a multisampled framebuffer is an error. A canvas
        // is antialiased by default, so asking for that off is what makes the
        // picture appear at all. Nothing here draws an edge, so there is
        // nothing for the samples to smooth anyway.
        let options = WebGlContextAttributes::new();
        options.set_antialias(false);
        options.set_depth(false);
        options.set_stencil(false);
        options.set_alpha(false);
        let gl: Gl = canvas
            .get_context_with_context_options("webgl2", &options)
            .ok()
            .flatten()?
            .dyn_into()
            .ok()?;

        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&vbo));
        // Two triangles covering clip space. Upstream's `verts`.
        let verts: [f32; 12] = [
            -1.0, -1.0, 1.0, -1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0, 1.0,
        ];
        unsafe {
            // SAFETY: the view borrows the wasm heap and is handed straight to
            // bufferData, which copies it before anything can grow the heap.
            let view = js_sys::Float32Array::view(&verts);
            gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
        }
        gl.bind_buffer(Gl::ARRAY_BUFFER, None);

        let mut engine = GlEngine {
            gl,
            st,
            vao,
            vbo,
            passes: Vec::new(),
            texture_size: (0, 0),
        };
        engine.compile();
        Some(engine)
    }

    pub fn def(&self) -> &'static SaverDef {
        self.st.def()
    }

    /// Run a different saver, or the same one with different knobs, on the
    /// context we already have.
    pub fn restart(&mut self, st: Shadertoy) {
        self.st = st;
        self.compile();
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        self.st.resize(width, height);
    }

    pub fn event(&mut self, event: &XEvent) -> bool {
        self.st.event(event)
    }

    /// Build the programs for the current variant, throwing away the previous
    /// ones. Called at startup and whenever the saver changes variant.
    fn compile(&mut self) {
        for pass in self.passes.drain(..) {
            self.gl.delete_program(Some(&pass.program));
            for i in 0..2 {
                self.gl.delete_texture(Some(&pass.texture[i]));
                self.gl.delete_framebuffer(Some(&pass.fbo[i]));
            }
        }
        self.texture_size = (0, 0);

        let variant: &'static Variant = match self.st.variants().get(self.st.variant()) {
            Some(v) => v,
            None => return,
        };
        let vertex = match self.shader(Gl::VERTEX_SHADER, &shadertoy::vertex_source()) {
            Some(s) => s,
            None => return,
        };

        for i in 0..variant.passes.len() {
            let source = shadertoy::fragment_source(variant, i);
            let Some(fragment) = self.shader(Gl::FRAGMENT_SHADER, &source) else {
                continue;
            };
            let Some(program) = self.gl.create_program() else {
                continue;
            };
            self.gl.attach_shader(&program, &vertex);
            self.gl.attach_shader(&program, &fragment);
            self.gl.link_program(&program);
            self.gl.delete_shader(Some(&fragment));
            if !self
                .gl
                .get_program_parameter(&program, Gl::LINK_STATUS)
                .as_bool()
                .unwrap_or(false)
            {
                let log = self.gl.get_program_info_log(&program).unwrap_or_default();
                log::error!("shadertoy: {} pass {i} did not link: {log}", self.slug());
                self.gl.delete_program(Some(&program));
                continue;
            }

            let position = self.gl.get_attrib_location(&program, "a_Position");
            if position < 0 {
                log::error!("shadertoy: {} pass {i} has no a_Position", self.slug());
                self.gl.delete_program(Some(&program));
                continue;
            }
            let uniform = |name: &str| self.gl.get_uniform_location(&program, name);
            let Some((texture, fbo)) = self.make_targets() else {
                self.gl.delete_program(Some(&program));
                continue;
            };
            self.passes.push(Pass {
                position: position as u32,
                resolution: uniform("iResolution"),
                time: uniform("iTime"),
                time_delta: uniform("iTimeDelta"),
                frame_rate: uniform("iFrameRate"),
                frame: uniform("iFrame"),
                mouse: uniform("iMouse"),
                date: uniform("iDate"),
                channel: [
                    uniform("iChannel0"),
                    uniform("iChannel1"),
                    uniform("iChannel2"),
                    uniform("iChannel3"),
                ],
                program,
                texture,
                fbo,
                front: 0,
            });
        }
        self.gl.delete_shader(Some(&vertex));
    }

    fn slug(&self) -> &'static str {
        self.st.def().slug
    }

    fn shader(&self, kind: u32, source: &str) -> Option<web_sys::WebGlShader> {
        let shader = self.gl.create_shader(kind)?;
        self.gl.shader_source(&shader, source);
        self.gl.compile_shader(&shader);
        if self
            .gl
            .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
            .as_bool()
            .unwrap_or(false)
        {
            return Some(shader);
        }
        let log = self.gl.get_shader_info_log(&shader).unwrap_or_default();
        log::error!("shadertoy: {} did not compile: {log}", self.slug());
        self.gl.delete_shader(Some(&shader));
        None
    }

    /// Two textures and a framebuffer for each, for one pass.
    ///
    /// Eight bits a channel, which is upstream's format where the driver does
    /// not offer half floats. It costs some banding in the passes that
    /// accumulate over many frames, and it is available everywhere.
    fn make_targets(&self) -> Option<([WebGlTexture; 2], [WebGlFramebuffer; 2])> {
        let mut textures = Vec::with_capacity(2);
        let mut fbos = Vec::with_capacity(2);
        for _ in 0..2 {
            let tex = self.gl.create_texture()?;
            // A texture from `createTexture` has no target until it has been
            // bound once, and attaching one that has never been bound fails
            // with "no texture is bound to the specified target". Giving it a
            // pixel here is what makes it a 2D texture; the real size arrives
            // in `size_targets`.
            self.allocate(&tex, 1, 1);
            let fbo = self.gl.create_framebuffer()?;
            self.gl.bind_framebuffer(Gl::FRAMEBUFFER, Some(&fbo));
            self.gl.framebuffer_texture_2d(
                Gl::FRAMEBUFFER,
                Gl::COLOR_ATTACHMENT0,
                Gl::TEXTURE_2D,
                Some(&tex),
                0,
            );
            textures.push(tex);
            fbos.push(fbo);
        }
        self.gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
        self.gl.bind_texture(Gl::TEXTURE_2D, None);
        let textures: [WebGlTexture; 2] = textures.try_into().ok()?;
        let fbos: [WebGlFramebuffer; 2] = fbos.try_into().ok()?;
        Some((textures, fbos))
    }

    /// Give one texture a size and the sampling upstream asks for: clamped, and
    /// nearest in both directions.
    ///
    /// Eight bits a channel, which is upstream's format where the driver does
    /// not offer half floats. It costs some banding in the passes that
    /// accumulate over many frames, and it is available everywhere.
    fn allocate(&self, tex: &WebGlTexture, width: i32, height: i32) {
        let gl = &self.gl;
        gl.bind_texture(Gl::TEXTURE_2D, Some(tex));
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::NEAREST as i32);
        gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::NEAREST as i32);
        let _ = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            Gl::TEXTURE_2D,
            0,
            Gl::RGBA8 as i32,
            width,
            height,
            0,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            None,
        );
    }

    /// Give every texture the current size, clearing them in the process.
    /// Upstream reallocates them on every reshape too.
    fn size_targets(&mut self, width: i32, height: i32) {
        for pass in &self.passes {
            for tex in &pass.texture {
                self.allocate(tex, width, height);
            }
        }
        self.gl.bind_texture(Gl::TEXTURE_2D, None);
        self.texture_size = (width, height);
    }

    /// Draw one frame, if the saver's requested delay says one is due.
    pub fn draw(&mut self, now: f64) {
        let Some(frame) = self.st.tick(now) else {
            return;
        };
        if self.st.take_reload() {
            self.compile();
        }
        if self.passes.is_empty() {
            return;
        }
        let (bw, bh) = self.st.buffer_size();
        if (bw, bh) != self.texture_size {
            self.size_targets(bw, bh);
        }

        let gl = self.gl.clone();
        gl.viewport(0, 0, bw, bh);
        gl.bind_vertex_array(Some(&self.vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.vbo));

        for i in 0..self.passes.len() {
            let back = 1 - self.passes[i].front;
            gl.use_program(Some(&self.passes[i].program));
            gl.bind_framebuffer(Gl::FRAMEBUFFER, Some(&self.passes[i].fbo[back]));

            // Every pass's latest output is readable by every pass, which is
            // what makes a chain a chain. A pass's own entry is still last
            // frame's, because it has not swapped yet.
            for j in 0..MAX_CHANNELS.min(self.passes.len()) {
                let Some(location) = self.passes[i].channel[j].clone() else {
                    continue;
                };
                gl.active_texture(Gl::TEXTURE0 + j as u32);
                let front = self.passes[j].front;
                gl.bind_texture(Gl::TEXTURE_2D, Some(&self.passes[j].texture[front]));
                gl.uniform1i(Some(&location), j as i32);
            }

            let pass = &self.passes[i];
            gl.uniform3f(
                pass.resolution.as_ref(),
                frame.resolution[0],
                frame.resolution[1],
                frame.resolution[2],
            );
            gl.uniform1f(pass.time.as_ref(), frame.time);
            gl.uniform1f(pass.time_delta.as_ref(), frame.time_delta);
            gl.uniform1f(pass.frame_rate.as_ref(), frame.frame_rate);
            gl.uniform1i(pass.frame.as_ref(), frame.frame);
            gl.uniform4f(
                pass.mouse.as_ref(),
                frame.mouse[0],
                frame.mouse[1],
                frame.mouse[2],
                frame.mouse[3],
            );
            gl.uniform4f(
                pass.date.as_ref(),
                frame.date[0],
                frame.date[1],
                frame.date[2],
                frame.date[3],
            );

            gl.vertex_attrib_pointer_with_i32(pass.position, 2, Gl::FLOAT, false, 0, 0);
            gl.enable_vertex_attrib_array(pass.position);
            gl.draw_arrays(Gl::TRIANGLES, 0, 6);

            self.passes[i].front = back;
        }

        gl.bind_buffer(Gl::ARRAY_BUFFER, None);
        gl.bind_vertex_array(None);
        gl.use_program(None);

        // The last pass is the picture. Scale it up to the canvas, which is the
        // only place the resolution knob shows.
        let last = &self.passes[self.passes.len() - 1];
        let (cw, ch) = (self.canvas_width(), self.canvas_height());
        gl.bind_framebuffer(Gl::READ_FRAMEBUFFER, Some(&last.fbo[last.front]));
        gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, None);
        gl.blit_framebuffer(0, 0, bw, bh, 0, 0, cw, ch, Gl::COLOR_BUFFER_BIT, Gl::LINEAR);
        gl.bind_framebuffer(Gl::FRAMEBUFFER, None);
    }

    fn canvas_width(&self) -> i32 {
        self.gl.drawing_buffer_width()
    }

    fn canvas_height(&self) -> i32 {
        self.gl.drawing_buffer_height()
    }
}
