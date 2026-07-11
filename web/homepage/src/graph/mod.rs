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
use data::GraphData;
use particles::ParticleSystem;
use renderer::Renderer;
use simulation::Simulation;
use texture::TextureManager;

fn pointer_client_xy(evt: &PointerEvent) -> (f32, f32) {
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
pub fn Graph(data: GraphData) -> Element {
    let mut tooltip_text = use_signal(|| Option::<String>::None);
    let mut tooltip_x = use_signal(|| 0.0f32);
    let mut tooltip_y = use_signal(|| 0.0f32);
    let mut cursor_pointer = use_signal(|| false);

    // The graph topology, stored once per mount. Rc so the many event closures
    // and the render loop share it cheaply. Callers `key` the component by
    // handle so a new graph remounts and re-reads the prop here.
    let data: Rc<GraphData> = use_hook(|| Rc::new(data));

    // Shared state for the render loop
    let mut state: Signal<Option<Rc<RefCell<GraphState>>>> = use_signal(|| None);

    // Initialize WebGL on canvas mount
    let onmounted = move |evt: MountedEvent| {
        let data = data.clone();
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
            let aspect = display_width as f32 / display_height as f32;
            camera.set_aspect(aspect);

            // A collapsible graph starts collapsed: only the center + hubs show.
            let expanded = std::collections::HashSet::new();
            let visible = filtered_graph(&data, &expanded);
            let distance = fit_camera_distance(&visible, aspect);
            camera.distance = distance;

            let simulation = Simulation::new(&visible);
            let particle_system = ParticleSystem::new(&visible);

            // Load every icon up front so expanding a hub is instant.
            let tex_manager = TextureManager::new(gl);
            for node in &data.nodes {
                tex_manager.load_icon(&node.icon);
            }

            let gs = Rc::new(RefCell::new(GraphState {
                renderer,
                camera,
                simulation,
                particle_system,
                textures: tex_manager.textures,
                data: visible,
                full: data,
                expanded,
                target_distance: distance,
                aspect,
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

    let onpointerdown = move |evt: PointerEvent| {
        let (x, y) = pointer_client_xy(&evt);
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
                &gs.data,
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

    let onpointermove = {
        move |evt: PointerEvent| {
            let (x, y) = pointer_client_xy(&evt);
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
                    tooltip_text.set(Some(gs_mut.data.nodes[idx].desc.clone()));
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
                    &gs_mut.data,
                );

                if let Some(hit) = hit {
                    let node = &gs_mut.data.nodes[hit.node_index];
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
                    cursor_pointer.set(node.url.is_some() || node.hub);
                } else {
                    tooltip_text.set(None);
                    cursor_pointer.set(false);
                }
            }
        }
    };

    let onpointerup = move |_evt: PointerEvent| {
        if let Some(ref gs) = *state.read() {
            let mut gs = gs.borrow_mut();
            if let Some(idx) = gs.dragging_node.take() {
                // Release the node and let the reheated layout absorb it.
                gs.simulation.unpin(idx);
                gs.simulation.set_alpha_target(0.0);
                // A grab that never moved is a click/tap.
                if !gs.drag_moved {
                    let node = &gs.data.nodes[idx];
                    let is_hub = node.hub;
                    let id = node.id.clone();
                    let url = node.url.clone();
                    if is_hub {
                        // Toggle this category hub's leaves.
                        if !gs.expanded.remove(&id) {
                            gs.expanded.insert(id);
                        }
                        rebuild(&mut gs);
                    } else if let Some(url) = url
                        && let Some(window) = web_sys::window()
                    {
                        let _ = window.location().set_href(&url);
                    }
                }
            } else {
                gs.camera.on_mouse_up();
            }
        }
    };

    // A cancelled pointer (e.g. the browser reclaims a touch gesture) releases
    // whatever was grabbed without treating it as a click.
    let onpointercancel = move |_evt: PointerEvent| {
        if let Some(ref gs) = *state.read() {
            let mut gs = gs.borrow_mut();
            if let Some(idx) = gs.dragging_node.take() {
                gs.simulation.unpin(idx);
                gs.simulation.set_alpha_target(0.0);
            } else {
                gs.camera.on_mouse_up();
            }
        }
    };

    let onwheel = move |evt: WheelEvent| {
        let dy = wheel_delta_y(&evt);
        if let Some(ref gs) = *state.read() {
            let mut gs = gs.borrow_mut();
            gs.target_distance = (gs.target_distance + dy * 0.5).clamp(50.0, 2000.0);
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
                // touch-action: none keeps the browser from swallowing touch
                // drags as scroll/zoom so pointer moves reach us on mobile.
                style: "width: 100%; height: 100%; display: block; cursor: {cursor}; touch-action: none; -webkit-user-select: none; user-select: none;",
                onmounted: onmounted,
                onpointerdown: onpointerdown,
                onpointermove: onpointermove,
                onpointerup: onpointerup,
                onpointercancel: onpointercancel,
                onwheel: onwheel,
            }
            // Tooltip overlay
            if let Some(text) = &*tooltip_text.read() {
                div {
                    style: "position: absolute; pointer-events: none; white-space: pre; color: #ffffff; font-family: 'Space Grotesk', system-ui, sans-serif; font-size: 30px; text-shadow: 0 0 5px #000000, 2px 2px 18px #ff0072; text-align: center; transform: translate(-50%, -100%); left: {tooltip_x}px; top: {tooltip_y}px; font-weight: bold;",
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
    /// The currently visible subgraph (all nodes for the personal graph; center
    /// + hubs + expanded hubs' leaves for a collapsible atproto graph).
    data: GraphData,
    /// The full graph, and which hub ids are currently expanded.
    full: Rc<GraphData>,
    expanded: std::collections::HashSet<String>,
    /// Camera distance the render loop eases toward (refit on expand/collapse).
    target_distance: f32,
    aspect: f32,
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

/// The hub id a leaf hangs off (the source of the link that targets it).
fn leaf_parent(full: &GraphData, leaf_id: &str) -> Option<String> {
    full.links
        .iter()
        .find(|l| l.target == leaf_id)
        .map(|l| l.source.clone())
}

/// The subgraph currently visible: everything for a non-collapsible graph;
/// otherwise the center, all hubs, and the leaves of expanded hubs only.
fn filtered_graph(full: &GraphData, expanded: &std::collections::HashSet<String>) -> GraphData {
    if !full.collapsible {
        return full.clone();
    }
    let nodes: Vec<_> = full
        .nodes
        .iter()
        .filter(|n| {
            n.center || n.hub || leaf_parent(full, &n.id).is_none_or(|hub| expanded.contains(&hub))
        })
        .cloned()
        .collect();
    let visible: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let links = full
        .links
        .iter()
        .filter(|l| visible.contains(l.source.as_str()) && visible.contains(l.target.as_str()))
        .cloned()
        .collect();
    GraphData {
        nodes,
        links,
        circular_icons: full.circular_icons,
        collapsible: full.collapsible,
    }
}

/// Recompute the visible subgraph after an expand/collapse, seeding kept nodes at
/// their current positions and newly-revealed leaves at their hub (so they spring
/// outward), then reheat the layout and refit the camera.
fn rebuild(gs: &mut GraphState) {
    let mut positions = std::collections::HashMap::new();
    for (i, n) in gs.data.nodes.iter().enumerate() {
        positions.insert(n.id.clone(), gs.simulation.positions[i]);
    }
    let new_data = filtered_graph(&gs.full, &gs.expanded);
    let mut seed = positions.clone();
    for n in &new_data.nodes {
        if !seed.contains_key(&n.id)
            && let Some(hub) = leaf_parent(&gs.full, &n.id)
            && let Some(&p) = positions.get(&hub)
        {
            seed.insert(n.id.clone(), p + glam::Vec3::new(0.1, 0.1, 0.1));
        }
    }
    let mut simulation = Simulation::new(&new_data);
    simulation.set_positions(&new_data, &seed);
    simulation.set_alpha(0.7);
    gs.particle_system = ParticleSystem::new(&new_data);
    gs.target_distance = fit_camera_distance(&new_data, gs.aspect);
    gs.simulation = simulation;
    gs.data = new_data;
}

/// Pick a camera distance that frames the whole graph, whatever its size. A
/// throwaway copy of the layout is settled to measure its radius, then the
/// distance is `R / tan(fov/2)` with margin (and a portrait correction). This
/// reproduces the hand-tuned ~240 for the personal graph and pulls in tighter
/// for the more compact atproto star graphs.
fn fit_camera_distance(data: &GraphData, aspect: f32) -> f32 {
    let mut sim = Simulation::new(data);
    let mut ticks = 0;
    while sim.is_active() && ticks < 1000 {
        sim.tick();
        ticks += 1;
    }
    let max_r = sim
        .positions
        .iter()
        .map(|p| p.length())
        .fold(0.0_f32, f32::max);

    let half_fov = Camera::new().fov * 0.5;
    let fit = max_r / half_fov.tan() / aspect.min(1.0);
    (fit * 1.6).clamp(120.0, 2000.0)
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

            // Ease the camera toward its target distance (refit on expand/collapse
            // and smooth for wheel zoom).
            let target = gs.target_distance;
            gs.camera.distance += (target - gs.camera.distance) * 0.12;

            let particle_pos = gs.particle_system.positions(&gs.simulation, &gs.data);
            let textures = gs.textures.borrow();

            gs.renderer.render(
                &gs.camera,
                &gs.simulation,
                &gs.data,
                &textures,
                &particle_pos,
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use data::{GraphLink, GraphNode};

    fn node(id: &str, center: bool, hub: bool) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            desc: id.to_string(),
            icon: String::new(),
            color: None,
            opacity: None,
            url: (!center && !hub).then(|| format!("https://{id}")),
            center,
            hub,
        }
    }

    fn link(source: &str, target: &str) -> GraphLink {
        GraphLink {
            source: source.to_string(),
            target: target.to_string(),
        }
    }

    // A center node, two category hubs, and two leaves under each.
    fn sample() -> GraphData {
        GraphData {
            nodes: vec![
                node("me", true, false),
                node("Social", false, true),
                node("Build", false, true),
                node("Bluesky", false, false),
                node("PinkLeap", false, false),
                node("Tangled", false, false),
                node("Rocksky", false, false),
            ],
            links: vec![
                link("me", "Social"),
                link("me", "Build"),
                link("Social", "Bluesky"),
                link("Social", "PinkLeap"),
                link("Build", "Tangled"),
                link("Build", "Rocksky"),
            ],
            circular_icons: true,
            collapsible: true,
        }
    }

    fn ids(g: &GraphData) -> Vec<&str> {
        g.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn collapsed_shows_only_center_and_hubs() {
        let full = sample();
        let g = filtered_graph(&full, &std::collections::HashSet::new());
        assert_eq!(ids(&g), ["me", "Social", "Build"]);
        // Only the center->hub links survive; no dangling links to hidden leaves.
        assert_eq!(g.links.len(), 2);
        assert!(
            g.links
                .iter()
                .all(|l| l.target == "Social" || l.target == "Build")
        );
    }

    #[test]
    fn expanding_a_hub_reveals_only_its_leaves() {
        let full = sample();
        let expanded = std::collections::HashSet::from(["Social".to_string()]);
        let g = filtered_graph(&full, &expanded);
        assert_eq!(ids(&g), ["me", "Social", "Build", "Bluesky", "PinkLeap"]);
        // The Social->leaf links appear; Build's leaves stay hidden.
        assert!(g.links.iter().any(|l| l.target == "Bluesky"));
        assert!(g.links.iter().all(|l| l.target != "Tangled"));
    }

    #[test]
    fn expanding_all_hubs_matches_the_full_graph() {
        let full = sample();
        let expanded = std::collections::HashSet::from(["Social".to_string(), "Build".to_string()]);
        let g = filtered_graph(&full, &expanded);
        assert_eq!(ids(&g), ids(&full));
        assert_eq!(g.links.len(), full.links.len());
    }

    #[test]
    fn leaf_parent_finds_the_owning_hub() {
        let full = sample();
        assert_eq!(leaf_parent(&full, "Tangled").as_deref(), Some("Build"));
        assert_eq!(leaf_parent(&full, "Bluesky").as_deref(), Some("Social"));
        // The center has no incoming link.
        assert_eq!(leaf_parent(&full, "me"), None);
    }

    #[test]
    fn non_collapsible_graph_is_returned_whole() {
        let personal = GraphData::personal();
        let g = filtered_graph(&personal, &std::collections::HashSet::new());
        assert_eq!(g.nodes.len(), personal.nodes.len());
        assert_eq!(g.links.len(), personal.links.len());
    }
}
