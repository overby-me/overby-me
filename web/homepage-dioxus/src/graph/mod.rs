pub mod camera;
pub mod data;
pub mod interaction;
pub mod particles;
pub mod renderer;
pub mod simulation;
pub mod texture;

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use dioxus::web::WebEventExt;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::WebGlRenderingContext as GL;

use camera::Camera;
use data::NODES;
use particles::ParticleSystem;
use renderer::Renderer;
use simulation::Simulation;
use texture::TextureManager;

fn mouse_client_xy(evt: &MouseEvent) -> (f32, f32) {
    let coords = evt.data().client_coordinates();
    (coords.x as f32, coords.y as f32)
}

fn wheel_delta_y(evt: &WheelEvent) -> f32 {
    use dioxus_elements::geometry::WheelDelta;
    match evt.data().delta() {
        WheelDelta::Pixels(v) => v.y as f32,
        WheelDelta::Lines(v) => v.y as f32 * 40.0,
        WheelDelta::Pages(v) => v.y as f32 * 800.0,
    }
}

#[component]
pub fn Graph() -> Element {
    let mut tooltip_text = use_signal(|| Option::<String>::None);
    let mut tooltip_x = use_signal(|| 0.0f32);
    let mut tooltip_y = use_signal(|| 0.0f32);
    let mut cursor_pointer = use_signal(|| false);

    // Shared state for the render loop
    let mut state: Signal<Option<Rc<RefCell<GraphState>>>> = use_signal(|| None);

    // Initialize WebGL on canvas mount
    let onmounted = move |evt: MountedEvent| {
        spawn(async move {
            let elem: web_sys::Element = evt.data().try_as_web_event().unwrap();
            let canvas: web_sys::HtmlCanvasElement = elem.dyn_into().unwrap();

            // Set canvas size to match display size
            let dpr = web_sys::window()
                .map(|w| w.device_pixel_ratio())
                .unwrap_or(1.0);
            let display_width = canvas.client_width() as f64 * dpr;
            let display_height = canvas.client_height() as f64 * dpr;
            canvas.set_width(display_width as u32);
            canvas.set_height(display_height as u32);

            let gl: GL = canvas
                .get_context("webgl")
                .unwrap()
                .unwrap()
                .dyn_into()
                .unwrap();

            let renderer = match Renderer::new(gl.clone()) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Failed to create renderer: {e}");
                    return;
                }
            };

            renderer.resize(display_width as u32, display_height as u32);

            let mut camera = Camera::new();
            camera.set_aspect(display_width as f32 / display_height as f32);

            let simulation = Simulation::new();
            let particle_system = ParticleSystem::new();

            let tex_manager = TextureManager::new(gl);
            for node in NODES {
                tex_manager.load_icon(node.icon);
            }

            let gs = Rc::new(RefCell::new(GraphState {
                renderer,
                camera,
                simulation,
                particle_system,
                textures: tex_manager.textures,
                canvas_width: display_width as f32,
                canvas_height: display_height as f32,
                dragging_node: None,
                drag_plane_point: glam::Vec3::ZERO,
                drag_moved: false,
                press_x: 0.0,
                press_y: 0.0,
            }));

            state.set(Some(Rc::clone(&gs)));

            // Start animation loop
            start_animation_loop(gs);
        });
    };

    let onmousedown = move |evt: MouseEvent| {
        let (x, y) = mouse_client_xy(&evt);
        if let Some(ref gs) = *state.read() {
            let mut gs = gs.borrow_mut();

            // Grab a node if one is under the cursor; otherwise orbit the camera.
            let view = gs.camera.view_matrix();
            let proj = gs.camera.projection_matrix();
            let hit = interaction::pick_node(
                x,
                y,
                gs.canvas_width,
                gs.canvas_height,
                &view,
                &proj,
                &gs.simulation,
            );

            if let Some(hit) = hit {
                let idx = hit.node_index;
                let pos = gs.simulation.positions[idx];
                gs.dragging_node = Some(idx);
                gs.drag_plane_point = pos;
                gs.drag_moved = false;
                gs.press_x = x;
                gs.press_y = y;
                gs.simulation.pin(idx, pos);
                // Reheat so the rest of the graph reacts while dragging.
                gs.simulation.set_alpha_target(0.3);
            } else {
                gs.camera.on_mouse_down(x, y);
            }
        }
    };

    let onmousemove = {
        move |evt: MouseEvent| {
            let (x, y) = mouse_client_xy(&evt);
            if let Some(ref gs) = *state.read() {
                let mut gs_mut = gs.borrow_mut();

                // If a node is grabbed, drag it in the plane facing the camera
                // that passes through its grab position (constant depth).
                if let Some(idx) = gs_mut.dragging_node {
                    if (x - gs_mut.press_x).hypot(y - gs_mut.press_y) > 4.0 {
                        gs_mut.drag_moved = true;
                    }
                    let view = gs_mut.camera.view_matrix();
                    let proj = gs_mut.camera.projection_matrix();
                    let (origin, dir) = interaction::screen_ray(
                        x,
                        y,
                        gs_mut.canvas_width,
                        gs_mut.canvas_height,
                        &view,
                        &proj,
                    );
                    // Plane normal points from the scene toward the camera eye.
                    let normal = gs_mut.camera.eye_position().normalize();
                    let plane_point = gs_mut.drag_plane_point;
                    if let Some(p) = interaction::intersect_plane(origin, dir, plane_point, normal)
                    {
                        gs_mut.simulation.pin(idx, p);
                    }
                    tooltip_text.set(Some(NODES[idx].desc.to_string()));
                    let (sx, sy) = interaction::project_to_screen(
                        gs_mut.simulation.positions[idx],
                        &view,
                        &proj,
                        gs_mut.canvas_width,
                        gs_mut.canvas_height,
                    );
                    tooltip_x.set(sx);
                    tooltip_y.set(sy);
                    cursor_pointer.set(true);
                    return;
                }

                gs_mut.camera.on_mouse_move(x, y);

                // Check hover
                let view = gs_mut.camera.view_matrix();
                let proj = gs_mut.camera.projection_matrix();
                let hit = interaction::pick_node(
                    x,
                    y,
                    gs_mut.canvas_width,
                    gs_mut.canvas_height,
                    &view,
                    &proj,
                    &gs_mut.simulation,
                );

                if let Some(hit) = hit {
                    let node = &NODES[hit.node_index];
                    let pos = gs_mut.simulation.positions[hit.node_index];
                    let (sx, sy) = interaction::project_to_screen(
                        pos,
                        &view,
                        &proj,
                        gs_mut.canvas_width,
                        gs_mut.canvas_height,
                    );
                    tooltip_text.set(Some(node.desc.to_string()));
                    tooltip_x.set(sx);
                    tooltip_y.set(sy);
                    cursor_pointer.set(node.url.is_some());
                } else {
                    tooltip_text.set(None);
                    cursor_pointer.set(false);
                }
            }
        }
    };

    let onmouseup = move |_evt: MouseEvent| {
        if let Some(ref gs) = *state.read() {
            let mut gs = gs.borrow_mut();
            if let Some(idx) = gs.dragging_node.take() {
                // Release the node and let the reheated layout absorb it.
                gs.simulation.unpin(idx);
                gs.simulation.set_alpha_target(0.0);
                // A grab that never moved is a click: follow the node's link.
                if !gs.drag_moved
                    && let Some(url) = NODES[idx].url
                    && let Some(window) = web_sys::window()
                {
                    let _ = window.location().set_href(url);
                }
            } else {
                gs.camera.on_mouse_up();
            }
        }
    };

    let onwheel = move |evt: WheelEvent| {
        let dy = wheel_delta_y(&evt);
        if let Some(ref gs) = *state.read() {
            gs.borrow_mut().camera.on_wheel(dy);
        }
    };

    let cursor = if *cursor_pointer.read() {
        "pointer"
    } else {
        "default"
    };

    rsx! {
        div {
            style: "width: 100vw; height: 100vh; position: relative; overflow: hidden; background: #222222;",
            canvas {
                id: "graph-canvas",
                style: "width: 100%; height: 100%; display: block; cursor: {cursor};",
                onmounted: onmounted,
                onmousedown: onmousedown,
                onmousemove: onmousemove,
                onmouseup: onmouseup,
                onwheel: onwheel,
            }
            // Tooltip overlay
            if let Some(text) = &*tooltip_text.read() {
                div {
                    style: "position: absolute; pointer-events: none; white-space: pre; color: #ffffff; font-size: 30px; text-shadow: 0 0 5px #000000, 2px 2px 18px #ff0072; text-align: center; transform: translate(-50%, -100%); left: {tooltip_x}px; top: {tooltip_y}px; font-weight: bold;",
                    "{text}"
                }
            }
        }
    }
}

struct GraphState {
    renderer: Renderer,
    camera: Camera,
    simulation: Simulation,
    particle_system: ParticleSystem,
    textures: Rc<RefCell<std::collections::HashMap<String, web_sys::WebGlTexture>>>,
    canvas_width: f32,
    canvas_height: f32,
    // Node-drag state: which node is grabbed, the screen-parallel plane it moves
    // in, whether the pointer actually moved (drag vs click), and the press
    // position used to tell those apart.
    dragging_node: Option<usize>,
    drag_plane_point: glam::Vec3,
    drag_moved: bool,
    press_x: f32,
    press_y: f32,
}

type AnimationClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

fn start_animation_loop(state: Rc<RefCell<GraphState>>) {
    let f: AnimationClosure = Rc::new(RefCell::new(None));
    let g = Rc::clone(&f);

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        {
            let mut gs = state.borrow_mut();
            gs.simulation.tick();
            gs.particle_system.tick();

            let particle_pos = gs.particle_system.positions(&gs.simulation);
            let textures = gs.textures.borrow();

            gs.renderer
                .render(&gs.camera, &gs.simulation, &textures, &particle_pos);
        }

        // Request next frame
        if let Some(window) = web_sys::window() {
            let _ = window
                .request_animation_frame(f.borrow().as_ref().unwrap().as_ref().unchecked_ref());
        }
    }) as Box<dyn FnMut()>));

    // Kick off the first frame
    if let Some(window) = web_sys::window() {
        let _ =
            window.request_animation_frame(g.borrow().as_ref().unwrap().as_ref().unchecked_ref());
    }
}
