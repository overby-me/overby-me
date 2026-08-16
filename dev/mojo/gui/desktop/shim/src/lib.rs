//! mojo-blitz-shim — C FFI shim for the Blitz HTML/CSS rendering engine.
//!
//! This Rust cdylib exposes Blitz's DOM manipulation, event handling, and
//! rendering pipeline via a flat C ABI so that Mojo can drive it through
//! `DLHandle` FFI calls.
//!
//! Architecture:
//!   Mojo → extern "C" calls → BlitzContext → blitz-dom / blitz-shell / blitz-paint
//!
//! The shim integrates three subsystems:
//!   1. DOM manipulation — create/modify nodes via DocumentMutator
//!   2. Windowing — Winit event loop with pump_app_events() for cooperative polling
//!   3. Rendering — Vello GPU renderer via anyrender_vello + blitz-paint
//!
//! Key design decisions:
//!   - Polling-based: no callbacks across FFI. Events are buffered in a ring buffer.
//!   - Node IDs are u32 (mapped from Blitz's usize slab keys).
//!   - Templates are stored as detached DOM subtrees, deep-cloned on use.
//!   - All functions must be called from the main/UI thread.
//!   - The Winit EventLoop is stored in an Option and temporarily taken out
//!     during pump_app_events() to avoid borrow conflicts with the document.

#![allow(private_interfaces)] // FFI functions intentionally expose opaque pointers to private types
#![allow(clippy::missing_safety_doc)] // All extern "C" functions share the same safety contract: ctx must be a valid pointer from mblitz_create()

use std::collections::HashMap;
use std::slice;
use std::sync::Arc;
use std::time::Duration;

use blitz_dom::{
    BaseDocument, DocumentConfig, ElementData, EventDriver, LocalName, NodeData, Prefix, QualName,
    local_name, ns,
};
use blitz_paint::paint_scene;
use blitz_traits::events::{
    BlitzMouseButtonEvent, DomEvent, DomEventData, EventState, MouseEventButton, MouseEventButtons,
    UiEvent,
};
use blitz_traits::shell::Viewport;

use anyrender::WindowRenderer;
use anyrender_vello::VelloWindowRenderer;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Window, WindowAttributes, WindowId};

// ═══════════════════════════════════════════════════════════════════════════
// Event types — must match mojo-gui/core event type constants
// ═══════════════════════════════════════════════════════════════════════════

const EVT_CLICK: u8 = 0;
const EVT_INPUT: u8 = 1;
const _EVT_CHANGE: u8 = 2;
const _EVT_KEYDOWN: u8 = 3;
const _EVT_KEYUP: u8 = 4;
const _EVT_FOCUS: u8 = 5;
const _EVT_BLUR: u8 = 6;
const _EVT_SUBMIT: u8 = 7;
const _EVT_MOUSEDOWN: u8 = 8;
const _EVT_MOUSEUP: u8 = 9;
const _EVT_MOUSEMOVE: u8 = 10;

// ═══════════════════════════════════════════════════════════════════════════
// Buffered event
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct BufferedEvent {
    pub handler_id: u32,
    pub event_type: u8,
    pub value: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// Event handler registration (mojo-gui handler_id → DOM event name)
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct MojoHandler {
    pub handler_id: u32,
    pub event_name: String,
}

// ═══════════════════════════════════════════════════════════════════════════
// MojoEventHandler — routes Blitz DOM events to mojo-gui handler IDs
// ═══════════════════════════════════════════════════════════════════════════

/// Custom EventHandler that intercepts DOM events during bubble propagation,
/// looks up registered mojo-gui handler IDs, and buffers them for polling.
struct MojoEventHandler<'a> {
    /// Reference to the mojo event handler registrations (Blitz node ID → handlers).
    event_handlers: &'a HashMap<usize, Vec<MojoHandler>>,
    /// Output buffer for events that matched a registered handler.
    event_queue: &'a mut Vec<BufferedEvent>,
}

impl blitz_dom::EventHandler for MojoEventHandler<'_> {
    fn handle_event(
        &mut self,
        chain: &[usize],
        event: &mut DomEvent,
        _mutr: &mut blitz_dom::DocumentMutator<'_>,
        _event_state: &mut EventState,
    ) {
        // Map DomEventData to the event name string used by mojo-gui
        let event_name = match &event.data {
            DomEventData::Click(_) => "click",
            DomEventData::Input(_) => "input",
            DomEventData::KeyDown(_) => "keydown",
            DomEventData::KeyUp(_) => "keyup",
            DomEventData::MouseDown(_) => "mousedown",
            DomEventData::MouseUp(_) => "mouseup",
            DomEventData::MouseMove(_) => "mousemove",
            DomEventData::Ime(_) => "input",
            DomEventData::KeyPress(_) => return, // not used by mojo-gui
        };

        let event_type = match &event.data {
            DomEventData::Click(_) => EVT_CLICK,
            DomEventData::Input(_) => EVT_INPUT,
            DomEventData::KeyDown(_) => _EVT_KEYDOWN,
            DomEventData::KeyUp(_) => _EVT_KEYUP,
            DomEventData::MouseDown(_) => _EVT_MOUSEDOWN,
            DomEventData::MouseUp(_) => _EVT_MOUSEUP,
            DomEventData::MouseMove(_) => _EVT_MOUSEMOVE,
            DomEventData::Ime(_) => EVT_INPUT,
            DomEventData::KeyPress(_) => return,
        };

        // Walk the bubble chain (target → ancestors), check for registered handlers
        for &node_id in chain {
            if let Some(handlers) = self.event_handlers.get(&node_id) {
                for handler in handlers {
                    if handler.event_name == event_name {
                        // Extract string value for input events
                        let value = match &event.data {
                            DomEventData::Input(data) => data.value.to_string(),
                            _ => String::new(),
                        };
                        self.event_queue.push(BufferedEvent {
                            handler_id: handler.handler_id,
                            event_type,
                            value,
                        });
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BlitzContext — owns the document, window, renderer, and event state
// ═══════════════════════════════════════════════════════════════════════════

pub struct BlitzContext {
    /// The Blitz DOM document.
    pub doc: BaseDocument,

    /// Node ID assigned to the mount point (the <body> or <div id="root">).
    /// mojo-gui's element ID 0 maps to this.
    pub mount_point_id: usize,

    /// Map from mojo-gui element IDs (u32) to Blitz slab node IDs (usize).
    /// Element ID 0 is always the mount point.
    pub id_to_node: HashMap<u32, usize>,

    /// Reverse map: Blitz node ID → mojo-gui element ID.
    /// Used for event dispatch (we need to find handler_id from the clicked node).
    pub node_to_id: HashMap<usize, u32>,

    /// Next available mojo-gui element ID for internally created nodes
    /// that don't get an explicit AssignId.
    #[allow(dead_code)]
    pub next_internal_id: u32,

    /// Registered templates: template_id → root Blitz node ID.
    pub templates: HashMap<u32, usize>,

    /// Event handlers: Blitz node ID → list of handlers.
    pub event_handlers: HashMap<usize, Vec<MojoHandler>>,

    /// Buffered events ready for Mojo to poll.
    pub event_queue: Vec<BufferedEvent>,

    /// Temporary storage for the last polled event's value string,
    /// kept alive until the next poll.
    pub last_polled_value: String,

    /// Stack for mutation interpretation (mirrors the JS interpreter's stack).
    /// Contains Blitz node IDs.
    pub stack: Vec<usize>,

    /// Whether the window is still alive.
    pub alive: bool,

    /// Debug mode flag.
    pub debug: bool,

    /// Whether we're currently inside a begin_mutations/end_mutations batch.
    pub in_mutation_batch: bool,

    // ── Windowing state (populated after first pump_app_events call) ────
    /// Winit event loop. Stored in an Option so it can be temporarily taken
    /// out during pump_app_events() (which needs &mut EventLoop while our
    /// ApplicationHandler impl needs &mut BlitzContext — same struct).
    pub event_loop: Option<EventLoop<()>>,

    /// The Winit window handle (Arc for sharing with the renderer).
    pub window: Option<Arc<Window>>,

    /// Vello GPU renderer. Wrapped in Option so it can be explicitly dropped
    /// before the event loop during teardown (GPU/EGL resources reference the
    /// Wayland display owned by the event loop).
    renderer: Option<VelloWindowRenderer>,

    /// Desired window title (stored until window is created).
    pub title: String,

    /// Desired window dimensions.
    pub initial_width: u32,
    pub initial_height: u32,

    /// Whether the window has been created and the renderer resumed.
    pub window_initialized: bool,

    /// Whether a redraw has been requested (set by mblitz_request_redraw,
    /// consumed during the next step).
    pub needs_redraw: bool,

    // ── Input state (tracked across Winit events) ──────────────────────
    /// Current mouse button state.
    pub mouse_buttons: MouseEventButtons,

    /// Current mouse position in logical pixels.
    pub mouse_pos: (f32, f32),
}

impl BlitzContext {
    fn new(title: &str, width: u32, height: u32, debug: bool) -> Self {
        let viewport = Viewport {
            window_size: (width, height),
            ..Default::default()
        };

        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };

        let mut doc = BaseDocument::new(config);

        // Build a minimal DOM structure: Document → <html> → <body>
        // The document root (node 0) is created by BaseDocument::new().
        // We need to create <html> and <body> elements.
        let html_name = QualName::new(None::<Prefix>, ns!(html), local_name!("html"));
        let html_id = doc.create_node(NodeData::Element(ElementData::new(html_name, vec![])));

        let body_name = QualName::new(None::<Prefix>, ns!(html), local_name!("body"));
        let body_id = doc.create_node(NodeData::Element(ElementData::new(body_name, vec![])));

        // Attach <html> to document root, <body> to <html>
        {
            let mut mutator = doc.mutate();
            mutator.append_children(0, &[html_id]);
            mutator.append_children(html_id, &[body_id]);
        }

        // Set window title via a <title> element
        if !title.is_empty() {
            let head_name = QualName::new(None::<Prefix>, ns!(html), local_name!("head"));
            let head_id = doc.create_node(NodeData::Element(ElementData::new(head_name, vec![])));

            let title_name = QualName::new(None::<Prefix>, ns!(html), local_name!("title"));
            let title_el_id =
                doc.create_node(NodeData::Element(ElementData::new(title_name, vec![])));
            let title_text_id = doc.create_text_node(title);

            let mut mutator = doc.mutate();
            mutator.insert_nodes_before(body_id, &[head_id]);
            mutator.append_children(head_id, &[title_el_id]);
            mutator.append_children(title_el_id, &[title_text_id]);
        }

        let mut id_to_node = HashMap::new();
        let mut node_to_id = HashMap::new();
        // Element ID 0 → body (mount point)
        id_to_node.insert(0, body_id);
        node_to_id.insert(body_id, 0);

        // Create the Winit event loop
        let event_loop = EventLoop::<()>::builder().build().unwrap();
        event_loop.set_control_flow(ControlFlow::Wait);

        BlitzContext {
            doc,
            mount_point_id: body_id,
            id_to_node,
            node_to_id,
            next_internal_id: 0x8000_0000, // Internal IDs start high to avoid collision
            templates: HashMap::new(),
            event_handlers: HashMap::new(),
            event_queue: Vec::new(),
            last_polled_value: String::new(),
            stack: Vec::new(),
            alive: true,
            debug,
            in_mutation_batch: false,

            // Windowing state
            event_loop: Some(event_loop),
            window: None,
            renderer: Some(VelloWindowRenderer::new()),
            title: title.to_string(),
            initial_width: width,
            initial_height: height,
            window_initialized: false,
            needs_redraw: false,

            // Input state
            mouse_buttons: MouseEventButtons::None,
            mouse_pos: (0.0, 0.0),
        }
    }

    /// Create a headless BlitzContext without an event loop or window.
    ///
    /// This is used for integration testing — DOM operations work normally
    /// but windowing/rendering functions are no-ops. Events can be injected
    /// via `queue_event()` and polled via `poll_event()`.
    pub fn new_headless(width: u32, height: u32) -> Self {
        let viewport = Viewport {
            window_size: (width, height),
            ..Default::default()
        };

        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };

        let mut doc = BaseDocument::new(config);

        let html_name = QualName::new(None::<Prefix>, ns!(html), local_name!("html"));
        let html_id = doc.create_node(NodeData::Element(ElementData::new(html_name, vec![])));

        let body_name = QualName::new(None::<Prefix>, ns!(html), local_name!("body"));
        let body_id = doc.create_node(NodeData::Element(ElementData::new(body_name, vec![])));

        {
            let mut mutator = doc.mutate();
            mutator.append_children(0, &[html_id]);
            mutator.append_children(html_id, &[body_id]);
        }

        let mut id_to_node = HashMap::new();
        let mut node_to_id = HashMap::new();
        id_to_node.insert(0, body_id);
        node_to_id.insert(body_id, 0);

        BlitzContext {
            doc,
            mount_point_id: body_id,
            id_to_node,
            node_to_id,
            next_internal_id: 0x8000_0000,
            templates: HashMap::new(),
            event_handlers: HashMap::new(),
            event_queue: Vec::new(),
            last_polled_value: String::new(),
            stack: Vec::new(),
            alive: true,
            debug: false,
            in_mutation_batch: false,

            // No event loop or window in headless mode
            event_loop: None,
            window: None,
            renderer: Some(VelloWindowRenderer::new()),
            title: String::new(),
            initial_width: width,
            initial_height: height,
            window_initialized: false,
            needs_redraw: false,

            mouse_buttons: MouseEventButtons::None,
            mouse_pos: (0.0, 0.0),
        }
    }

    /// Resolve a mojo-gui element ID to a Blitz node ID.
    pub fn resolve_id(&self, mojo_id: u32) -> Option<usize> {
        self.id_to_node.get(&mojo_id).copied()
    }

    /// Assign a mojo-gui element ID to a Blitz node ID.
    pub fn assign_id(&mut self, mojo_id: u32, blitz_id: usize) {
        self.id_to_node.insert(mojo_id, blitz_id);
        self.node_to_id.insert(blitz_id, mojo_id);
    }

    /// Allocate an internal element ID for nodes that don't get explicit AssignId.
    #[allow(dead_code)] // Will be used in Winit event loop integration (Step 4.6)
    pub fn alloc_internal_id(&mut self) -> u32 {
        let id = self.next_internal_id;
        self.next_internal_id += 1;
        id
    }

    /// Create an HTML element by tag name string.
    /// Uses DocumentMutator for proper stylo data initialization.
    pub fn create_element(&mut self, tag: &str) -> usize {
        let local = LocalName::from(tag);
        let name = QualName::new(None::<Prefix>, ns!(html), local);
        let mut mutator = self.doc.mutate();
        mutator.create_element(name, vec![])
    }

    /// Create a text node.
    pub fn create_text_node(&mut self, text: &str) -> usize {
        self.doc.create_text_node(text)
    }

    /// Create a comment/placeholder node.
    pub fn create_placeholder(&mut self) -> usize {
        self.doc.create_node(NodeData::Comment)
    }

    /// Set an attribute on a node (via DocumentMutator).
    pub fn set_attribute(&mut self, node_id: usize, name: &str, value: &str) {
        let qname = QualName::new(None::<Prefix>, ns!(), LocalName::from(name));
        let mut mutator = self.doc.mutate();
        mutator.set_attribute(node_id, qname, value);
    }

    /// Remove an attribute from a node.
    pub fn remove_attribute(&mut self, node_id: usize, name: &str) {
        let qname = QualName::new(None::<Prefix>, ns!(), LocalName::from(name));
        let mut mutator = self.doc.mutate();
        mutator.clear_attribute(node_id, qname);
    }

    /// Set text content of a text node.
    pub fn set_text_content(&mut self, node_id: usize, text: &str) {
        let mut mutator = self.doc.mutate();
        mutator.set_node_text(node_id, text);
    }

    /// Append children to a parent.
    pub fn append_children(&mut self, parent_id: usize, child_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.append_children(parent_id, child_ids);
    }

    /// Insert nodes before an anchor.
    pub fn insert_before(&mut self, anchor_id: usize, new_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.insert_nodes_before(anchor_id, new_ids);
    }

    /// Insert nodes after an anchor.
    pub fn insert_after(&mut self, anchor_id: usize, new_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.insert_nodes_after(anchor_id, new_ids);
    }

    /// Replace a node with new nodes.
    pub fn replace_with(&mut self, old_id: usize, new_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.replace_node_with(old_id, new_ids);
    }

    /// Remove and drop a node.
    pub fn remove_node(&mut self, node_id: usize) {
        // Clean up ID mappings
        if let Some(mojo_id) = self.node_to_id.remove(&node_id) {
            self.id_to_node.remove(&mojo_id);
        }
        self.event_handlers.remove(&node_id);

        let mut mutator = self.doc.mutate();
        mutator.remove_and_drop_node(node_id);
    }

    /// Deep clone a node.
    pub fn deep_clone_node(&mut self, node_id: usize) -> usize {
        self.doc.deep_clone_node(node_id)
    }

    /// Navigate to a child at path from a starting node.
    /// Uses the public `get_node` API to traverse children.
    pub fn node_at_path(&self, start_id: usize, path: &[u8]) -> usize {
        let mut current = start_id;
        for &idx in path {
            let node = self
                .doc
                .get_node(current)
                .expect("node_at_path: node not found");
            current = node.children[idx as usize];
        }
        current
    }

    /// Add an event handler registration.
    pub fn add_event_listener(&mut self, node_id: usize, handler_id: u32, event_name: &str) {
        let handlers = self.event_handlers.entry(node_id).or_default();
        handlers.push(MojoHandler {
            handler_id,
            event_name: event_name.to_string(),
        });
    }

    /// Remove an event handler registration.
    pub fn remove_event_listener(&mut self, node_id: usize, event_name: &str) {
        if let Some(handlers) = self.event_handlers.get_mut(&node_id) {
            handlers.retain(|h| h.event_name != event_name);
            if handlers.is_empty() {
                self.event_handlers.remove(&node_id);
            }
        }
    }

    /// Queue a synthetic event (for testing or programmatic dispatch).
    pub fn queue_event(&mut self, handler_id: u32, event_type: u8, value: String) {
        self.event_queue.push(BufferedEvent {
            handler_id,
            event_type,
            value,
        });
    }

    // ── DOM inspection methods (for testing) ────────────────────────

    /// Get the tag name of an element by its mojo-gui element ID.
    /// Returns an empty string for non-element nodes (text, comment).
    pub fn get_node_tag(&self, mojo_id: u32) -> String {
        let Some(blitz_id) = self.resolve_id(mojo_id) else {
            return String::new();
        };
        let Some(node) = self.doc.get_node(blitz_id) else {
            return String::new();
        };
        match &node.data {
            NodeData::Element(el) => el.name.local.to_string(),
            NodeData::Text(_) => "#text".to_string(),
            NodeData::Comment => "#comment".to_string(),
            NodeData::Document => "#document".to_string(),
            _ => String::new(),
        }
    }

    /// Get the concatenated text content of a node and its descendants.
    /// For element nodes, recursively collects all descendant text.
    /// For text nodes, returns the text directly.
    pub fn get_text_content(&self, mojo_id: u32) -> String {
        let Some(blitz_id) = self.resolve_id(mojo_id) else {
            return String::new();
        };
        self.collect_text(blitz_id)
    }

    /// Recursively collect text content from a Blitz node ID.
    fn collect_text(&self, blitz_id: usize) -> String {
        let Some(node) = self.doc.get_node(blitz_id) else {
            return String::new();
        };
        match &node.data {
            NodeData::Text(text_data) => text_data.content.to_string(),
            NodeData::Element(_) | NodeData::AnonymousBlock(_) => {
                let mut result = String::new();
                for &child_id in &node.children {
                    result.push_str(&self.collect_text(child_id));
                }
                result
            }
            _ => String::new(),
        }
    }

    /// Get the value of an attribute on an element by mojo-gui element ID.
    /// Returns None if the node doesn't exist, isn't an element, or
    /// doesn't have the attribute.
    pub fn get_attribute_value(&self, mojo_id: u32, attr_name: &str) -> Option<String> {
        let blitz_id = self.resolve_id(mojo_id)?;
        let node = self.doc.get_node(blitz_id)?;
        match &node.data {
            NodeData::Element(el) => {
                let local = LocalName::from(attr_name);
                el.attr(local).map(|v| v.to_string())
            }
            _ => None,
        }
    }

    /// Serialize a subtree rooted at a mojo-gui element ID into a compact
    /// HTML-like string for test assertions. Format:
    ///
    ///   <div><h1>#text("Hello")</h1><button>#text("Click")</button></div>
    ///
    /// Comment/placeholder nodes render as `<!---->`.
    pub fn serialize_subtree(&self, mojo_id: u32) -> String {
        let Some(blitz_id) = self.resolve_id(mojo_id) else {
            return String::new();
        };
        self.serialize_node(blitz_id)
    }

    /// Recursively serialize a single Blitz node.
    fn serialize_node(&self, blitz_id: usize) -> String {
        let Some(node) = self.doc.get_node(blitz_id) else {
            return String::new();
        };
        match &node.data {
            NodeData::Text(text_data) => {
                let content = text_data.content.to_string();
                format!("#text(\"{}\")", content.replace('"', "\\\""))
            }
            NodeData::Comment => "<!---->".to_string(),
            NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
                let tag = el.name.local.to_string();
                let mut attrs = String::new();
                for attr in el.attrs() {
                    attrs.push(' ');
                    attrs.push_str(&attr.name.local);
                    attrs.push_str("=\"");
                    attrs.push_str(&attr.value.to_string().replace('"', "&quot;"));
                    attrs.push('"');
                }
                let mut children_html = String::new();
                for &child_id in &node.children {
                    children_html.push_str(&self.serialize_node(child_id));
                }
                format!("<{tag}{attrs}>{children_html}</{tag}>")
            }
            _ => String::new(),
        }
    }

    /// Get the number of children of a node by mojo-gui element ID.
    pub fn get_child_count(&self, mojo_id: u32) -> u32 {
        let Some(blitz_id) = self.resolve_id(mojo_id) else {
            return 0;
        };
        let Some(node) = self.doc.get_node(blitz_id) else {
            return 0;
        };
        node.children.len() as u32
    }

    /// Get the mojo-gui element ID of a child at a given index.
    /// Returns u32::MAX if the child is not mapped or doesn't exist.
    pub fn get_child_mojo_id(&self, parent_mojo_id: u32, index: u32) -> u32 {
        let Some(parent_blitz_id) = self.resolve_id(parent_mojo_id) else {
            return u32::MAX;
        };
        let Some(node) = self.doc.get_node(parent_blitz_id) else {
            return u32::MAX;
        };
        let idx = index as usize;
        if idx >= node.children.len() {
            return u32::MAX;
        }
        let child_blitz_id = node.children[idx];
        self.node_to_id
            .get(&child_blitz_id)
            .copied()
            .unwrap_or(u32::MAX)
    }

    // ── Windowing methods ───────────────────────────────────────────────

    /// Process a single Winit window event, converting to Blitz UI events
    /// and routing DOM events to the mojo handler queue.
    fn handle_winit_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.alive = false;
            }

            WindowEvent::RedrawRequested => {
                // Resolve styles and layout, then paint
                self.doc.resolve(0.0);
                let (width, height) = self.doc.viewport().window_size;
                let scale = self.doc.viewport().scale_f64();
                if let Some(ref mut renderer) = self.renderer {
                    renderer.render(|scene| {
                        paint_scene(scene, &self.doc, scale, width, height);
                    });
                }
            }

            WindowEvent::Resized(physical_size) => {
                let (w, h) = (physical_size.width, physical_size.height);
                if w > 0 && h > 0 {
                    let mut viewport = self.doc.viewport().clone();
                    viewport.window_size = (w, h);
                    self.doc.set_viewport(viewport);
                    if let Some(ref mut renderer) = self.renderer {
                        renderer.set_size(w, h);
                    }
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let mut viewport = self.doc.viewport().clone();
                viewport.set_hidpi_scale(scale_factor as f32);
                self.doc.set_viewport(viewport);
            }

            WindowEvent::CursorMoved { position, .. } => {
                if let Some(ref window) = self.window {
                    let scale = window.scale_factor();
                    let pos: winit::dpi::LogicalPosition<f32> = position.to_logical(scale);
                    self.mouse_pos = (pos.x, pos.y);

                    let ui_event = UiEvent::MouseMove(BlitzMouseButtonEvent {
                        x: pos.x,
                        y: pos.y,
                        button: Default::default(),
                        buttons: self.mouse_buttons,
                        mods: Default::default(),
                    });
                    self.dispatch_ui_event(ui_event);
                }
            }

            WindowEvent::MouseInput {
                button: mouse_button,
                state,
                ..
            } => {
                let btn = match mouse_button {
                    MouseButton::Left => MouseEventButton::Main,
                    MouseButton::Right => MouseEventButton::Secondary,
                    _ => return,
                };

                // Mouse position is already tracked by CursorMoved events

                match state {
                    ElementState::Pressed => self.mouse_buttons |= btn.into(),
                    ElementState::Released => self.mouse_buttons ^= btn.into(),
                }

                let event_data = BlitzMouseButtonEvent {
                    x: self.mouse_pos.0,
                    y: self.mouse_pos.1,
                    button: btn,
                    buttons: self.mouse_buttons,
                    mods: Default::default(),
                };

                let ui_event = match state {
                    ElementState::Pressed => UiEvent::MouseDown(event_data),
                    ElementState::Released => UiEvent::MouseUp(event_data),
                };
                self.dispatch_ui_event(ui_event);

                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }

            // Ignore other events for now
            _ => {}
        }
    }

    /// Dispatch a UiEvent through the Blitz event pipeline with our custom
    /// MojoEventHandler that captures DOM events for polling.
    fn dispatch_ui_event(&mut self, event: UiEvent) {
        // We need to split borrows: the EventDriver borrows the document
        // (via mutate()), while MojoEventHandler borrows event_handlers
        // and event_queue. These are disjoint fields of BlitzContext.
        //
        // To satisfy the borrow checker, we use a raw pointer trick:
        // take references to disjoint fields before creating the driver.
        let event_handlers_ptr = &self.event_handlers as *const HashMap<usize, Vec<MojoHandler>>;
        let event_queue_ptr = &mut self.event_queue as *mut Vec<BufferedEvent>;

        let handler = MojoEventHandler {
            event_handlers: unsafe { &*event_handlers_ptr },
            event_queue: unsafe { &mut *event_queue_ptr },
        };

        let mut driver = EventDriver::new(self.doc.mutate(), handler);
        driver.handle_ui_event(event);
    }

    /// Poll the next event from the queue.
    pub fn poll_event(&mut self) -> Option<BufferedEvent> {
        if self.event_queue.is_empty() {
            None
        } else {
            Some(self.event_queue.remove(0))
        }
    }

    /// Push a node ID onto the interpreter stack.
    pub fn stack_push(&mut self, node_id: usize) {
        self.stack.push(node_id);
    }

    /// Pop N node IDs from the interpreter stack.
    pub fn stack_pop_n(&mut self, n: usize) -> Vec<usize> {
        let start = self.stack.len().saturating_sub(n);
        self.stack.drain(start..).collect()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Drop — ensure GPU resources are released before the Wayland connection
// ═══════════════════════════════════════════════════════════════════════════

impl Drop for BlitzContext {
    fn drop(&mut self) {
        // The renderer holds EGL/GPU resources that reference the Wayland
        // display owned by the event loop. Rust drops struct fields in
        // declaration order, which would tear down the event loop (and its
        // Wayland connection) *before* the renderer — causing a SIGSEGV
        // when eglTerminate tries to use the dead display.
        //
        // Fully drop the renderer first (releasing all GPU/EGL resources)
        // while the Wayland connection is still alive, then drop the window
        // before the event loop.
        drop(self.renderer.take());
        self.window = None;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ApplicationHandler — Winit event loop integration
// ═══════════════════════════════════════════════════════════════════════════

impl ApplicationHandler for BlitzContext {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            // Create the window on first resumed notification
            let attrs = WindowAttributes::default()
                .with_title(self.title.clone())
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.initial_width,
                    self.initial_height,
                ));
            let winit_window = Arc::new(event_loop.create_window(attrs).unwrap());
            winit_window.set_ime_allowed(true);

            // Update document viewport with actual window size and scale
            let size = winit_window.inner_size();
            let scale = winit_window.scale_factor() as f32;
            let viewport = Viewport::new(size.width, size.height, scale, Default::default());
            self.doc.set_viewport(viewport);

            // Resume the Vello renderer — anyrender 0.6 expects Arc<dyn WindowHandle>
            let (width, height) = (size.width, size.height);
            if let Some(ref mut renderer) = self.renderer {
                renderer.resume(
                    winit_window.clone() as Arc<dyn anyrender::WindowHandle>,
                    width,
                    height,
                );
            }

            self.window = Some(winit_window);
            self.window_initialized = true;
        } else {
            // Re-resume after suspend
            if let Some(ref window) = self.window {
                let size = window.inner_size();
                if let Some(ref mut renderer) = self.renderer {
                    renderer.resume(
                        window.clone() as Arc<dyn anyrender::WindowHandle>,
                        size.width,
                        size.height,
                    );
                }
            }
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(ref mut renderer) = self.renderer {
            renderer.suspend();
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        self.handle_winit_event(event);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper: read a UTF-8 string from a pointer + length (no null terminator)
// ═══════════════════════════════════════════════════════════════════════════

unsafe fn str_from_ptr(ptr: *const u8, len: u32) -> &'static str {
    if ptr.is_null() || len == 0 {
        return "";
    }
    let bytes = unsafe { slice::from_raw_parts(ptr, len as usize) };
    std::str::from_utf8(bytes).unwrap_or("")
}

// ═══════════════════════════════════════════════════════════════════════════
// FFI event structure
// ═══════════════════════════════════════════════════════════════════════════

#[repr(C)]
pub struct MblitzEvent {
    pub valid: i32,
    pub handler_id: u32,
    pub event_type: u8,
    pub value_ptr: *const u8,
    pub value_len: u32,
}

impl Default for MblitzEvent {
    fn default() -> Self {
        MblitzEvent {
            valid: 0,
            handler_id: 0,
            event_type: 0,
            value_ptr: std::ptr::null(),
            value_len: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Lifecycle FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_create(
    title: *const u8,
    title_len: u32,
    width: u32,
    height: u32,
    debug: i32,
) -> *mut BlitzContext {
    let title_str = unsafe { str_from_ptr(title, title_len) };
    let ctx = BlitzContext::new(title_str, width, height, debug != 0);
    Box::into_raw(Box::new(ctx))
}

/// Create a headless BlitzContext (no event loop, no window, no GPU).
/// Used for integration testing — DOM operations work, rendering is skipped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_create_headless(width: u32, height: u32) -> *mut BlitzContext {
    let ctx = BlitzContext::new_headless(width, height);
    Box::into_raw(Box::new(ctx))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_destroy(ctx: *mut BlitzContext) {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_is_alive(ctx: *mut BlitzContext) -> i32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    if ctx.alive { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_step(ctx: *mut BlitzContext, blocking: i32) -> i32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };

    // Take the event loop out temporarily so we can pass `ctx` as the
    // ApplicationHandler (pump_app_events needs &mut EventLoop and
    // &mut ApplicationHandler — both are &mut BlitzContext otherwise).
    let Some(mut event_loop) = ctx.event_loop.take() else {
        return 0;
    };

    // If a redraw was requested, ensure we trigger it
    if ctx.needs_redraw {
        if let Some(ref window) = ctx.window {
            window.request_redraw();
        }
        ctx.needs_redraw = false;
    }

    let timeout = if blocking != 0 {
        // Block until an event arrives (up to 100ms to allow periodic checks)
        Some(Duration::from_millis(100))
    } else {
        // Non-blocking: process pending events and return immediately
        Some(Duration::ZERO)
    };

    let status = event_loop.pump_app_events(timeout, &mut *ctx);

    // Check if the event loop requested exit
    if matches!(status, PumpStatus::Exit(_)) {
        ctx.alive = false;
    }

    // Put the event loop back
    ctx.event_loop = Some(event_loop);

    if ctx.alive { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_request_redraw(ctx: *mut BlitzContext) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.needs_redraw = true;
    if let Some(ref window) = ctx.window {
        window.request_redraw();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Window management FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_set_title(
    ctx: *mut BlitzContext,
    title: *const u8,
    title_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let title_str = unsafe { str_from_ptr(title, title_len) };
    ctx.title = title_str.to_string();
    if let Some(ref window) = ctx.window {
        window.set_title(title_str);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_set_size(ctx: *mut BlitzContext, width: u32, height: u32) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let viewport = Viewport {
        window_size: (width, height),
        ..Default::default()
    };
    ctx.doc.set_viewport(viewport);
}

// ═══════════════════════════════════════════════════════════════════════════
// User-agent stylesheet FFI
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_add_ua_stylesheet(
    ctx: *mut BlitzContext,
    css: *const u8,
    css_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let css_str = unsafe { str_from_ptr(css, css_len) };
    ctx.doc.add_user_agent_stylesheet(css_str);
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM node creation FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_create_element(
    ctx: *mut BlitzContext,
    tag: *const u8,
    tag_len: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };
    let tag_str = unsafe { str_from_ptr(tag, tag_len) };
    let node_id = ctx.create_element(tag_str);
    node_id as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_create_text_node(
    ctx: *mut BlitzContext,
    text: *const u8,
    text_len: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };
    let text_str = unsafe { str_from_ptr(text, text_len) };
    let node_id = ctx.create_text_node(text_str);
    node_id as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_create_placeholder(ctx: *mut BlitzContext) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };
    let node_id = ctx.create_placeholder();
    node_id as u32
}

// ═══════════════════════════════════════════════════════════════════════════
// Template FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_register_template(
    ctx: *mut BlitzContext,
    tmpl_id: u32,
    root_id: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.templates.insert(tmpl_id, root_id as usize);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_clone_template(ctx: *mut BlitzContext, tmpl_id: u32) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };
    let Some(&root_id) = ctx.templates.get(&tmpl_id) else {
        return 0;
    };
    let cloned = ctx.deep_clone_node(root_id);
    cloned as u32
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM tree mutation FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_append_children(
    ctx: *mut BlitzContext,
    parent_id: u32,
    child_ids: *const u32,
    child_count: u32,
) {
    if ctx.is_null() || child_ids.is_null() || child_count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let ids_slice = unsafe { slice::from_raw_parts(child_ids, child_count as usize) };

    // Resolve the parent: mojo element ID 0 → mount point
    let parent_blitz = if parent_id == 0 {
        ctx.mount_point_id
    } else {
        ctx.resolve_id(parent_id).unwrap_or(parent_id as usize)
    };

    // Child IDs may be raw Blitz node IDs (from create_element) or
    // mojo element IDs (from the stack). Try resolving, fall back to raw.
    let blitz_children: Vec<usize> = ids_slice
        .iter()
        .map(|&id| ctx.resolve_id(id).unwrap_or(id as usize))
        .collect();

    ctx.append_children(parent_blitz, &blitz_children);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_insert_before(
    ctx: *mut BlitzContext,
    anchor_id: u32,
    new_ids: *const u32,
    new_count: u32,
) {
    if ctx.is_null() || new_ids.is_null() || new_count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let ids_slice = unsafe { slice::from_raw_parts(new_ids, new_count as usize) };
    let anchor_blitz = ctx.resolve_id(anchor_id).unwrap_or(anchor_id as usize);
    let blitz_new: Vec<usize> = ids_slice
        .iter()
        .map(|&id| ctx.resolve_id(id).unwrap_or(id as usize))
        .collect();
    ctx.insert_before(anchor_blitz, &blitz_new);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_insert_after(
    ctx: *mut BlitzContext,
    anchor_id: u32,
    new_ids: *const u32,
    new_count: u32,
) {
    if ctx.is_null() || new_ids.is_null() || new_count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let ids_slice = unsafe { slice::from_raw_parts(new_ids, new_count as usize) };
    let anchor_blitz = ctx.resolve_id(anchor_id).unwrap_or(anchor_id as usize);
    let blitz_new: Vec<usize> = ids_slice
        .iter()
        .map(|&id| ctx.resolve_id(id).unwrap_or(id as usize))
        .collect();
    ctx.insert_after(anchor_blitz, &blitz_new);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_replace_with(
    ctx: *mut BlitzContext,
    old_id: u32,
    new_ids: *const u32,
    new_count: u32,
) {
    if ctx.is_null() || new_ids.is_null() || new_count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let ids_slice = unsafe { slice::from_raw_parts(new_ids, new_count as usize) };
    let old_blitz = ctx.resolve_id(old_id).unwrap_or(old_id as usize);
    let blitz_new: Vec<usize> = ids_slice
        .iter()
        .map(|&id| ctx.resolve_id(id).unwrap_or(id as usize))
        .collect();

    // Clean up ID mapping for old node
    if let Some(mojo_id) = ctx.node_to_id.remove(&old_blitz) {
        ctx.id_to_node.remove(&mojo_id);
    }
    ctx.event_handlers.remove(&old_blitz);

    ctx.replace_with(old_blitz, &blitz_new);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_remove_node(ctx: *mut BlitzContext, node_id: u32) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.remove_node(blitz_id);
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM attributes FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_set_attribute(
    ctx: *mut BlitzContext,
    node_id: u32,
    name: *const u8,
    name_len: u32,
    value: *const u8,
    value_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let name_str = unsafe { str_from_ptr(name, name_len) };
    let value_str = unsafe { str_from_ptr(value, value_len) };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.set_attribute(blitz_id, name_str, value_str);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_remove_attribute(
    ctx: *mut BlitzContext,
    node_id: u32,
    name: *const u8,
    name_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let name_str = unsafe { str_from_ptr(name, name_len) };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.remove_attribute(blitz_id, name_str);
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM text content FFI export
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_set_text_content(
    ctx: *mut BlitzContext,
    node_id: u32,
    text: *const u8,
    text_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let text_str = unsafe { str_from_ptr(text, text_len) };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.set_text_content(blitz_id, text_str);
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM tree traversal FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_node_at_path(
    ctx: *mut BlitzContext,
    start_id: u32,
    path: *const u8,
    path_len: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };
    let start_blitz = ctx.resolve_id(start_id).unwrap_or(start_id as usize);
    if path.is_null() || path_len == 0 {
        return start_blitz as u32;
    }
    let path_slice = unsafe { slice::from_raw_parts(path, path_len as usize) };
    let result = ctx.node_at_path(start_blitz, path_slice);
    result as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_child_at(ctx: *mut BlitzContext, node_id: u32, index: u32) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    let blitz_id = ctx
        .id_to_node
        .get(&node_id)
        .copied()
        .unwrap_or(node_id as usize);
    let node = match ctx.doc.get_node(blitz_id) {
        Some(n) => n,
        None => return 0,
    };
    match node.children.get(index as usize) {
        Some(&child_id) => child_id as u32,
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_child_count(ctx: *mut BlitzContext, node_id: u32) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    let blitz_id = ctx
        .id_to_node
        .get(&node_id)
        .copied()
        .unwrap_or(node_id as usize);
    let node = match ctx.doc.get_node(blitz_id) {
        Some(n) => n,
        None => return 0,
    };
    node.children.len() as u32
}

// ═══════════════════════════════════════════════════════════════════════════
// Event handling FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_add_event_listener(
    ctx: *mut BlitzContext,
    node_id: u32,
    handler_id: u32,
    event_name: *const u8,
    event_name_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let name = unsafe { str_from_ptr(event_name, event_name_len) };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.add_event_listener(blitz_id, handler_id, name);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_remove_event_listener(
    ctx: *mut BlitzContext,
    node_id: u32,
    event_name: *const u8,
    event_name_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let name = unsafe { str_from_ptr(event_name, event_name_len) };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.remove_event_listener(blitz_id, name);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_poll_event(ctx: *mut BlitzContext) -> MblitzEvent {
    if ctx.is_null() {
        return MblitzEvent::default();
    }
    let ctx = unsafe { &mut *ctx };
    match ctx.poll_event() {
        Some(event) => {
            ctx.last_polled_value = event.value.clone();
            MblitzEvent {
                valid: 1,
                handler_id: event.handler_id,
                event_type: event.event_type,
                value_ptr: if ctx.last_polled_value.is_empty() {
                    std::ptr::null()
                } else {
                    ctx.last_polled_value.as_ptr()
                },
                value_len: ctx.last_polled_value.len() as u32,
            }
        }
        None => MblitzEvent::default(),
    }
}

/// Poll the next event, writing fields to caller-provided output pointers.
///
/// This avoids struct-return ABI issues with Mojo's DLHandle by writing
/// each field to a separate output pointer.
///
/// Returns 1 if an event was available, 0 if the queue was empty.
/// When returning 0, the output pointers are not modified.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_poll_event_into(
    ctx: *mut BlitzContext,
    out_handler_id: *mut u32,
    out_event_type: *mut u8,
    out_value_ptr: *mut *const u8,
    out_value_len: *mut u32,
) -> i32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *ctx };
    match ctx.poll_event() {
        Some(event) => {
            unsafe {
                *out_handler_id = event.handler_id;
                *out_event_type = event.event_type;
            }
            ctx.last_polled_value = event.value;
            unsafe {
                if ctx.last_polled_value.is_empty() {
                    *out_value_ptr = std::ptr::null();
                    *out_value_len = 0;
                } else {
                    *out_value_ptr = ctx.last_polled_value.as_ptr();
                    *out_value_len = ctx.last_polled_value.len() as u32;
                }
            }
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_event_count(ctx: *mut BlitzContext) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    ctx.event_queue.len() as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_event_clear(ctx: *mut BlitzContext) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.event_queue.clear();
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation batch FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_begin_mutations(ctx: *mut BlitzContext) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.in_mutation_batch = true;
    // Note: In the future, we could acquire a DocumentMutator here and hold
    // it for the duration of the batch. For now, each DOM operation creates
    // its own short-lived mutator (which is correct but slightly less
    // efficient due to repeated flush cycles).
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_end_mutations(ctx: *mut BlitzContext) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.in_mutation_batch = false;
    // Trigger any pending style/layout invalidations.
    // The DocumentMutator's Drop impl handles deferred processing (style
    // elements, linked stylesheets, images, fonts, title updates, etc.).
    // Since we create/drop mutators per-operation right now, this is
    // already handled. Once we hold a long-lived mutator for the batch,
    // we'll drop it here to trigger the flush.
}

// ═══════════════════════════════════════════════════════════════════════════
// Document root access FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_root_node_id(_ctx: *mut BlitzContext) -> u32 {
    // The Blitz document root is always node 0.
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_mount_point_id(ctx: *mut BlitzContext) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    ctx.mount_point_id as u32
}

// ═══════════════════════════════════════════════════════════════════════════
// Layout FFI export
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_resolve_layout(ctx: *mut BlitzContext) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    // Resolve styles (Stylo) and layout (Taffy) for the entire document.
    ctx.doc.resolve(0.0);
}

// ═══════════════════════════════════════════════════════════════════════════
// Stack operations (used by the Mojo mutation interpreter)
// ═══════════════════════════════════════════════════════════════════════════

/// Push a node onto the interpreter stack.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_stack_push(ctx: *mut BlitzContext, node_id: u32) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let blitz_id = ctx.resolve_id(node_id).unwrap_or(node_id as usize);
    ctx.stack_push(blitz_id);
}

/// Pop N nodes from the stack and append them as children of the given parent.
/// This mirrors OP_APPEND_CHILDREN behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_stack_pop_append(
    ctx: *mut BlitzContext,
    parent_id: u32,
    count: u32,
) {
    if ctx.is_null() || count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let parent_blitz = if parent_id == 0 {
        ctx.mount_point_id
    } else {
        ctx.resolve_id(parent_id).unwrap_or(parent_id as usize)
    };
    let children = ctx.stack_pop_n(count as usize);
    ctx.append_children(parent_blitz, &children);
}

/// Pop N nodes from the stack and use them to replace the node with the given ID.
/// This mirrors OP_REPLACE_WITH behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_stack_pop_replace(ctx: *mut BlitzContext, old_id: u32, count: u32) {
    if ctx.is_null() || count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let old_blitz = ctx.resolve_id(old_id).unwrap_or(old_id as usize);
    let replacements = ctx.stack_pop_n(count as usize);

    // Clean up ID mapping for old node
    if let Some(mojo_id) = ctx.node_to_id.remove(&old_blitz) {
        ctx.id_to_node.remove(&mojo_id);
    }
    ctx.event_handlers.remove(&old_blitz);

    ctx.replace_with(old_blitz, &replacements);
}

/// Pop N nodes from the stack and insert them before the anchor node.
/// This mirrors OP_INSERT_BEFORE behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_stack_pop_insert_before(
    ctx: *mut BlitzContext,
    anchor_id: u32,
    count: u32,
) {
    if ctx.is_null() || count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let anchor_blitz = ctx.resolve_id(anchor_id).unwrap_or(anchor_id as usize);
    let new_nodes = ctx.stack_pop_n(count as usize);
    ctx.insert_before(anchor_blitz, &new_nodes);
}

/// Pop N nodes from the stack and insert them after the anchor node.
/// This mirrors OP_INSERT_AFTER behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_stack_pop_insert_after(
    ctx: *mut BlitzContext,
    anchor_id: u32,
    count: u32,
) {
    if ctx.is_null() || count == 0 {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let anchor_blitz = ctx.resolve_id(anchor_id).unwrap_or(anchor_id as usize);
    let new_nodes = ctx.stack_pop_n(count as usize);
    ctx.insert_after(anchor_blitz, &new_nodes);
}

/// Assign a mojo-gui element ID to a Blitz node ID.
/// Used by the mutation interpreter for OP_ASSIGN_ID.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_assign_id(
    ctx: *mut BlitzContext,
    mojo_id: u32,
    blitz_node_id: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.assign_id(mojo_id, blitz_node_id as usize);
}

// ═══════════════════════════════════════════════════════════════════════════
// Debug / diagnostics FFI exports
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_print_tree(ctx: *mut BlitzContext) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &*ctx };
    ctx.doc.print_tree();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_set_debug_overlay(ctx: *mut BlitzContext, enabled: i32) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    ctx.debug = enabled != 0;
}

static VERSION: &str = concat!("mojo-blitz-shim ", env!("CARGO_PKG_VERSION"), "\0");

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_version(out_ptr: *mut *const u8, out_len: *mut u32) {
    if !out_ptr.is_null() {
        unsafe { *out_ptr = VERSION.as_ptr() };
    }
    if !out_len.is_null() {
        unsafe { *out_len = (VERSION.len() - 1) as u32 }; // Exclude null terminator
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM inspection FFI exports (for testing)
// ═══════════════════════════════════════════════════════════════════════════

/// Get the tag name of an element by mojo-gui element ID.
/// Writes the tag string to the provided buffer. Returns the number of
/// bytes written, or 0 if the node doesn't exist or isn't an element.
/// If the buffer is too small, returns the required length (caller retries).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_get_node_tag(
    ctx: *mut BlitzContext,
    mojo_id: u32,
    out_ptr: *mut u8,
    out_capacity: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    let tag = ctx.get_node_tag(mojo_id);
    if tag.is_empty() {
        return 0;
    }
    let bytes = tag.as_bytes();
    if !out_ptr.is_null() && out_capacity as usize >= bytes.len() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
        }
    }
    bytes.len() as u32
}

/// Get the concatenated text content of a node and its descendants.
/// Returns the number of bytes written, or the required length if buffer
/// is too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_get_text_content(
    ctx: *mut BlitzContext,
    mojo_id: u32,
    out_ptr: *mut u8,
    out_capacity: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    let text = ctx.get_text_content(mojo_id);
    if text.is_empty() {
        return 0;
    }
    let bytes = text.as_bytes();
    if !out_ptr.is_null() && out_capacity as usize >= bytes.len() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
        }
    }
    bytes.len() as u32
}

/// Get the value of an attribute on an element.
/// Returns the number of bytes written, 0 if not found, or the required
/// length if the buffer is too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_get_attribute_value(
    ctx: *mut BlitzContext,
    mojo_id: u32,
    attr_name: *const u8,
    attr_name_len: u32,
    out_ptr: *mut u8,
    out_capacity: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    let name = unsafe { str_from_ptr(attr_name, attr_name_len) };
    let Some(value) = ctx.get_attribute_value(mojo_id, name) else {
        return 0;
    };
    let bytes = value.as_bytes();
    if !out_ptr.is_null() && out_capacity as usize >= bytes.len() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
        }
    }
    bytes.len() as u32
}

/// Serialize a subtree rooted at a mojo-gui element ID into a compact
/// HTML-like string. Returns the number of bytes written, or required
/// length if the buffer is too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_serialize_subtree(
    ctx: *mut BlitzContext,
    mojo_id: u32,
    out_ptr: *mut u8,
    out_capacity: u32,
) -> u32 {
    if ctx.is_null() {
        return 0;
    }
    let ctx = unsafe { &*ctx };
    let html = ctx.serialize_subtree(mojo_id);
    if html.is_empty() {
        return 0;
    }
    let bytes = html.as_bytes();
    if !out_ptr.is_null() && out_capacity as usize >= bytes.len() {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
        }
    }
    bytes.len() as u32
}

/// Inject a synthetic event into the event queue for testing.
/// The event will be returned by the next mblitz_poll_event() call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_inject_event(
    ctx: *mut BlitzContext,
    handler_id: u32,
    event_type: u8,
    value_ptr: *const u8,
    value_len: u32,
) {
    if ctx.is_null() {
        return;
    }
    let ctx = unsafe { &mut *ctx };
    let value = unsafe { str_from_ptr(value_ptr, value_len) }.to_string();
    ctx.queue_event(handler_id, event_type, value);
}

/// Get the mojo-gui element ID of a child at a given index within a parent.
/// Returns u32::MAX if out of bounds or unmapped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblitz_get_child_mojo_id(
    ctx: *mut BlitzContext,
    parent_mojo_id: u32,
    index: u32,
) -> u32 {
    if ctx.is_null() {
        return u32::MAX;
    }
    let ctx = unsafe { &*ctx };
    ctx.get_child_mojo_id(parent_mojo_id, index)
}
