//! The 3D face of the compositor: the hidden 2D window canvases become
//! textures on quads along an arc, rendered with raw WebGL.
//!
//! The same scene serves the flat 3D preview (fixed camera, mouse picking)
//! and an immersive WebXR session; the session drives view and projection
//! matrices through dynamic JS calls so no unstable web-sys APIs are
//! needed, and everything else is shared.

use std::cell::Cell;
use std::collections::BTreeMap;

use dioxus::logger::tracing;
use dioxus::prelude::Coroutine;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlCanvasElement, WebGlProgram, WebGlRenderingContext as Gl, WebGlTexture,
    WebGlUniformLocation,
};
use webxr_compositor_protocol as protocol;

/// One textured rectangle in metres, upright, rotated `yaw` about Y.
pub struct Quad {
    pub id: protocol::WindowId,
    pub center: [f32; 3],
    pub yaw: f32,
    pub w: f32,
    pub h: f32,
}

/// 1000 surface pixels take one metre of arc.
const PX_PER_METRE: f32 = 1000.0;
const ARC_RADIUS: f32 = 2.2;
const ARC_STEP: f32 = 0.55;
const EYE_HEIGHT: f32 = 1.4;

pub struct Scene {
    pub(crate) gl: Gl,
    program: WebGlProgram,
    u_mvp: Option<WebGlUniformLocation>,
    u_size: Option<WebGlUniformLocation>,
    textures: BTreeMap<protocol::WindowId, WebGlTexture>,
    pub quads: Vec<Quad>,
    /// Preview camera orientation; the XR session replaces it wholesale.
    pub yaw: f32,
    pub pitch: f32,
}

const VERTEX: &str = r"
attribute vec2 corner;
uniform mat4 mvp;
uniform vec2 size;
varying vec2 uv;
void main() {
    uv = corner;
    vec3 local = vec3((corner.x - 0.5) * size.x, (0.5 - corner.y) * size.y, 0.0);
    gl_Position = mvp * vec4(local, 1.0);
}
";

const FRAGMENT: &str = r"
precision mediump float;
uniform sampler2D tex;
varying vec2 uv;
void main() {
    gl_FragColor = texture2D(tex, uv);
}
";

impl Scene {
    pub fn new(canvas: &HtmlCanvasElement) -> Option<Scene> {
        let options = js_sys::Object::new();
        // Kept readable so the browser tests can sample the rendered frame.
        let _ = js_sys::Reflect::set(&options, &"preserveDrawingBuffer".into(), &JsValue::TRUE);
        let gl: Gl = canvas
            .get_context_with_context_options("webgl", &options)
            .ok()??
            .dyn_into()
            .ok()?;

        let program = link(&gl)?;
        gl.use_program(Some(&program));
        let u_mvp = gl.get_uniform_location(&program, "mvp");
        let u_size = gl.get_uniform_location(&program, "size");

        let buffer = gl.create_buffer()?;
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&buffer));
        let corners: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        // SAFETY: the view is used before any further allocation can move
        // wasm memory.
        unsafe {
            let view = js_sys::Float32Array::view(&corners);
            gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STATIC_DRAW);
        }
        let corner = gl.get_attrib_location(&program, "corner");
        if corner < 0 {
            return None;
        }
        let corner = corner.unsigned_abs();
        gl.enable_vertex_attrib_array(corner);
        gl.vertex_attrib_pointer_with_i32(corner, 2, Gl::FLOAT, false, 0, 0);

        gl.enable(Gl::DEPTH_TEST);
        gl.clear_color(0.04, 0.04, 0.06, 1.0);

        Some(Scene {
            gl,
            program,
            u_mvp,
            u_size,
            textures: BTreeMap::new(),
            quads: Vec::new(),
            yaw: 0.0,
            pitch: 0.0,
        })
    }

    /// Windows along the arc, popups floating just in front of their parent.
    pub fn layout(
        &mut self,
        windows: &[(protocol::WindowId, u32, u32)],
        popups: &[(protocol::WindowId, protocol::WindowId, i32, i32, u32, u32)],
    ) {
        self.quads.clear();
        let count = windows.len();
        for (index, (id, px_w, px_h)) in windows.iter().enumerate() {
            let half = (f32::from(count as u16) - 1.0) / 2.0;
            let angle = (f32::from(index as u16) - half) * ARC_STEP;
            let center = [
                ARC_RADIUS * angle.sin(),
                EYE_HEIGHT,
                -ARC_RADIUS * angle.cos(),
            ];
            self.quads.push(Quad {
                id: *id,
                center,
                yaw: -angle,
                w: f32::from(*px_w as u16) / PX_PER_METRE,
                h: f32::from(*px_h as u16) / PX_PER_METRE,
            });
        }
        for (id, parent, x, y, px_w, px_h) in popups {
            let Some(parent) = self.quads.iter().find(|q| q.id == *parent) else {
                continue;
            };
            let w = f32::from(*px_w as u16) / PX_PER_METRE;
            let h = f32::from(*px_h as u16) / PX_PER_METRE;
            // Local offset from the parent's top-left corner, in metres.
            let lx = -parent.w / 2.0 + (f32::from(*x as i16) / PX_PER_METRE) + w / 2.0;
            let ly = parent.h / 2.0 - (f32::from(*y as i16) / PX_PER_METRE) - h / 2.0;
            let (sin, cos) = parent.yaw.sin_cos();
            let center = [
                parent.center[0] + lx * cos + 0.02 * sin,
                parent.center[1] + ly,
                parent.center[2] - lx * sin + 0.02 * cos,
            ];
            let yaw = parent.yaw;
            self.quads.push(Quad {
                id: *id,
                center,
                yaw,
                w,
                h,
            });
        }
    }

    /// Refresh each quad's texture from its window canvas in the hidden DOM.
    pub fn upload_textures(&mut self) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        for quad in &self.quads {
            let Some(canvas) = document
                .get_element_by_id(&format!("win-{}", quad.id))
                .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
            else {
                continue;
            };
            if canvas.width() == 0 {
                continue;
            }
            if !self.textures.contains_key(&quad.id) {
                let Some(texture) = self.gl.create_texture() else {
                    continue;
                };
                self.textures.insert(quad.id, texture);
            }
            let Some(texture) = self.textures.get(&quad.id) else {
                continue;
            };
            self.gl.bind_texture(Gl::TEXTURE_2D, Some(texture));
            let _ = self.gl.tex_image_2d_with_u32_and_u32_and_canvas(
                Gl::TEXTURE_2D,
                0,
                Gl::RGBA as i32,
                Gl::RGBA,
                Gl::UNSIGNED_BYTE,
                &canvas,
            );
            self.gl
                .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::LINEAR as i32);
            self.gl
                .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::LINEAR as i32);
            self.gl
                .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
            self.gl
                .tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
        }
    }

    /// Draw every quad with the given view-projection matrix.
    pub fn draw(&self, view_projection: &[f32; 16], viewport: Option<(i32, i32, i32, i32)>) {
        let gl = &self.gl;
        if let Some((x, y, w, h)) = viewport {
            gl.viewport(x, y, w, h);
        }
        gl.use_program(Some(&self.program));
        for quad in &self.quads {
            let Some(texture) = self.textures.get(&quad.id) else {
                continue;
            };
            gl.bind_texture(Gl::TEXTURE_2D, Some(texture));
            let model = model_matrix(quad);
            let mvp = multiply(view_projection, &model);
            gl.uniform_matrix4fv_with_f32_array(self.u_mvp.as_ref(), false, &mvp);
            gl.uniform2f(self.u_size.as_ref(), quad.w, quad.h);
            gl.draw_arrays(Gl::TRIANGLES, 0, 6);
        }
    }

    /// One preview frame: clear, camera from yaw/pitch, draw.
    pub fn render_preview(&mut self, width: u32, height: u32) {
        let gl = &self.gl;
        gl.viewport(0, 0, width as i32, height as i32);
        gl.clear(Gl::COLOR_BUFFER_BIT | Gl::DEPTH_BUFFER_BIT);
        let aspect = f32::from(width as u16) / f32::from(height.max(1) as u16);
        let projection = perspective(60.0_f32.to_radians(), aspect, 0.05, 50.0);
        let view = view_matrix(self.yaw, self.pitch);
        let view_projection = multiply(&projection, &view);
        self.upload_textures();
        self.draw(&view_projection, None);
    }

    /// The window id and surface pixel hit by a click at normalised device
    /// coordinates, from the preview camera.
    pub fn pick(
        &self,
        ndc_x: f32,
        ndc_y: f32,
        aspect: f32,
    ) -> Option<(protocol::WindowId, f64, f64)> {
        let f = 1.0 / (60.0_f32.to_radians() / 2.0).tan();
        // Camera-space ray through the pixel, then rotated by camera yaw and
        // pitch into world space.
        let (cx, cy, cz) = (ndc_x * aspect / f, ndc_y / f, -1.0);
        let (sp, cp) = self.pitch.sin_cos();
        let (ry, rz) = (cy * cp - cz * sp, cy * sp + cz * cp);
        let (sy, cyw) = self.yaw.sin_cos();
        let dir = [cx * cyw + rz * sy, ry, -cx * sy + rz * cyw];
        let origin = [0.0, EYE_HEIGHT, 0.0];
        self.pick_ray(origin, dir)
    }

    /// The nearest quad a world-space ray hits, as surface pixels.
    pub fn pick_ray(
        &self,
        origin: [f32; 3],
        dir: [f32; 3],
    ) -> Option<(protocol::WindowId, f64, f64)> {
        let mut best: Option<(f32, protocol::WindowId, f64, f64)> = None;
        for quad in &self.quads {
            let (sin, cos) = quad.yaw.sin_cos();
            // Into quad-local space: translate, then rotate by -yaw.
            let rel = [
                origin[0] - quad.center[0],
                origin[1] - quad.center[1],
                origin[2] - quad.center[2],
            ];
            let local_origin = [
                rel[0] * cos - rel[2] * sin,
                rel[1],
                rel[0] * sin + rel[2] * cos,
            ];
            let local_dir = [
                dir[0] * cos - dir[2] * sin,
                dir[1],
                dir[0] * sin + dir[2] * cos,
            ];
            if local_dir[2].abs() < 1e-6 {
                continue;
            }
            let t = -local_origin[2] / local_dir[2];
            if t <= 0.0 {
                continue;
            }
            let hx = local_origin[0] + t * local_dir[0];
            let hy = local_origin[1] + t * local_dir[1];
            if hx.abs() > quad.w / 2.0 || hy.abs() > quad.h / 2.0 {
                continue;
            }
            let u = (hx / quad.w + 0.5) as f64;
            let v = (0.5 - hy / quad.h) as f64;
            if best.is_none_or(|(bt, _, _, _)| t < bt) {
                best = Some((
                    t,
                    quad.id,
                    u * f64::from(quad.w * PX_PER_METRE),
                    v * f64::from(quad.h * PX_PER_METRE),
                ));
            }
        }
        best.map(|(_, id, x, y)| (id, x, y))
    }
}

fn link(gl: &Gl) -> Option<WebGlProgram> {
    let compile = |kind: u32, source: &str| {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(&shader, source);
        gl.compile_shader(&shader);
        if gl
            .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
            .as_bool()
            != Some(true)
        {
            tracing::warn!(
                "shader: {}",
                gl.get_shader_info_log(&shader).unwrap_or_default()
            );
            return None;
        }
        Some(shader)
    };
    let vertex = compile(Gl::VERTEX_SHADER, VERTEX)?;
    let fragment = compile(Gl::FRAGMENT_SHADER, FRAGMENT)?;
    let program = gl.create_program()?;
    gl.attach_shader(&program, &vertex);
    gl.attach_shader(&program, &fragment);
    gl.link_program(&program);
    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        != Some(true)
    {
        tracing::warn!(
            "link: {}",
            gl.get_program_info_log(&program).unwrap_or_default()
        );
        return None;
    }
    Some(program)
}

fn model_matrix(quad: &Quad) -> [f32; 16] {
    let (s, c) = quad.yaw.sin_cos();
    let [x, y, z] = quad.center;
    // Column-major: rotation about Y, then translation.
    [
        c, 0.0, -s, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        s, 0.0, c, 0.0, //
        x, y, z, 1.0,
    ]
}

fn view_matrix(yaw: f32, pitch: f32) -> [f32; 16] {
    // Inverse of the camera pose at (0, EYE_HEIGHT, 0) with yaw then pitch.
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let right = [cy, 0.0, -sy];
    let up = [sy * sp, cp, cy * sp];
    let back = [sy * cp, -sp, cy * cp];
    let eye = [0.0, EYE_HEIGHT, 0.0];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    [
        right[0],
        up[0],
        back[0],
        0.0, //
        right[1],
        up[1],
        back[1],
        0.0, //
        right[2],
        up[2],
        back[2],
        0.0, //
        -dot(right, eye),
        -dot(up, eye),
        -dot(back, eye),
        1.0,
    ]
}

fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> [f32; 16] {
    let f = 1.0 / (fov_y / 2.0).tan();
    let range = 1.0 / (near - far);
    [
        f / aspect,
        0.0,
        0.0,
        0.0, //
        0.0,
        f,
        0.0,
        0.0, //
        0.0,
        0.0,
        (near + far) * range,
        -1.0, //
        0.0,
        0.0,
        2.0 * near * far * range,
        0.0,
    ]
}

fn multiply(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[column * 4 + k];
            }
            out[column * 4 + row] = sum;
        }
    }
    out
}

thread_local! {
    /// The one live scene; tied to the mounted #xr-canvas.
    static SCENE: std::cell::RefCell<Option<Scene>> = const { std::cell::RefCell::new(None) };
    /// While an immersive session runs it owns the GL context; the preview
    /// loop must not draw over its frames.
    static XR_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// What the controller ray currently points at, so select events know
    /// their target without re-deriving the pose.
    static XR_HIT: Cell<Option<(protocol::WindowId, f64, f64)>> = const { Cell::new(None) };
}

/// Create the scene on this canvas unless one exists; true when usable.
pub fn init(canvas: &HtmlCanvasElement) -> bool {
    SCENE.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Scene::new(canvas);
        }
        cell.borrow().is_some()
    })
}

/// Leaving 3D unmounts the canvas, which kills the GL context with it.
pub fn drop_scene() {
    SCENE.with(|cell| *cell.borrow_mut() = None);
}

/// One preview frame over the current window and popup sets.
pub fn render_frame(
    width: u32,
    height: u32,
    windows: &[(protocol::WindowId, u32, u32)],
    popups: &[(protocol::WindowId, protocol::WindowId, i32, i32, u32, u32)],
) {
    SCENE.with(|cell| {
        if let Some(scene) = &mut *cell.borrow_mut() {
            scene.layout(windows, popups);
            if !XR_ACTIVE.get() {
                scene.render_preview(width, height);
            }
        }
    });
}

pub fn pick_at(ndc_x: f32, ndc_y: f32, aspect: f32) -> Option<(protocol::WindowId, f64, f64)> {
    SCENE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|scene| scene.pick(ndc_x, ndc_y, aspect))
    })
}

/// Mouse-drag look around, clamped so the floor stays down.
pub fn orbit(dx: f32, dy: f32) {
    SCENE.with(|cell| {
        if let Some(scene) = &mut *cell.borrow_mut() {
            scene.yaw += dx * 0.005;
            scene.pitch = (scene.pitch + dy * 0.005).clamp(-1.3, 1.3);
        }
    });
}

/// Whether this browser can offer an immersive session at all.
pub fn xr_available() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w.navigator(), &"xr".into()).ok())
        .is_some_and(|xr| !xr.is_undefined() && !xr.is_null())
}

/// Start an immersive-vr session driving the shared scene. Everything goes
/// through dynamic JS: web-sys keeps WebXR behind unstable cfg, and none of
/// this is reachable without a headset anyway. Failures log and give up.
pub fn enter_xr(session_handle: Coroutine<protocol::ClientToHost>) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = try_enter_xr(session_handle).await {
            XR_ACTIVE.set(false);
            tracing::warn!(?error, "entering XR failed");
        }
    });
}

async fn try_enter_xr(session_handle: Coroutine<protocol::ClientToHost>) -> Result<(), JsValue> {
    use js_sys::{Array, Function, Object, Reflect};
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or("no window")?;
    let xr = Reflect::get(&window.navigator(), &"xr".into())?;
    let request: Function = Reflect::get(&xr, &"requestSession".into())?.dyn_into()?;
    let session = JsFuture::from(js_sys::Promise::from(
        request.call1(&xr, &"immersive-vr".into())?,
    ))
    .await?;

    let gl = SCENE
        .with(|cell| cell.borrow().as_ref().map(|scene| scene.gl.clone()))
        .ok_or("no scene")?;
    // The context must be marked XR-compatible before a layer can wrap it.
    let make_compatible: Function = Reflect::get(&gl, &"makeXRCompatible".into())?.dyn_into()?;
    JsFuture::from(js_sys::Promise::from(make_compatible.call0(&gl)?)).await?;

    let layer_ctor: Function = Reflect::get(&window, &"XRWebGLLayer".into())?.dyn_into()?;
    let layer = Reflect::construct(&layer_ctor, &Array::of2(&session, &gl))?;
    let state = Object::new();
    Reflect::set(&state, &"baseLayer".into(), &layer)?;
    let update: Function = Reflect::get(&session, &"updateRenderState".into())?.dyn_into()?;
    update.call1(&session, &state)?;

    let request_space: Function =
        Reflect::get(&session, &"requestReferenceSpace".into())?.dyn_into()?;
    let space = match JsFuture::from(js_sys::Promise::from(
        request_space.call1(&session, &"local-floor".into())?,
    ))
    .await
    {
        Ok(space) => space,
        Err(_) => {
            JsFuture::from(js_sys::Promise::from(
                request_space.call1(&session, &"local".into())?,
            ))
            .await?
        }
    };

    XR_ACTIVE.set(true);
    install_session_listeners(&session, session_handle)?;
    xr_frame_loop(&session, layer, space, session_handle);
    Ok(())
}

/// End hands the context back to the preview; select is the controller
/// trigger, clicking whatever the ray last pointed at.
fn install_session_listeners(
    session: &JsValue,
    session_handle: Coroutine<protocol::ClientToHost>,
) -> Result<(), JsValue> {
    use js_sys::{Function, Reflect};
    use wasm_bindgen::closure::Closure;

    let add: Function = Reflect::get(session, &"addEventListener".into())?.dyn_into()?;

    let on_end = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
        XR_ACTIVE.set(false);
        XR_HIT.set(None);
    });
    add.call2(session, &"end".into(), on_end.as_ref().unchecked_ref())?;
    on_end.forget();

    let press = session_handle;
    let on_select_start = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
        if let Some((id, x, y)) = XR_HIT.get() {
            press.send(protocol::ClientToHost::Focus { id: Some(id) });
            press.send(protocol::ClientToHost::PointerMotion { id, x, y });
            press.send(protocol::ClientToHost::PointerButton {
                id,
                button: 0x110,
                pressed: true,
            });
        }
    });
    add.call2(
        session,
        &"selectstart".into(),
        on_select_start.as_ref().unchecked_ref(),
    )?;
    on_select_start.forget();

    let release = session_handle;
    let on_select_end = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
        if let Some((id, _, _)) = XR_HIT.get() {
            release.send(protocol::ClientToHost::PointerButton {
                id,
                button: 0x110,
                pressed: false,
            });
        }
    });
    add.call2(
        session,
        &"selectend".into(),
        on_select_end.as_ref().unchecked_ref(),
    )?;
    on_select_end.forget();

    Ok(())
}

/// Self-rescheduling XR frame callback: one draw per view, matrices straight
/// from the pose.
fn xr_frame_loop(
    session: &JsValue,
    layer: JsValue,
    space: JsValue,
    session_handle: Coroutine<protocol::ClientToHost>,
) {
    use js_sys::{Function, Reflect};
    use wasm_bindgen::closure::Closure;

    type FrameFn = Closure<dyn FnMut(f64, JsValue)>;
    let holder: std::rc::Rc<std::cell::RefCell<Option<FrameFn>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let again = std::rc::Rc::clone(&holder);
    let session_for_loop = session.clone();

    let callback = Closure::new(move |_time: f64, frame: JsValue| {
        let step = || -> Result<(), JsValue> {
            let get_pose: Function = Reflect::get(&frame, &"getViewerPose".into())?.dyn_into()?;
            let pose = get_pose.call1(&frame, &space)?;
            if pose.is_undefined() || pose.is_null() {
                return Ok(());
            }
            SCENE.with(|cell| -> Result<(), JsValue> {
                let mut guard = cell.borrow_mut();
                let Some(scene) = guard.as_mut() else {
                    return Ok(());
                };
                scene.upload_textures();
                // The session presents from the layer's framebuffer, not the
                // canvas default; a null framebuffer means it shares default.
                let framebuffer = Reflect::get(&layer, &"framebuffer".into())?;
                scene.gl.bind_framebuffer(
                    Gl::FRAMEBUFFER,
                    framebuffer.dyn_ref::<web_sys::WebGlFramebuffer>(),
                );
                scene.gl.clear(Gl::COLOR_BUFFER_BIT | Gl::DEPTH_BUFFER_BIT);

                let views: js_sys::Array = Reflect::get(&pose, &"views".into())?.dyn_into()?;
                let get_viewport: Function =
                    Reflect::get(&layer, &"getViewport".into())?.dyn_into()?;
                for view in views.iter() {
                    let viewport = get_viewport.call1(&layer, &view)?;
                    let number = |o: &JsValue, k: &str| -> f64 {
                        Reflect::get(o, &k.into())
                            .ok()
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                    };
                    let projection: js_sys::Float32Array =
                        Reflect::get(&view, &"projectionMatrix".into())?.dyn_into()?;
                    let transform = Reflect::get(&view, &"transform".into())?;
                    let inverse = Reflect::get(&transform, &"inverse".into())?;
                    let view_matrix: js_sys::Float32Array =
                        Reflect::get(&inverse, &"matrix".into())?.dyn_into()?;
                    let mut p = [0.0_f32; 16];
                    let mut v = [0.0_f32; 16];
                    projection.copy_to(&mut p);
                    view_matrix.copy_to(&mut v);
                    let view_projection = multiply(&p, &v);
                    scene.draw(
                        &view_projection,
                        Some((
                            number(&viewport, "x") as i32,
                            number(&viewport, "y") as i32,
                            number(&viewport, "width") as i32,
                            number(&viewport, "height") as i32,
                        )),
                    );
                }

                // The first controller with a pose steers the pointer; its
                // ray is the -Z axis of the target-ray transform.
                let previous = XR_HIT.get();
                XR_HIT.set(None);
                if let Ok(sources) = Reflect::get(&session_for_loop, &"inputSources".into()) {
                    let sources = js_sys::Array::from(&sources);
                    if let Some(source) = sources.iter().next() {
                        let ray_space = Reflect::get(&source, &"targetRaySpace".into())?;
                        let get_pose: Function =
                            Reflect::get(&frame, &"getPose".into())?.dyn_into()?;
                        let pose = get_pose.call2(&frame, &ray_space, &space)?;
                        if !pose.is_undefined() && !pose.is_null() {
                            let transform = Reflect::get(&pose, &"transform".into())?;
                            let matrix: js_sys::Float32Array =
                                Reflect::get(&transform, &"matrix".into())?.dyn_into()?;
                            let mut m = [0.0_f32; 16];
                            matrix.copy_to(&mut m);
                            let hit = scene.pick_ray([m[12], m[13], m[14]], [-m[8], -m[9], -m[10]]);
                            XR_HIT.set(hit);
                            if let Some((id, x, y)) = hit {
                                let moved = previous.is_none_or(|(pid, px, py)| {
                                    pid != id || (px - x).abs() >= 1.0 || (py - y).abs() >= 1.0
                                });
                                if moved {
                                    session_handle.send(protocol::ClientToHost::PointerMotion {
                                        id,
                                        x,
                                        y,
                                    });
                                }
                            }
                        }
                    }
                }
                Ok(())
            })
        };
        if let Err(error) = step() {
            tracing::warn!(?error, "XR frame failed");
            return;
        }
        if let Some(callback) = again.borrow().as_ref()
            && let Ok(raf) = Reflect::get(&session_for_loop, &"requestAnimationFrame".into())
            && let Ok(raf) = raf.dyn_into::<Function>()
        {
            let _ = raf.call1(&session_for_loop, callback.as_ref().unchecked_ref());
        }
    });

    if let Ok(raf) = Reflect::get(session, &"requestAnimationFrame".into())
        && let Ok(raf) = raf.dyn_into::<Function>()
    {
        let _ = raf.call1(session, callback.as_ref().unchecked_ref());
    }
    *holder.borrow_mut() = Some(callback);
}
