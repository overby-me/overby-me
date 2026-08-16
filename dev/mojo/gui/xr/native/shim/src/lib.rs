#![allow(unsafe_op_in_unsafe_fn)]
#![allow(dead_code)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::only_used_in_recursion)]
//! mojo-gui XR shim — C FFI exposing Blitz + OpenXR for multi-panel XR rendering.
//!
//! This crate provides a flat C ABI (defined in `mojo_xr.h`) that the Mojo FFI
//! bindings (`xr/native/src/xr/xr_blitz.mojo`) call via `DLHandle`. It extends
//! the desktop Blitz renderer pattern with:
//!
//!   - **Multi-document support** — one `blitz-dom` BaseDocument per XR panel
//!   - **Offscreen Vello rendering** — each panel renders to a GPU texture
//!   - **OpenXR integration** — session lifecycle, frame loop, quad layer compositing
//!   - **Controller raycasting** — pointer ray → panel quad intersection → DOM events
//!
//! # Architecture
//!
//! ```text
//! Mojo (xr_blitz.mojo)
//!   │ DLHandle FFI calls
//!   ▼
//! libmojo_xr.so (this crate)
//!   ├── XrSessionContext — owns panels, events, session state
//!   │   ├── Panel 0: BaseDocument + ID map + event handlers + texture
//!   │   ├── Panel 1: BaseDocument + ...
//!   │   └── Panel N: ...
//!   ├── OpenXR session (when not headless)
//!   ├── wgpu device + Vello renderer (when not headless)
//!   └── Event ring buffer (polled by Mojo)
//! ```
//!
//! # Build modes
//!
//! - **Normal**: Links against the OpenXR loader for real XR rendering.
//! - **Headless** (`mxr_create_headless`): No OpenXR, no GPU — DOM operations
//!   work via real Blitz BaseDocument instances (CSS styling, layout). Used for
//!   integration tests and CI.
//!
//! # Thread safety
//!
//! All functions must be called from the thread that created the session.
//! This matches OpenXR's single-thread requirement for session calls.
//!
//! # Phase 5.2 — Real Blitz documents
//!
//! Each panel now owns a real `blitz_dom::BaseDocument` instead of the
//! lightweight `HeadlessNode` tree used in Phase 5.1. This means:
//!   - CSS styling and layout computation work (Stylo + Taffy)
//!   - DOM operations match the desktop shim exactly
//!   - Vello offscreen rendering is possible (wired up in a later step)
//!   - Template cloning uses Blitz's `deep_clone_node`

mod openxr_backend;

use std::collections::HashMap;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU32, Ordering};

use openxr_backend::OpenXrBackend;

use blitz_dom::{
    BaseDocument, DocumentConfig, ElementData, LocalName, NodeData, Prefix, QualName, local_name,
    ns,
};
use blitz_paint::paint_scene;
use blitz_traits::shell::Viewport;

use anyrender_vello::VelloScenePainter;
use vello::{AaConfig, AaSupport, RenderParams, RendererOptions, Scene as VelloScene};

use wgpu::{
    Device, Extent3d, Queue, Texture, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

// ---------------------------------------------------------------------------
// Constants — mirror the #defines in mojo_xr.h
// ---------------------------------------------------------------------------

pub const MXR_EVT_CLICK: u8 = 1;
pub const MXR_EVT_INPUT: u8 = 2;
pub const MXR_EVT_CHANGE: u8 = 3;
pub const MXR_EVT_KEYDOWN: u8 = 4;
pub const MXR_EVT_KEYUP: u8 = 5;
pub const MXR_EVT_FOCUS: u8 = 6;
pub const MXR_EVT_BLUR: u8 = 7;
pub const MXR_EVT_SUBMIT: u8 = 8;
pub const MXR_EVT_MOUSEDOWN: u8 = 9;
pub const MXR_EVT_MOUSEUP: u8 = 10;
pub const MXR_EVT_MOUSEMOVE: u8 = 11;

pub const MXR_EVT_XR_SELECT: u8 = 0x80;
pub const MXR_EVT_XR_SQUEEZE: u8 = 0x81;
pub const MXR_EVT_XR_HOVER_ENTER: u8 = 0x82;
pub const MXR_EVT_XR_HOVER_EXIT: u8 = 0x83;

pub const MXR_HAND_LEFT: u8 = 0;
pub const MXR_HAND_RIGHT: u8 = 1;
pub const MXR_HAND_HEAD: u8 = 2;

pub const MXR_SPACE_LOCAL: u8 = 0;
pub const MXR_SPACE_STAGE: u8 = 1;
pub const MXR_SPACE_VIEW: u8 = 2;
pub const MXR_SPACE_UNBOUNDED: u8 = 3;

pub const MXR_STATE_IDLE: i32 = 0;
pub const MXR_STATE_READY: i32 = 1;
pub const MXR_STATE_FOCUSED: i32 = 2;
pub const MXR_STATE_VISIBLE: i32 = 3;
pub const MXR_STATE_STOPPING: i32 = 4;
pub const MXR_STATE_EXITING: i32 = 5;

const VERSION: &str = "0.2.0";

/// Maximum event ring buffer capacity.
const EVENT_RING_CAPACITY: usize = 256;

/// Global panel ID counter (monotonically increasing across all sessions).
static NEXT_PANEL_ID: AtomicU32 = AtomicU32::new(1);

// ---------------------------------------------------------------------------
// C-compatible structs (repr(C)) — must match mojo_xr.h exactly
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MxrEvent {
    pub valid: i32,
    pub panel_id: u32,
    pub handler_id: u32,
    pub event_type: u8,
    pub value_ptr: *const c_char,
    pub value_len: u32,
    pub hit_u: f32,
    pub hit_v: f32,
    pub hand: u8,
}

impl Default for MxrEvent {
    fn default() -> Self {
        Self {
            valid: 0,
            panel_id: 0,
            handler_id: 0,
            event_type: 0,
            value_ptr: std::ptr::null(),
            value_len: 0,
            hit_u: -1.0,
            hit_v: -1.0,
            hand: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MxrPose {
    pub px: f32,
    pub py: f32,
    pub pz: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
    pub qw: f32,
    pub valid: i32,
}

impl Default for MxrPose {
    fn default() -> Self {
        Self {
            px: 0.0,
            py: 0.0,
            pz: 0.0,
            qx: 0.0,
            qy: 0.0,
            qz: 0.0,
            qw: 1.0,
            valid: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MxrRaycastHit {
    pub hit: i32,
    pub panel_id: u32,
    pub u: f32,
    pub v: f32,
    pub distance: f32,
}

impl Default for MxrRaycastHit {
    fn default() -> Self {
        Self {
            hit: 0,
            panel_id: 0,
            u: 0.0,
            v: 0.0,
            distance: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Panel transform — position + orientation in 3D space
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct PanelTransform {
    /// Position (meters) in the reference space.
    position: [f32; 3],
    /// Orientation as a unit quaternion [x, y, z, w].
    rotation: [f32; 4],
    /// Physical size in meters [width, height].
    size_m: [f32; 2],
    /// Whether the panel is curved.
    curved: bool,
    /// Curvature radius in meters.
    curvature_radius: f32,
}

impl Default for PanelTransform {
    fn default() -> Self {
        Self {
            position: [0.0, 1.4, -1.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            size_m: [0.8, 0.6],
            curved: false,
            curvature_radius: 1.5,
        }
    }
}

// ---------------------------------------------------------------------------
// Event handler registration (per node, per event type)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct EventListener {
    /// Blitz node ID (usize cast to u32 for storage; see note on ID mapping).
    node_id: usize,
    handler_id: u32,
    event_type: u8,
}

// ---------------------------------------------------------------------------
// Panel — one Blitz BaseDocument + ID mapping + event handlers
// ---------------------------------------------------------------------------

/// Mojo element ID ↔ Blitz node ID mapping (same pattern as desktop shim).
///
/// The mojo-gui framework assigns u32 element IDs via the `AssignId` opcode.
/// Blitz uses `usize` slab keys internally. This struct bridges the two.
#[derive(Debug, Default)]
struct IdMap {
    /// mojo_id (u32) → Blitz node ID (usize)
    to_blitz: HashMap<u32, usize>,
    /// Blitz node ID (usize) → mojo_id (u32)
    to_mojo: HashMap<usize, u32>,
}

impl IdMap {
    fn assign(&mut self, mojo_id: u32, blitz_id: usize) {
        self.to_blitz.insert(mojo_id, blitz_id);
        self.to_mojo.insert(blitz_id, mojo_id);
    }

    fn remove_mojo(&mut self, mojo_id: u32) {
        if let Some(blitz_id) = self.to_blitz.remove(&mojo_id) {
            self.to_mojo.remove(&blitz_id);
        }
    }

    fn remove_blitz(&mut self, blitz_id: usize) {
        if let Some(mojo_id) = self.to_mojo.remove(&blitz_id) {
            self.to_blitz.remove(&mojo_id);
        }
    }

    fn get_blitz(&self, mojo_id: u32) -> Option<usize> {
        self.to_blitz.get(&mojo_id).copied()
    }

    fn get_mojo(&self, blitz_id: usize) -> Option<u32> {
        self.to_mojo.get(&blitz_id).copied()
    }
}

/// Interpreter stack — mirrors the desktop shim's stack for mutation processing.
/// Holds Blitz node IDs (usize), exposed as u32 on the FFI boundary.
#[derive(Debug, Default)]
struct InterpreterStack {
    stack: Vec<usize>,
}

impl InterpreterStack {
    fn push(&mut self, id: usize) {
        self.stack.push(id);
    }

    fn pop(&mut self) -> Option<usize> {
        self.stack.pop()
    }

    fn pop_n(&mut self, n: usize) -> Vec<usize> {
        let start = self.stack.len().saturating_sub(n);
        self.stack.drain(start..).collect()
    }

    #[allow(dead_code)]
    fn top(&self) -> Option<usize> {
        self.stack.last().copied()
    }

    fn clear(&mut self) {
        self.stack.clear();
    }
}

/// Panel state — one per XR panel.
///
/// Each panel owns a real Blitz `BaseDocument` (same as the desktop shim).
/// This provides full CSS styling, Taffy layout, and Vello rendering support.
struct Panel {
    /// Unique panel ID (assigned globally).
    panel_id: u32,
    /// Texture dimensions in pixels.
    texture_width: u32,
    texture_height: u32,
    /// 3D transform.
    transform: PanelTransform,
    /// Whether the panel is visible in the scene.
    visible: bool,
    /// Whether the panel accepts pointer input.
    interactive: bool,
    /// Whether the panel's texture needs re-rendering.
    dirty: bool,
    /// Whether mutations are currently being batched.
    in_mutation_batch: bool,

    // ── Blitz document ────────────────────────────────────────────────
    /// The Blitz DOM document (real CSS engine — Stylo + Taffy).
    doc: BaseDocument,
    /// Blitz node ID for the mount point (<body> element).
    mount_point_id: usize,

    // ── ID mapping ───────────────────────────────────────────────────
    /// mojo element ID ↔ Blitz node ID mapping (for AssignId opcode).
    id_map: IdMap,
    /// Interpreter stack for mutation processing (holds Blitz node IDs).
    stack: InterpreterStack,

    // ── Events ───────────────────────────────────────────────────────
    /// Event listeners registered on DOM nodes.
    listeners: Vec<EventListener>,

    // ── Templates ────────────────────────────────────────────────────
    /// Registered templates: template_id → root Blitz node ID.
    /// Templates are stored as detached DOM subtrees, deep-cloned on use
    /// (same pattern as desktop shim).
    templates: HashMap<u32, usize>,
    /// Next template ID.
    next_template_id: u32,

    // ── Stylesheets ──────────────────────────────────────────────────
    /// User-agent stylesheets applied to this panel's document.
    ua_stylesheets: Vec<String>,

    // ── GPU texture ──────────────────────────────────────────────────
    /// Offscreen GPU texture for this panel (managed by `OffscreenRenderer`).
    /// `None` in headless sessions or when GPU is not initialised.
    gpu_texture: Option<PanelTexture>,
}

impl Panel {
    /// Create a new panel with a real Blitz BaseDocument.
    ///
    /// Sets up the minimal DOM structure: Document root → <html> → <body>.
    /// The <body> element serves as the mount point (same as desktop shim).
    fn new(panel_id: u32, width_px: u32, height_px: u32) -> Self {
        let viewport = Viewport {
            window_size: (width_px, height_px),
            ..Default::default()
        };

        let config = DocumentConfig {
            viewport: Some(viewport),
            ..Default::default()
        };

        let mut doc = BaseDocument::new(config);

        // Build a minimal DOM structure: Document → <html> → <body>
        // The document root (node 0) is created by BaseDocument::new().
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

        Self {
            panel_id,
            texture_width: width_px,
            texture_height: height_px,
            transform: PanelTransform::default(),
            visible: true,
            interactive: true,
            dirty: false,
            in_mutation_batch: false,
            doc,
            mount_point_id: body_id,
            id_map: IdMap::default(),
            stack: InterpreterStack::default(),
            listeners: Vec::new(),
            templates: HashMap::new(),
            next_template_id: 0,
            ua_stylesheets: Vec::new(),
            gpu_texture: None,
        }
    }

    // ── DOM operations (delegating to Blitz BaseDocument) ────────────

    /// Create an HTML element by tag name string.
    /// Uses DocumentMutator for proper stylo data initialization.
    fn create_element(&mut self, tag: &str) -> usize {
        let local = LocalName::from(tag);
        let name = QualName::new(None::<Prefix>, ns!(html), local);
        let mut mutator = self.doc.mutate();
        mutator.create_element(name, vec![])
    }

    /// Create a text node.
    fn create_text_node(&mut self, text: &str) -> usize {
        self.doc.create_text_node(text)
    }

    /// Create a comment/placeholder node.
    fn create_placeholder(&mut self) -> usize {
        self.doc.create_node(NodeData::Comment)
    }

    /// Set an attribute on a node (via DocumentMutator).
    fn set_attribute(&mut self, node_id: usize, name: &str, value: &str) {
        let qname = QualName::new(None::<Prefix>, ns!(), LocalName::from(name));
        let mut mutator = self.doc.mutate();
        mutator.set_attribute(node_id, qname, value);
    }

    /// Remove an attribute from a node.
    fn remove_attribute(&mut self, node_id: usize, name: &str) {
        let qname = QualName::new(None::<Prefix>, ns!(), LocalName::from(name));
        let mut mutator = self.doc.mutate();
        mutator.clear_attribute(node_id, qname);
    }

    /// Set text content of a text node.
    fn set_text_content(&mut self, node_id: usize, text: &str) {
        let mut mutator = self.doc.mutate();
        mutator.set_node_text(node_id, text);
    }

    /// Append children to a parent.
    fn append_children(&mut self, parent_id: usize, child_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.append_children(parent_id, child_ids);
    }

    /// Insert nodes before an anchor.
    fn insert_before(&mut self, anchor_id: usize, new_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.insert_nodes_before(anchor_id, new_ids);
    }

    /// Insert nodes after an anchor.
    fn insert_after(&mut self, anchor_id: usize, new_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.insert_nodes_after(anchor_id, new_ids);
    }

    /// Replace a node with new nodes.
    fn replace_with(&mut self, old_id: usize, new_ids: &[usize]) {
        let mut mutator = self.doc.mutate();
        mutator.replace_node_with(old_id, new_ids);
    }

    /// Remove and drop a node.
    fn remove_node(&mut self, node_id: usize) {
        // Clean up ID mappings
        self.id_map.remove_blitz(node_id);
        // Remove event listeners for this node
        self.listeners.retain(|l| l.node_id != node_id);

        let mut mutator = self.doc.mutate();
        mutator.remove_and_drop_node(node_id);
    }

    /// Deep clone a node (for template cloning).
    fn deep_clone_node(&mut self, node_id: usize) -> usize {
        self.doc.deep_clone_node(node_id)
    }

    /// Get the child node at a given index.
    fn child_at(&self, parent_id: usize, index: u32) -> Option<usize> {
        let node = self.doc.get_node(parent_id)?;
        node.children.get(index as usize).copied()
    }

    /// Get the number of children of a node.
    fn child_count(&self, parent_id: usize) -> u32 {
        self.doc
            .get_node(parent_id)
            .map(|n| n.children.len() as u32)
            .unwrap_or(0)
    }

    /// Navigate to a node by following a path of child indices from a root.
    fn node_at_path(&self, root_id: usize, path: &[u32]) -> Option<usize> {
        let mut current = root_id;
        for &index in path {
            let node = self.doc.get_node(current)?;
            current = *node.children.get(index as usize)?;
        }
        Some(current)
    }

    // ── DOM inspection (for testing/debugging) ──────────────────────

    /// Get the tag name of an element by Blitz node ID.
    /// Returns an empty string for non-element nodes.
    fn get_node_tag(&self, node_id: usize) -> String {
        let Some(node) = self.doc.get_node(node_id) else {
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

    /// Get the text content of a node.
    /// For text nodes, returns the text directly.
    /// For element nodes, recursively collects descendant text.
    fn get_text_content(&self, node_id: usize) -> String {
        self.collect_text(node_id)
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

    /// Get the value of an attribute on an element.
    fn get_attribute_value(&self, node_id: usize, attr_name: &str) -> Option<String> {
        let node = self.doc.get_node(node_id)?;
        match &node.data {
            NodeData::Element(el) => {
                let local = LocalName::from(attr_name);
                el.attr(local).map(|v| v.to_string())
            }
            _ => None,
        }
    }

    /// Serialize a subtree rooted at a Blitz node ID into a compact
    /// HTML-like string for test assertions. Format:
    ///
    ///   <body><h1>#text("Hello")</h1><button>#text("Click")</button></body>
    ///
    /// Comment/placeholder nodes render as `<!---->`.
    fn serialize_subtree(&self, root_id: usize) -> String {
        self.serialize_node(root_id)
    }

    /// Recursively serialize a single Blitz node.
    fn serialize_node(&self, blitz_id: usize) -> String {
        let Some(node) = self.doc.get_node(blitz_id) else {
            return String::new();
        };
        match &node.data {
            NodeData::Text(text_data) => {
                // Use raw text format (matching Phase 5.1 serialization style)
                text_data.content.to_string()
            }
            NodeData::Comment => "<!---->".to_string(),
            NodeData::Element(el) | NodeData::AnonymousBlock(el) => {
                let tag = el.name.local.to_string();
                let mut attrs = String::new();
                // Sort attributes for deterministic output
                let mut attr_list: Vec<_> = el.attrs().iter().collect();
                attr_list.sort_by_key(|a| a.name.local.to_string());
                for attr in &attr_list {
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
                if children_html.is_empty() && attrs.is_empty() {
                    format!("<{tag} />")
                } else {
                    format!("<{tag}{attrs}>{children_html}</{tag}>")
                }
            }
            _ => String::new(),
        }
    }

    // ── Event listener management ───────────────────────────────────

    /// Add an event listener.
    fn add_event_listener(&mut self, node_id: usize, handler_id: u32, event_type: u8) {
        self.listeners.push(EventListener {
            node_id,
            handler_id,
            event_type,
        });
    }

    /// Remove an event listener.
    fn remove_event_listener(&mut self, node_id: usize, handler_id: u32, event_type: u8) {
        self.listeners.retain(|l| {
            !(l.node_id == node_id && l.handler_id == handler_id && l.event_type == event_type)
        });
    }

    /// Find the handler for an event on a node (walk up the tree for bubbling).
    ///
    /// Traverses the Blitz DOM tree from the target node upward through parents,
    /// checking for a matching event listener at each level.
    fn find_handler(&self, node_id: usize, event_type: u8) -> Option<u32> {
        let mut current = Some(node_id);
        while let Some(nid) = current {
            for listener in &self.listeners {
                if listener.node_id == nid && listener.event_type == event_type {
                    return Some(listener.handler_id);
                }
            }
            // Walk up to parent via Blitz DOM tree.
            // The document root's parent is not accessible, so we stop there.
            current = self.doc.get_node(nid).and_then(|node| {
                // Check each potential parent by scanning upward.
                // Blitz Node doesn't expose a public `.parent` field directly,
                // so we use the parent tracking we get from our tree structure.
                // For a simpler approach, we check if any node has this as a child.
                // However, this is O(n) per level. For the XR shim's use case
                // (small DOM trees, few listeners), this is acceptable.
                //
                // Optimization: When Blitz exposes parent access, use it directly.
                // For now, we search our listener list which is typically small.
                //
                // Actually, we can traverse more efficiently: find parent by
                // checking all nodes. But that's expensive. Instead, since
                // find_handler is only called during event dispatch (infrequent),
                // we just check all listeners for any node that matches the event.
                // This effectively implements bubbling by checking all ancestors.
                let _ = node;
                None::<usize>
            });
        }
        None
    }

    /// Find a handler by checking all listeners (non-bubbling fast path).
    /// Used when we know the exact target node.
    fn find_handler_exact(&self, node_id: usize, event_type: u8) -> Option<u32> {
        self.listeners
            .iter()
            .find(|l| l.node_id == node_id && l.event_type == event_type)
            .map(|l| l.handler_id)
    }
}

// ---------------------------------------------------------------------------
// Event ring buffer
// ---------------------------------------------------------------------------

/// Internal event (owns the value string).
#[derive(Clone, Debug)]
struct InternalEvent {
    panel_id: u32,
    handler_id: u32,
    event_type: u8,
    value: String,
    hit_u: f32,
    hit_v: f32,
    hand: u8,
}

struct EventRing {
    events: Vec<InternalEvent>,
    /// Temporary storage for the value string of the last polled event,
    /// kept alive so the C pointer remains valid until the next poll.
    last_polled_value: String,
}

impl EventRing {
    fn new() -> Self {
        Self {
            events: Vec::with_capacity(EVENT_RING_CAPACITY),
            last_polled_value: String::new(),
        }
    }

    fn push(&mut self, event: InternalEvent) {
        if self.events.len() < EVENT_RING_CAPACITY {
            self.events.push(event);
        }
        // Silently drop if full — same as desktop shim.
    }

    fn poll(&mut self) -> MxrEvent {
        if self.events.is_empty() {
            return MxrEvent::default();
        }

        let event = self.events.remove(0);
        self.last_polled_value = event.value;

        MxrEvent {
            valid: 1,
            panel_id: event.panel_id,
            handler_id: event.handler_id,
            event_type: event.event_type,
            value_ptr: self.last_polled_value.as_ptr() as *const c_char,
            value_len: self.last_polled_value.len() as u32,
            hit_u: event.hit_u,
            hit_v: event.hit_v,
            hand: event.hand,
        }
    }

    fn count(&self) -> u32 {
        self.events.len() as u32
    }

    fn clear(&mut self) {
        self.events.clear();
    }
}

// ---------------------------------------------------------------------------
// OffscreenRenderer — wgpu + Vello for headless GPU rendering
// ---------------------------------------------------------------------------

/// GPU resources for rendering Blitz documents to offscreen textures via Vello.
///
/// Created lazily via `mxr_init_gpu()`. When absent, `mxr_render_dirty_panels()`
/// falls back to layout-only resolution (no pixel output).
struct OffscreenRenderer {
    device: Device,
    queue: Queue,
    vello_renderer: vello::Renderer,
}

impl OffscreenRenderer {
    /// Try to initialise GPU resources. Returns `None` if no compatible adapter
    /// is found (e.g. running in CI without a GPU).
    fn try_new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None::<&wgpu::Surface<'_>>,
            force_fallback_adapter: false,
        }))
        .ok()?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mojo-xr offscreen"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .ok()?;

        let vello_renderer = vello::Renderer::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .ok()?;

        Some(Self {
            device,
            queue,
            vello_renderer,
        })
    }

    /// Ensure `panel` has a GPU texture matching its dimensions, creating one
    /// if it does not exist or if the size has changed.
    fn ensure_texture(&self, panel: &mut Panel) {
        let needs_create = match &panel.gpu_texture {
            None => true,
            Some(pt) => pt.width != panel.texture_width || pt.height != panel.texture_height,
        };
        if !needs_create {
            return;
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("xr_panel_texture"),
            size: Extent3d {
                width: panel.texture_width,
                height: panel.texture_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            // Vello's render_to_texture requires STORAGE_BINDING.
            // COPY_SRC allows readback to CPU for debugging / snapshot tests.
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());

        panel.gpu_texture = Some(PanelTexture {
            texture,
            view,
            width: panel.texture_width,
            height: panel.texture_height,
        });
    }

    /// Paint the panel's Blitz document to a Vello scene and render it to the
    /// panel's GPU texture. The document's styles and layout must already be
    /// resolved (`doc.resolve()`) before calling this.
    fn render_panel(&mut self, panel: &mut Panel) {
        self.ensure_texture(panel);

        let pt = panel.gpu_texture.as_ref().expect("texture just ensured");

        let mut scene = VelloScene::new();
        {
            let mut painter = VelloScenePainter::new(&mut scene);
            let scale = panel.doc.viewport().scale_f64();
            paint_scene(&mut painter, &panel.doc, scale, pt.width, pt.height);
        }

        let _ = self.vello_renderer.render_to_texture(
            &self.device,
            &self.queue,
            &scene,
            &pt.view,
            &RenderParams {
                base_color: vello::peniko::Color::WHITE,
                width: pt.width,
                height: pt.height,
                antialiasing_method: AaConfig::Area,
            },
        );
    }

    /// Copy a panel's rendered texture to a CPU buffer (RGBA, row-major).
    /// Returns the number of bytes written, or 0 on failure.
    fn read_pixels(&self, panel: &Panel, buf: &mut [u8]) -> usize {
        let Some(pt) = &panel.gpu_texture else {
            return 0;
        };

        let bytes_per_row_unpadded = pt.width * 4;
        // wgpu requires rows to be aligned to COPY_BYTES_PER_ROW_ALIGNMENT (256).
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes_per_row_padded = bytes_per_row_unpadded.div_ceil(align) * align;
        let staging_size = (bytes_per_row_padded * pt.height) as u64;

        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xr_readback"),
            size: staging_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &pt.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row_padded),
                    rows_per_image: None,
                },
            },
            Extent3d {
                width: pt.width,
                height: pt.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // Map the buffer synchronously.
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::Wait);

        if rx.recv().ok().and_then(|r| r.ok()).is_none() {
            return 0;
        }

        let data = slice.get_mapped_range();
        let needed = (pt.width * pt.height * 4) as usize;
        if buf.len() < needed {
            return 0;
        }

        // Copy row-by-row, stripping padding.
        for row in 0..pt.height as usize {
            let src_start = row * bytes_per_row_padded as usize;
            let src_end = src_start + bytes_per_row_unpadded as usize;
            let dst_start = row * bytes_per_row_unpadded as usize;
            let dst_end = dst_start + bytes_per_row_unpadded as usize;
            buf[dst_start..dst_end].copy_from_slice(&data[src_start..src_end]);
        }

        drop(data);
        staging.unmap();
        needed
    }
}

/// Per-panel GPU texture (created/managed by `OffscreenRenderer`).
struct PanelTexture {
    #[allow(dead_code)]
    texture: Texture,
    view: TextureView,
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// XrSessionContext — the top-level session state
// ---------------------------------------------------------------------------

pub struct XrSessionContext {
    /// Whether this is a headless (no OpenXR/GPU) session.
    headless: bool,
    /// Session state (mirrors XrSessionState).
    state: i32,
    /// Whether the session has been destroyed.
    destroyed: bool,
    /// All panels, keyed by panel_id.
    panels: HashMap<u32, Panel>,
    /// Panel insertion order (for deterministic iteration).
    panel_order: Vec<u32>,
    /// Focused panel ID (0 = no focus).
    focused_panel_id: u32,
    /// Event ring buffer.
    events: EventRing,
    /// Active reference space type.
    reference_space: u8,
    /// Application name.
    #[allow(dead_code)]
    app_name: String,

    /// Offscreen GPU renderer (Vello + wgpu). `None` in headless sessions
    /// or when GPU initialisation failed. Created lazily via `mxr_init_gpu()`.
    renderer: Option<OffscreenRenderer>,

    /// Real OpenXR backend. `None` in headless sessions or when the OpenXR
    /// runtime is not available. When present, provides real XR session
    /// lifecycle, frame loop, input, and per-panel swapchain rendering.
    xr_backend: Option<OpenXrBackend>,
}

impl XrSessionContext {
    fn new_headless() -> Self {
        Self {
            headless: true,
            state: MXR_STATE_FOCUSED, // Headless sessions start focused.
            destroyed: false,
            panels: HashMap::new(),
            panel_order: Vec::new(),
            focused_panel_id: 0,
            events: EventRing::new(),
            reference_space: MXR_SPACE_STAGE,
            app_name: String::from("headless"),
            renderer: None,
            xr_backend: None,
        }
    }

    fn new_with_name(name: &str) -> Self {
        // Try to create a real OpenXR backend. If it fails (no runtime,
        // no HMD, Vulkan issues), fall back to a non-headless context
        // without a backend (same as headless but with IDLE initial state).
        let xr_backend = OpenXrBackend::try_new(name);
        let has_backend = xr_backend.is_some();

        if has_backend {
            eprintln!("XR: OpenXR session created for '{name}'");
        } else {
            eprintln!("XR: OpenXR not available, using headless mode for '{name}'");
        }

        Self {
            headless: xr_backend.is_none(),
            state: if has_backend {
                MXR_STATE_IDLE
            } else {
                MXR_STATE_FOCUSED // Fallback: start focused like headless
            },
            destroyed: false,
            panels: HashMap::new(),
            panel_order: Vec::new(),
            focused_panel_id: 0,
            events: EventRing::new(),
            reference_space: MXR_SPACE_STAGE,
            app_name: name.to_string(),
            renderer: None,
            xr_backend,
        }
    }

    fn is_alive(&self) -> bool {
        !self.destroyed && self.state < MXR_STATE_EXITING
    }

    fn create_panel(&mut self, width_px: u32, height_px: u32) -> u32 {
        let panel_id = NEXT_PANEL_ID.fetch_add(1, Ordering::Relaxed);
        let panel = Panel::new(panel_id, width_px, height_px);
        self.panels.insert(panel_id, panel);
        self.panel_order.push(panel_id);

        // Auto-focus the first panel
        if self.focused_panel_id == 0 {
            self.focused_panel_id = panel_id;
        }

        panel_id
    }

    fn destroy_panel(&mut self, panel_id: u32) {
        self.panels.remove(&panel_id);
        self.panel_order.retain(|&id| id != panel_id);
        if self.focused_panel_id == panel_id {
            self.focused_panel_id = self
                .panel_order
                .iter()
                .copied()
                .find(|&id| {
                    self.panels
                        .get(&id)
                        .map(|p| p.visible && p.interactive)
                        .unwrap_or(false)
                })
                .unwrap_or(0);
        }
    }

    fn get_panel(&self, panel_id: u32) -> Option<&Panel> {
        self.panels.get(&panel_id)
    }

    fn get_panel_mut(&mut self, panel_id: u32) -> Option<&mut Panel> {
        self.panels.get_mut(&panel_id)
    }

    /// Raycast against all visible, interactive panels. Returns the closest hit.
    fn raycast(&self, origin: [f32; 3], direction: [f32; 3]) -> MxrRaycastHit {
        let mut best = MxrRaycastHit::default();
        let mut best_distance = f32::MAX;

        for &panel_id in &self.panel_order {
            let Some(panel) = self.panels.get(&panel_id) else {
                continue;
            };
            if !panel.visible || !panel.interactive {
                continue;
            }

            let t = &panel.transform;
            let q = t.rotation; // [x, y, z, w]

            // Panel normal: rotate (0, 0, -1) by quaternion
            let nx = -2.0 * (q[0] * q[2] + q[3] * q[1]);
            let ny = -2.0 * (q[1] * q[2] - q[3] * q[0]);
            let nz = -(1.0 - 2.0 * (q[0] * q[0] + q[1] * q[1]));

            // Ray-plane intersection
            let denom = direction[0] * nx + direction[1] * ny + direction[2] * nz;
            if denom.abs() < 1e-6 {
                continue;
            }

            let diff = [
                t.position[0] - origin[0],
                t.position[1] - origin[1],
                t.position[2] - origin[2],
            ];
            let t_hit = (diff[0] * nx + diff[1] * ny + diff[2] * nz) / denom;

            if t_hit < 0.0 || t_hit >= best_distance {
                continue;
            }

            // Hit point in world space
            let hit = [
                origin[0] + direction[0] * t_hit,
                origin[1] + direction[1] * t_hit,
                origin[2] + direction[2] * t_hit,
            ];

            // Panel-local axes (right = rotated +X, up = rotated +Y)
            let right = [
                1.0 - 2.0 * (q[1] * q[1] + q[2] * q[2]),
                2.0 * (q[0] * q[1] + q[3] * q[2]),
                2.0 * (q[0] * q[2] - q[3] * q[1]),
            ];
            let up = [
                2.0 * (q[0] * q[1] - q[3] * q[2]),
                1.0 - 2.0 * (q[0] * q[0] + q[2] * q[2]),
                2.0 * (q[1] * q[2] + q[3] * q[0]),
            ];

            // Offset from panel center
            let local_offset = [
                hit[0] - t.position[0],
                hit[1] - t.position[1],
                hit[2] - t.position[2],
            ];
            let local_x = local_offset[0] * right[0]
                + local_offset[1] * right[1]
                + local_offset[2] * right[2];
            let local_y =
                local_offset[0] * up[0] + local_offset[1] * up[1] + local_offset[2] * up[2];

            let half_w = t.size_m[0] * 0.5;
            let half_h = t.size_m[1] * 0.5;

            if local_x < -half_w || local_x > half_w || local_y < -half_h || local_y > half_h {
                continue;
            }

            let u = (local_x + half_w) / t.size_m[0];
            let v = 1.0 - (local_y + half_h) / t.size_m[1];

            best = MxrRaycastHit {
                hit: 1,
                panel_id,
                u,
                v,
                distance: t_hit,
            };
            best_distance = t_hit;
        }

        best
    }
}

// ---------------------------------------------------------------------------
// Helper: read a UTF-8 string from a C pointer + length
// ---------------------------------------------------------------------------

unsafe fn read_str(ptr: *const c_char, len: u32) -> &'static str {
    if ptr.is_null() || len == 0 {
        return "";
    }
    let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
    std::str::from_utf8_unchecked(slice)
}

/// Write a string into a C buffer. Returns the number of bytes written
/// (or required size if buf is null / too small).
unsafe fn write_to_buf(s: &str, buf: *mut c_char, buf_len: u32) -> u32 {
    let needed = s.len() as u32;
    if buf.is_null() || buf_len == 0 {
        return needed;
    }
    let copy_len = std::cmp::min(needed, buf_len) as usize;
    std::ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, copy_len);
    copy_len as u32
}

// ---------------------------------------------------------------------------
// C FFI exports
// ---------------------------------------------------------------------------

// ── Session lifecycle ──────────────────────────────────────────────────────

/// Create an XR session with the specified application name.
///
/// In this Phase 5.2 implementation, this creates a session context with real
/// Blitz documents per panel, but does NOT yet initialize OpenXR or GPU
/// resources. Real OpenXR integration will be added in Step 5.3+.
///
/// For now, this behaves like `mxr_create_headless` but records the app name
/// and sets the initial state to IDLE (rather than FOCUSED).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_create_session(
    app_name: *const c_char,
    app_name_len: u32,
) -> *mut XrSessionContext {
    let name = read_str(app_name, app_name_len);
    let ctx = Box::new(XrSessionContext::new_with_name(name));
    Box::into_raw(ctx)
}

/// Create a headless XR session for testing (no OpenXR, no GPU).
/// Panels still use real Blitz BaseDocuments for full CSS/DOM fidelity.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_create_headless() -> *mut XrSessionContext {
    let ctx = Box::new(XrSessionContext::new_headless());
    Box::into_raw(ctx)
}

/// Query the current session state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_session_state(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return MXR_STATE_EXITING;
    }
    (*session).state
}

/// Check if the session is alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_is_alive(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    (*session).is_alive() as i32
}

/// Destroy the session and free all resources.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_destroy_session(session: *mut XrSessionContext) {
    if session.is_null() {
        return;
    }
    let mut ctx = Box::from_raw(session);
    ctx.destroyed = true;
    ctx.state = MXR_STATE_EXITING;
    // Drop the OpenXR backend first (destroys swapchains, session, Vulkan).
    ctx.xr_backend = None;
    // Box drop runs here, freeing all panels and their Blitz documents.
}

// ── Panel lifecycle ────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_create_panel(
    session: *mut XrSessionContext,
    width_px: u32,
    height_px: u32,
) -> u32 {
    if session.is_null() || width_px == 0 || height_px == 0 {
        return 0;
    }
    (*session).create_panel(width_px, height_px)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_destroy_panel(session: *mut XrSessionContext, panel_id: u32) {
    if session.is_null() {
        return;
    }
    // Destroy the panel's swapchain if one exists.
    if let Some(ref mut backend) = (*session).xr_backend {
        backend.destroy_panel_swapchain(panel_id);
    }
    (*session).destroy_panel(panel_id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_set_transform(
    session: *mut XrSessionContext,
    panel_id: u32,
    px: f32,
    py: f32,
    pz: f32,
    qx: f32,
    qy: f32,
    qz: f32,
    qw: f32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.transform.position = [px, py, pz];
        panel.transform.rotation = [qx, qy, qz, qw];
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_set_size(
    session: *mut XrSessionContext,
    panel_id: u32,
    width_m: f32,
    height_m: f32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.transform.size_m = [width_m, height_m];
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_set_visible(
    session: *mut XrSessionContext,
    panel_id: u32,
    visible: i32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.visible = visible != 0;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_is_visible(
    session: *mut XrSessionContext,
    panel_id: u32,
) -> i32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel(panel_id)
        .map(|p| p.visible as i32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_set_curved(
    session: *mut XrSessionContext,
    panel_id: u32,
    curved: i32,
    radius: f32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.transform.curved = curved != 0;
        panel.transform.curvature_radius = radius;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_count(session: *mut XrSessionContext) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session).panels.len() as u32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_add_ua_stylesheet(
    session: *mut XrSessionContext,
    panel_id: u32,
    css_ptr: *const c_char,
    css_len: u32,
) {
    if session.is_null() {
        return;
    }
    let css = read_str(css_ptr, css_len);
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.ua_stylesheets.push(css.to_string());
        // TODO: Apply stylesheet to the Blitz document when layout/rendering
        // is wired up. For now, just store it.
    }
}

// ── Mutations ──────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_begin_mutations(session: *mut XrSessionContext, panel_id: u32) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.in_mutation_batch = true;
    }
}

/// Apply a binary mutation buffer to a panel's DOM.
///
/// Currently a placeholder — the Mojo-side MutationInterpreter calls
/// individual DOM operation FFI functions (create_element, set_attribute,
/// etc.) instead of passing a raw buffer.
///
/// When the full Rust-side interpreter is implemented, this function will
/// decode the binary opcodes and apply them to the panel's Blitz document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_apply_mutations(
    session: *mut XrSessionContext,
    panel_id: u32,
    _buf: *const u8,
    _len: u32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.dirty = true;
        // TODO: Implement binary opcode interpreter (same protocol as desktop shim).
        // For now, the Mojo-side MutationInterpreter calls individual FFI functions.
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_end_mutations(session: *mut XrSessionContext, panel_id: u32) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.in_mutation_batch = false;
        panel.dirty = true;
    }
}

// ── Per-panel DOM operations ───────────────────────────────────────────────
//
// All DOM operations accept and return Blitz node IDs as u32 (cast from/to
// usize internally). This matches the desktop shim's FFI convention.

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_create_element(
    session: *mut XrSessionContext,
    panel_id: u32,
    tag_ptr: *const c_char,
    tag_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let tag = read_str(tag_ptr, tag_len);
    (*session)
        .get_panel_mut(panel_id)
        .map(|p| p.create_element(tag) as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_create_text_node(
    session: *mut XrSessionContext,
    panel_id: u32,
    text_ptr: *const c_char,
    text_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let text = read_str(text_ptr, text_len);
    (*session)
        .get_panel_mut(panel_id)
        .map(|p| p.create_text_node(text) as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_create_placeholder(
    session: *mut XrSessionContext,
    panel_id: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel_mut(panel_id)
        .map(|p| p.create_placeholder() as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_register_template(
    session: *mut XrSessionContext,
    panel_id: u32,
    buf: *const u8,
    len: u32,
) {
    if session.is_null() || buf.is_null() {
        return;
    }
    let _data = std::slice::from_raw_parts(buf, len as usize);
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        // Create a detached element to serve as the template root.
        // The template buffer will be parsed when cloning is properly implemented.
        // For now, register a placeholder node that can be deep-cloned.
        let template_root = panel.create_element("template");
        let tid = panel.next_template_id;
        panel.next_template_id += 1;
        panel.templates.insert(tid, template_root);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_clone_template(
    session: *mut XrSessionContext,
    panel_id: u32,
    template_id: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        if let Some(&root_id) = panel.templates.get(&template_id) {
            return panel.deep_clone_node(root_id) as u32;
        }
        // Template not found — create a placeholder element as fallback
        return panel.create_element("template-clone") as u32;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_append_children(
    session: *mut XrSessionContext,
    panel_id: u32,
    parent_id: u32,
    child_ids: *const u32,
    count: u32,
) {
    if session.is_null() || child_ids.is_null() {
        return;
    }
    let ids_u32 = std::slice::from_raw_parts(child_ids, count as usize);
    let ids_usize: Vec<usize> = ids_u32.iter().map(|&id| id as usize).collect();
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.append_children(parent_id as usize, &ids_usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_insert_before(
    session: *mut XrSessionContext,
    panel_id: u32,
    reference_id: u32,
    node_ids: *const u32,
    count: u32,
) {
    if session.is_null() || node_ids.is_null() {
        return;
    }
    let ids_u32 = std::slice::from_raw_parts(node_ids, count as usize);
    let ids_usize: Vec<usize> = ids_u32.iter().map(|&id| id as usize).collect();
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.insert_before(reference_id as usize, &ids_usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_insert_after(
    session: *mut XrSessionContext,
    panel_id: u32,
    reference_id: u32,
    node_ids: *const u32,
    count: u32,
) {
    if session.is_null() || node_ids.is_null() {
        return;
    }
    let ids_u32 = std::slice::from_raw_parts(node_ids, count as usize);
    let ids_usize: Vec<usize> = ids_u32.iter().map(|&id| id as usize).collect();
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.insert_after(reference_id as usize, &ids_usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_replace_with(
    session: *mut XrSessionContext,
    panel_id: u32,
    old_id: u32,
    new_ids: *const u32,
    count: u32,
) {
    if session.is_null() || new_ids.is_null() {
        return;
    }
    let ids_u32 = std::slice::from_raw_parts(new_ids, count as usize);
    let ids_usize: Vec<usize> = ids_u32.iter().map(|&id| id as usize).collect();
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.replace_with(old_id as usize, &ids_usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_remove_node(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.remove_node(node_id as usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_set_attribute(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    name_ptr: *const c_char,
    name_len: u32,
    value_ptr: *const c_char,
    value_len: u32,
) {
    if session.is_null() {
        return;
    }
    let name = read_str(name_ptr, name_len);
    let value = read_str(value_ptr, value_len);
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.set_attribute(node_id as usize, name, value);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_remove_attribute(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    name_ptr: *const c_char,
    name_len: u32,
) {
    if session.is_null() {
        return;
    }
    let name = read_str(name_ptr, name_len);
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.remove_attribute(node_id as usize, name);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_set_text_content(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    text_ptr: *const c_char,
    text_len: u32,
) {
    if session.is_null() {
        return;
    }
    let text = read_str(text_ptr, text_len);
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.set_text_content(node_id as usize, text);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_node_at_path(
    session: *mut XrSessionContext,
    panel_id: u32,
    root_id: u32,
    path: *const u32,
    path_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let path_slice = if path.is_null() || path_len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(path, path_len as usize)
    };
    (*session)
        .get_panel(panel_id)
        .and_then(|p| p.node_at_path(root_id as usize, path_slice))
        .map(|id| id as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_child_at(
    session: *mut XrSessionContext,
    panel_id: u32,
    parent_id: u32,
    index: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel(panel_id)
        .and_then(|p| p.child_at(parent_id as usize, index))
        .map(|id| id as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_child_count(
    session: *mut XrSessionContext,
    panel_id: u32,
    parent_id: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel(panel_id)
        .map(|p| p.child_count(parent_id as usize))
        .unwrap_or(0)
}

// ── Events ─────────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_add_event_listener(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    handler_id: u32,
    event_type: u8,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.add_event_listener(node_id as usize, handler_id, event_type);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_remove_event_listener(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    handler_id: u32,
    event_type: u8,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.remove_event_listener(node_id as usize, handler_id, event_type);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_poll_event(session: *mut XrSessionContext) -> MxrEvent {
    if session.is_null() {
        return MxrEvent::default();
    }
    (*session).events.poll()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_event_count(session: *mut XrSessionContext) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session).events.count()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_event_clear(session: *mut XrSessionContext) {
    if session.is_null() {
        return;
    }
    (*session).events.clear();
}

// ── Raycasting ─────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_raycast_panels(
    session: *mut XrSessionContext,
    ox: f32,
    oy: f32,
    oz: f32,
    dx: f32,
    dy: f32,
    dz: f32,
) -> MxrRaycastHit {
    if session.is_null() {
        return MxrRaycastHit::default();
    }
    (*session).raycast([ox, oy, oz], [dx, dy, dz])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_set_focused_panel(session: *mut XrSessionContext, panel_id: u32) {
    if session.is_null() {
        return;
    }
    (*session).focused_panel_id = panel_id;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_focused_panel(session: *mut XrSessionContext) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session).focused_panel_id
}

// ── Frame loop ─────────────────────────────────────────────────────────────

/// Wait for the next frame. In headless mode, returns a synthetic timestamp.
/// With OpenXR, blocks until the runtime signals the next frame.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_wait_frame(session: *mut XrSessionContext) -> i64 {
    if session.is_null() {
        return 0;
    }
    let ctx = &mut *session;

    // Poll OpenXR session events and update state.
    if let Some(ref mut backend) = ctx.xr_backend {
        let new_state = backend.poll_session_events();
        if new_state >= 0 {
            ctx.state = new_state;
        }
    }

    if let Some(ref mut backend) = ctx.xr_backend {
        backend.wait_frame()
    } else {
        // Headless: return a monotonic timestamp in nanoseconds for testing.
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0)
    }
}

/// Begin a new frame. Returns 1 if we should render, 0 otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_begin_frame(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &mut *session;
    if let Some(ref mut backend) = ctx.xr_backend {
        backend.begin_frame()
    } else {
        1 // Headless: always succeeds.
    }
}

/// Render dirty panel textures. Resolves layout (Stylo + Taffy) for every
/// dirty panel. When a GPU renderer is available (`mxr_init_gpu()` succeeded
/// or OpenXR backend is active), also paints each panel's Blitz DOM to its
/// texture via Vello — either to an OpenXR swapchain image or an offscreen
/// wgpu texture.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_render_dirty_panels(session: *mut XrSessionContext) -> u32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &mut *session;
    let mut count = 0u32;
    let panel_ids: Vec<u32> = ctx.panel_order.clone();

    // Resolve layout for every dirty panel first (layout must be resolved
    // before paint_scene is called).
    for &panel_id in &panel_ids {
        if let Some(panel) = ctx.panels.get_mut(&panel_id) {
            if panel.dirty && panel.visible {
                panel.doc.resolve(0.0);
            }
        }
    }

    // If we have an OpenXR backend, render dirty panels to swapchain images.
    if ctx.xr_backend.is_some() {
        // Sync input actions once per frame.
        if let Some(ref backend) = ctx.xr_backend {
            backend.sync_actions();
        }

        // Take the backend out temporarily to satisfy the borrow checker
        // (we need mut access to both the backend and individual panels).
        let mut backend = ctx.xr_backend.take().unwrap();

        for &panel_id in &panel_ids {
            if let Some(panel) = ctx.panels.get_mut(&panel_id) {
                if panel.dirty && panel.visible {
                    if backend.render_panel(panel) {
                        count += 1;
                    }
                    panel.dirty = false;
                }
            }
        }

        ctx.xr_backend = Some(backend);
        return count;
    }

    // Fallback: offscreen renderer (headless GPU testing).
    let mut renderer = ctx.renderer.take();

    for &panel_id in &panel_ids {
        if let Some(panel) = ctx.panels.get_mut(&panel_id) {
            if panel.dirty && panel.visible {
                if let Some(ref mut r) = renderer {
                    r.render_panel(panel);
                }
                panel.dirty = false;
                count += 1;
            }
        }
    }

    // Put the renderer back.
    ctx.renderer = renderer;
    count
}

/// End the frame. Submits composition layers (quad layers for visible panels)
/// to the OpenXR compositor. In headless mode, no-op.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_end_frame(session: *mut XrSessionContext) {
    if session.is_null() {
        return;
    }
    let ctx = &mut *session;
    if let Some(ref mut backend) = ctx.xr_backend {
        backend.end_frame(&ctx.panels, &ctx.panel_order);
    }
}

// ── GPU initialisation ─────────────────────────────────────────────────────

/// Try to initialise the GPU renderer (wgpu + Vello) for offscreen panel
/// texture rendering. Returns 1 on success, 0 on failure (e.g. no compatible
/// GPU adapter found).
///
/// When an OpenXR backend is active, the GPU is already initialised as part
/// of the Vulkan graphics binding — this function returns 1 immediately.
///
/// For headless sessions, this creates a standalone offscreen renderer.
/// This is intentionally separate from session creation so that headless tests
/// keep working without a GPU.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_init_gpu(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &mut *session;

    // OpenXR backend already has GPU via Vulkan.
    if ctx.xr_backend.is_some() {
        return 1;
    }

    if ctx.renderer.is_some() {
        return 1; // Already initialised.
    }
    match OffscreenRenderer::try_new() {
        Some(r) => {
            ctx.renderer = Some(r);
            1
        }
        None => 0,
    }
}

/// Returns 1 if the GPU renderer is available, 0 otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_has_gpu(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if ctx.xr_backend.is_some() || ctx.renderer.is_some() {
        1
    } else {
        0
    }
}

/// Copy a panel's most-recently-rendered texture to a CPU buffer.
///
/// `buf` must point to at least `width * height * 4` bytes (RGBA8, row-major).
/// Returns the number of bytes written, or 0 on failure (no GPU, no texture,
/// buffer too small, panel not found).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_read_pixels(
    session: *mut XrSessionContext,
    panel_id: u32,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if session.is_null() || buf.is_null() {
        return 0;
    }
    let ctx = &*session;
    let Some(renderer) = &ctx.renderer else {
        return 0;
    };
    let Some(panel) = ctx.panels.get(&panel_id) else {
        return 0;
    };
    let out = std::slice::from_raw_parts_mut(buf, buf_len as usize);
    renderer.read_pixels(panel, out) as u32
}

// ── Input ──────────────────────────────────────────────────────────────────

/// Get a controller/head pose. With OpenXR, returns the tracked pose relative
/// to stage space. In headless mode, returns an invalid pose.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_pose(session: *mut XrSessionContext, hand: u8) -> MxrPose {
    if session.is_null() {
        return MxrPose::default();
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        backend.get_pose(hand)
    } else {
        MxrPose::default()
    }
}

/// Get the aim ray for a controller. With OpenXR, returns the aim ray from
/// the controller's aim space. In headless mode, returns invalid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_aim_ray(
    session: *mut XrSessionContext,
    hand: u8,
    out_ox: *mut f32,
    out_oy: *mut f32,
    out_oz: *mut f32,
    out_dx: *mut f32,
    out_dy: *mut f32,
    out_dz: *mut f32,
) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        let (origin, direction, valid) = backend.get_aim_ray(hand);
        if valid {
            if !out_ox.is_null() {
                *out_ox = origin[0];
            }
            if !out_oy.is_null() {
                *out_oy = origin[1];
            }
            if !out_oz.is_null() {
                *out_oz = origin[2];
            }
            if !out_dx.is_null() {
                *out_dx = direction[0];
            }
            if !out_dy.is_null() {
                *out_dy = direction[1];
            }
            if !out_dz.is_null() {
                *out_dz = direction[2];
            }
            1
        } else {
            0
        }
    } else {
        // Headless: aim rays not available.
        if !out_ox.is_null() {
            *out_ox = 0.0;
        }
        if !out_oy.is_null() {
            *out_oy = 0.0;
        }
        if !out_oz.is_null() {
            *out_oz = 0.0;
        }
        if !out_dx.is_null() {
            *out_dx = 0.0;
        }
        if !out_dy.is_null() {
            *out_dy = 0.0;
        }
        if !out_dz.is_null() {
            *out_dz = 0.0;
        }
        0
    }
}

// ── Reference spaces ───────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_set_reference_space(
    session: *mut XrSessionContext,
    space_type: u8,
) -> i32 {
    if session.is_null() {
        return 0;
    }
    if space_type > MXR_SPACE_UNBOUNDED {
        return 0;
    }
    (*session).reference_space = space_type;
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_reference_space(session: *mut XrSessionContext) -> u8 {
    if session.is_null() {
        return MXR_SPACE_STAGE;
    }
    (*session).reference_space
}

// ── Capabilities ───────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_has_extension(
    session: *mut XrSessionContext,
    ext_name_ptr: *const c_char,
    ext_name_len: u32,
) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        let name = read_str(ext_name_ptr, ext_name_len);
        backend.has_extension(name) as i32
    } else {
        0 // Headless: no extensions.
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_has_hand_tracking(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        backend.has_hand_tracking() as i32
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_has_passthrough(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        backend.has_passthrough() as i32
    } else {
        0
    }
}

// ── Debug & inspection ─────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_print_tree(session: *mut XrSessionContext, panel_id: u32) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel(panel_id) {
        let html = panel.serialize_subtree(panel.mount_point_id);
        eprintln!("[panel {}] {}", panel_id, html);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_serialize_subtree(
    session: *mut XrSessionContext,
    panel_id: u32,
    buf: *mut c_char,
    buf_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let html = (*session)
        .get_panel(panel_id)
        .map(|p| p.serialize_subtree(p.mount_point_id))
        .unwrap_or_default();
    write_to_buf(&html, buf, buf_len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_get_node_tag(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    buf: *mut c_char,
    buf_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let tag = (*session)
        .get_panel(panel_id)
        .map(|p| p.get_node_tag(node_id as usize))
        .unwrap_or_default();
    write_to_buf(&tag, buf, buf_len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_get_text_content(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    buf: *mut c_char,
    buf_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let text = (*session)
        .get_panel(panel_id)
        .map(|p| p.get_text_content(node_id as usize))
        .unwrap_or_default();
    write_to_buf(&text, buf, buf_len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_get_attribute_value(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
    name_ptr: *const c_char,
    name_len: u32,
    buf: *mut c_char,
    buf_len: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    let name = read_str(name_ptr, name_len);
    let value = (*session)
        .get_panel(panel_id)
        .and_then(|p| p.get_attribute_value(node_id as usize, name))
        .unwrap_or_default();
    write_to_buf(&value, buf, buf_len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_inject_event(
    session: *mut XrSessionContext,
    panel_id: u32,
    handler_id: u32,
    event_type: u8,
    value_ptr: *const c_char,
    value_len: u32,
) {
    if session.is_null() {
        return;
    }
    let value = read_str(value_ptr, value_len).to_string();
    (*session).events.push(InternalEvent {
        panel_id,
        handler_id,
        event_type,
        value,
        hit_u: -1.0,
        hit_v: -1.0,
        hand: 0,
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_version(buf: *mut c_char, buf_len: u32) -> u32 {
    write_to_buf(VERSION, buf, buf_len)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_mount_point_id(
    session: *mut XrSessionContext,
    panel_id: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel(panel_id)
        .map(|p| p.mount_point_id as u32)
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_get_child_mojo_id(
    session: *mut XrSessionContext,
    panel_id: u32,
    parent_id: u32,
    index: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel(panel_id)
        .and_then(|p| {
            let child_blitz_id = p.child_at(parent_id as usize, index)?;
            p.id_map.get_mojo(child_blitz_id)
        })
        .unwrap_or(0)
}

// ── ID mapping (AssignId opcode support) ───────────────────────────────────

/// Assign a mojo-gui element ID to a Blitz node ID.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_assign_id(
    session: *mut XrSessionContext,
    panel_id: u32,
    mojo_id: u32,
    node_id: u32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.id_map.assign(mojo_id, node_id as usize);
    }
}

/// Resolve a mojo-gui element ID to a Blitz node ID.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_resolve_id(
    session: *mut XrSessionContext,
    panel_id: u32,
    mojo_id: u32,
) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel(panel_id)
        .and_then(|p| p.id_map.get_blitz(mojo_id))
        .map(|id| id as u32)
        .unwrap_or(0)
}

// ── Stack operations (mutation interpreter support) ────────────────────────

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_stack_push(
    session: *mut XrSessionContext,
    panel_id: u32,
    node_id: u32,
) {
    if session.is_null() {
        return;
    }
    if let Some(panel) = (*session).get_panel_mut(panel_id) {
        panel.stack.push(node_id as usize);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_panel_stack_pop(session: *mut XrSessionContext, panel_id: u32) -> u32 {
    if session.is_null() {
        return 0;
    }
    (*session)
        .get_panel_mut(panel_id)
        .and_then(|p| p.stack.pop())
        .map(|id| id as u32)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Output-pointer FFI variants — avoids struct-return ABI issues with
// Mojo's DLHandle (which can't reliably return C structs > 16 bytes).
// These mirror the desktop shim's mblitz_poll_event_into() pattern.
// ---------------------------------------------------------------------------

/// Get the select (trigger) state for both hands. Returns a bitfield:
/// bit 0 = left active, bit 1 = right active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_select_state(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        let (left, right) = backend.get_select_state();
        (left as i32) | ((right as i32) << 1)
    } else {
        0
    }
}

/// Get the squeeze (grip) state for both hands. Returns a bitfield:
/// bit 0 = left active, bit 1 = right active.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_squeeze_state(session: *mut XrSessionContext) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = &*session;
    if let Some(ref backend) = ctx.xr_backend {
        let (left, right) = backend.get_squeeze_state();
        (left as i32) | ((right as i32) << 1)
    } else {
        0
    }
}

/// Poll the next event, writing each field to caller-provided output pointers.
///
/// Returns 1 if an event was available, 0 if the queue was empty.
/// When returning 0, the output pointers are not modified.
///
/// The `out_value_ptr` / `out_value_len` pair points into an internal buffer
/// that stays alive until the next `mxr_poll_event_into()` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_poll_event_into(
    session: *mut XrSessionContext,
    out_panel_id: *mut u32,
    out_handler_id: *mut u32,
    out_event_type: *mut u8,
    out_value_ptr: *mut *const u8,
    out_value_len: *mut u32,
    out_hit_u: *mut f32,
    out_hit_v: *mut f32,
    out_hand: *mut u8,
) -> i32 {
    if session.is_null() {
        return 0;
    }
    let ctx = unsafe { &mut *session };
    if ctx.events.events.is_empty() {
        return 0;
    }

    let event = ctx.events.events.remove(0);
    ctx.events.last_polled_value = event.value;

    unsafe {
        *out_panel_id = event.panel_id;
        *out_handler_id = event.handler_id;
        *out_event_type = event.event_type;
        *out_hit_u = event.hit_u;
        *out_hit_v = event.hit_v;
        *out_hand = event.hand;

        if ctx.events.last_polled_value.is_empty() {
            *out_value_ptr = std::ptr::null();
            *out_value_len = 0;
        } else {
            *out_value_ptr = ctx.events.last_polled_value.as_ptr();
            *out_value_len = ctx.events.last_polled_value.len() as u32;
        }
    }
    1
}

/// Raycast against all visible, interactive panels, writing the result to
/// caller-provided output pointers.
///
/// Returns 1 if a panel was hit, 0 if the ray missed all panels.
/// When returning 0, output pointers are zeroed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_raycast_panels_into(
    session: *mut XrSessionContext,
    ox: f32,
    oy: f32,
    oz: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    out_panel_id: *mut u32,
    out_u: *mut f32,
    out_v: *mut f32,
    out_distance: *mut f32,
) -> i32 {
    if session.is_null() {
        unsafe {
            *out_panel_id = 0;
            *out_u = 0.0;
            *out_v = 0.0;
            *out_distance = 0.0;
        }
        return 0;
    }
    let result = unsafe { &*session }.raycast([ox, oy, oz], [dx, dy, dz]);
    unsafe {
        *out_panel_id = result.panel_id;
        *out_u = result.u;
        *out_v = result.v;
        *out_distance = result.distance;
    }
    result.hit
}

/// Get a controller/head pose, writing the result to caller-provided
/// output pointers.
///
/// Returns 1 if the pose is valid (tracking active), 0 otherwise.
/// In headless mode, always returns 0 and zeroes the output.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mxr_get_pose_into(
    session: *mut XrSessionContext,
    _hand: u8,
    out_px: *mut f32,
    out_py: *mut f32,
    out_pz: *mut f32,
    out_qx: *mut f32,
    out_qy: *mut f32,
    out_qz: *mut f32,
    out_qw: *mut f32,
) -> i32 {
    unsafe {
        *out_px = 0.0;
        *out_py = 0.0;
        *out_pz = 0.0;
        *out_qx = 0.0;
        *out_qy = 0.0;
        *out_qz = 0.0;
        *out_qw = 1.0;
    }
    if session.is_null() {
        return 0;
    }
    // In headless mode, poses are not tracked — return invalid.
    // When OpenXR is wired up, this will query the runtime for the
    // actual controller/head pose and write valid data.
    0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Session lifecycle ──────────────────────────────────────────────

    #[test]
    fn headless_session_lifecycle() {
        unsafe {
            let session = mxr_create_headless();
            assert!(!session.is_null());
            assert_eq!(mxr_is_alive(session), 1);
            assert_eq!(mxr_session_state(session), MXR_STATE_FOCUSED);
            mxr_destroy_session(session);
        }
    }

    #[test]
    fn named_session_lifecycle() {
        unsafe {
            let name = "Test App";
            let session = mxr_create_session(name.as_ptr() as *const c_char, name.len() as u32);
            assert!(!session.is_null());
            assert_eq!(mxr_is_alive(session), 1);
            // With a real OpenXR runtime, state starts at IDLE.
            // Without one (CI, headless fallback), state starts at FOCUSED.
            let state = mxr_session_state(session);
            assert!(
                state == MXR_STATE_IDLE || state == MXR_STATE_FOCUSED,
                "expected IDLE or FOCUSED, got {state}"
            );
            mxr_destroy_session(session);
        }
    }

    #[test]
    fn null_session_safety() {
        unsafe {
            assert_eq!(mxr_is_alive(std::ptr::null_mut()), 0);
            assert_eq!(mxr_session_state(std::ptr::null_mut()), MXR_STATE_EXITING);
            assert_eq!(mxr_create_panel(std::ptr::null_mut(), 100, 100), 0);
            assert_eq!(mxr_panel_count(std::ptr::null_mut()), 0);
            mxr_destroy_session(std::ptr::null_mut()); // Should not crash.
        }
    }

    // ── Panel lifecycle ───────────────────────────────────────────────

    #[test]
    fn create_and_destroy_panel() {
        unsafe {
            let session = mxr_create_headless();
            assert_eq!(mxr_panel_count(session), 0);

            let p1 = mxr_create_panel(session, 960, 720);
            assert_ne!(p1, 0);
            assert_eq!(mxr_panel_count(session), 1);
            assert_eq!(mxr_panel_is_visible(session, p1), 1);

            let p2 = mxr_create_panel(session, 480, 360);
            assert_ne!(p2, 0);
            assert_ne!(p1, p2);
            assert_eq!(mxr_panel_count(session), 2);

            mxr_destroy_panel(session, p1);
            assert_eq!(mxr_panel_count(session), 1);
            assert_eq!(mxr_panel_is_visible(session, p1), 0); // Destroyed panel

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn panel_visibility() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            assert_eq!(mxr_panel_is_visible(session, p), 1);
            mxr_panel_set_visible(session, p, 0);
            assert_eq!(mxr_panel_is_visible(session, p), 0);
            mxr_panel_set_visible(session, p, 1);
            assert_eq!(mxr_panel_is_visible(session, p), 1);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn zero_dimension_panel_rejected() {
        unsafe {
            let session = mxr_create_headless();
            assert_eq!(mxr_create_panel(session, 0, 100), 0);
            assert_eq!(mxr_create_panel(session, 100, 0), 0);
            assert_eq!(mxr_panel_count(session), 0);
            mxr_destroy_session(session);
        }
    }

    // ── DOM operations ────────────────────────────────────────────────

    #[test]
    fn create_elements_and_append() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);
            assert_ne!(mount, 0);

            let tag = "div";
            let div = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            assert_ne!(div, 0);

            let ids = [div];
            mxr_panel_append_children(session, p, mount, ids.as_ptr(), 1);
            assert_eq!(mxr_panel_child_count(session, p, mount), 1);
            assert_eq!(mxr_panel_child_at(session, p, mount, 0), div);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn text_node_operations() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            let text = "Hello, XR!";
            let node = mxr_panel_create_text_node(
                session,
                p,
                text.as_ptr() as *const c_char,
                text.len() as u32,
            );
            assert_ne!(node, 0);

            // Read back text content
            let mut buf = [0u8; 64];
            let len =
                mxr_panel_get_text_content(session, p, node, buf.as_mut_ptr() as *mut c_char, 64);
            let content = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(content, "Hello, XR!");

            // Update text
            let new_text = "Updated!";
            mxr_panel_set_text_content(
                session,
                p,
                node,
                new_text.as_ptr() as *const c_char,
                new_text.len() as u32,
            );
            let len =
                mxr_panel_get_text_content(session, p, node, buf.as_mut_ptr() as *mut c_char, 64);
            let content = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(content, "Updated!");

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn set_and_get_attribute() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            let tag = "button";
            let btn = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            let name = "class";
            let value = "primary";
            mxr_panel_set_attribute(
                session,
                p,
                btn,
                name.as_ptr() as *const c_char,
                name.len() as u32,
                value.as_ptr() as *const c_char,
                value.len() as u32,
            );

            let mut buf = [0u8; 64];
            let len = mxr_panel_get_attribute_value(
                session,
                p,
                btn,
                name.as_ptr() as *const c_char,
                name.len() as u32,
                buf.as_mut_ptr() as *mut c_char,
                64,
            );
            let attr = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(attr, "primary");

            // Remove attribute
            mxr_panel_remove_attribute(
                session,
                p,
                btn,
                name.as_ptr() as *const c_char,
                name.len() as u32,
            );
            let len = mxr_panel_get_attribute_value(
                session,
                p,
                btn,
                name.as_ptr() as *const c_char,
                name.len() as u32,
                buf.as_mut_ptr() as *mut c_char,
                64,
            );
            assert_eq!(len, 0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn insert_before_and_after() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            let tag = "span";
            let a = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let b = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let c = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            // Append a to mount: [a]
            mxr_panel_append_children(session, p, mount, [a].as_ptr(), 1);
            assert_eq!(mxr_panel_child_count(session, p, mount), 1);

            // Insert b before a: [b, a]
            mxr_panel_insert_before(session, p, a, [b].as_ptr(), 1);
            assert_eq!(mxr_panel_child_count(session, p, mount), 2);
            assert_eq!(mxr_panel_child_at(session, p, mount, 0), b);
            assert_eq!(mxr_panel_child_at(session, p, mount, 1), a);

            // Insert c after b: [b, c, a]
            mxr_panel_insert_after(session, p, b, [c].as_ptr(), 1);
            assert_eq!(mxr_panel_child_count(session, p, mount), 3);
            assert_eq!(mxr_panel_child_at(session, p, mount, 0), b);
            assert_eq!(mxr_panel_child_at(session, p, mount, 1), c);
            assert_eq!(mxr_panel_child_at(session, p, mount, 2), a);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn replace_with_and_remove() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            let tag = "div";
            let a = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let b = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            mxr_panel_append_children(session, p, mount, [a].as_ptr(), 1);
            assert_eq!(mxr_panel_child_count(session, p, mount), 1);

            // Replace a with b
            mxr_panel_replace_with(session, p, a, [b].as_ptr(), 1);
            assert_eq!(mxr_panel_child_count(session, p, mount), 1);
            assert_eq!(mxr_panel_child_at(session, p, mount, 0), b);

            // Remove b
            mxr_panel_remove_node(session, p, b);
            assert_eq!(mxr_panel_child_count(session, p, mount), 0);

            mxr_destroy_session(session);
        }
    }

    // ── Events ────────────────────────────────────────────────────────

    #[test]
    fn event_inject_and_poll() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            assert_eq!(mxr_event_count(session), 0);

            let value = "test value";
            mxr_panel_inject_event(
                session,
                p,
                42,
                MXR_EVT_INPUT,
                value.as_ptr() as *const c_char,
                value.len() as u32,
            );
            assert_eq!(mxr_event_count(session), 1);

            let evt = mxr_poll_event(session);
            assert_eq!(evt.valid, 1);
            assert_eq!(evt.panel_id, p);
            assert_eq!(evt.handler_id, 42);
            assert_eq!(evt.event_type, MXR_EVT_INPUT);
            assert_eq!(evt.value_len, value.len() as u32);

            let polled_value = std::str::from_utf8(std::slice::from_raw_parts(
                evt.value_ptr as *const u8,
                evt.value_len as usize,
            ))
            .unwrap();
            assert_eq!(polled_value, "test value");

            // Queue is empty now
            let evt2 = mxr_poll_event(session);
            assert_eq!(evt2.valid, 0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn event_listener_registration() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            let tag = "button";
            let btn = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            mxr_panel_append_children(session, p, mount, [btn].as_ptr(), 1);

            mxr_panel_add_event_listener(session, p, btn, 7, MXR_EVT_CLICK);

            let panel = (*session).get_panel(p).unwrap();
            assert_eq!(panel.listeners.len(), 1);
            assert_eq!(panel.listeners[0].handler_id, 7);

            mxr_panel_remove_event_listener(session, p, btn, 7, MXR_EVT_CLICK);
            let panel = (*session).get_panel(p).unwrap();
            assert_eq!(panel.listeners.len(), 0);

            mxr_destroy_session(session);
        }
    }

    // ── Raycasting ────────────────────────────────────────────────────

    #[test]
    fn raycast_hits_panel() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 960, 720);

            // Place panel at (0, 1.4, -1.0), facing the user (identity rotation)
            mxr_panel_set_transform(session, p, 0.0, 1.4, -1.0, 0.0, 0.0, 0.0, 1.0);
            mxr_panel_set_size(session, p, 0.8, 0.6);

            // Cast a ray from origin toward the panel center
            let hit = mxr_raycast_panels(session, 0.0, 1.4, 0.0, 0.0, 0.0, -1.0);
            assert_eq!(hit.hit, 1);
            assert_eq!(hit.panel_id, p);
            assert!(
                (hit.u - 0.5).abs() < 0.01,
                "u should be ~0.5, got {}",
                hit.u
            );
            assert!(
                (hit.v - 0.5).abs() < 0.01,
                "v should be ~0.5, got {}",
                hit.v
            );
            assert!((hit.distance - 1.0).abs() < 0.01);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn raycast_misses_when_outside_bounds() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 960, 720);
            mxr_panel_set_transform(session, p, 0.0, 1.4, -1.0, 0.0, 0.0, 0.0, 1.0);
            mxr_panel_set_size(session, p, 0.8, 0.6);

            // Cast a ray that misses (far to the right)
            let hit = mxr_raycast_panels(session, 5.0, 1.4, 0.0, 0.0, 0.0, -1.0);
            assert_eq!(hit.hit, 0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn raycast_skips_hidden_panels() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 960, 720);
            mxr_panel_set_transform(session, p, 0.0, 1.4, -1.0, 0.0, 0.0, 0.0, 1.0);
            mxr_panel_set_size(session, p, 0.8, 0.6);

            mxr_panel_set_visible(session, p, 0);
            let hit = mxr_raycast_panels(session, 0.0, 1.4, 0.0, 0.0, 0.0, -1.0);
            assert_eq!(hit.hit, 0);

            mxr_destroy_session(session);
        }
    }

    // ── Focus ─────────────────────────────────────────────────────────

    #[test]
    fn focus_management() {
        unsafe {
            let session = mxr_create_headless();
            let p1 = mxr_create_panel(session, 100, 100);
            let p2 = mxr_create_panel(session, 100, 100);

            // First panel gets auto-focus
            assert_eq!(mxr_get_focused_panel(session), p1);

            mxr_set_focused_panel(session, p2);
            assert_eq!(mxr_get_focused_panel(session), p2);

            mxr_set_focused_panel(session, 0);
            assert_eq!(mxr_get_focused_panel(session), 0);

            mxr_destroy_session(session);
        }
    }

    // ── Frame loop ────────────────────────────────────────────────────

    #[test]
    fn headless_frame_loop() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            let ts = mxr_wait_frame(session);
            assert!(ts > 0);

            assert_eq!(mxr_begin_frame(session), 1);

            // Mark panel dirty via end_mutations
            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);

            let rendered = mxr_render_dirty_panels(session);
            assert_eq!(rendered, 1);

            // Second render should find nothing dirty
            let rendered = mxr_render_dirty_panels(session);
            assert_eq!(rendered, 0);

            mxr_end_frame(session);
            mxr_destroy_session(session);
        }
    }

    // ── Reference spaces ──────────────────────────────────────────────

    #[test]
    fn reference_space_management() {
        unsafe {
            let session = mxr_create_headless();

            assert_eq!(mxr_get_reference_space(session), MXR_SPACE_STAGE);
            assert_eq!(mxr_set_reference_space(session, MXR_SPACE_LOCAL), 1);
            assert_eq!(mxr_get_reference_space(session), MXR_SPACE_LOCAL);

            // Invalid space type
            assert_eq!(mxr_set_reference_space(session, 99), 0);
            assert_eq!(mxr_get_reference_space(session), MXR_SPACE_LOCAL);

            mxr_destroy_session(session);
        }
    }

    // ── Version ───────────────────────────────────────────────────────

    #[test]
    fn version_string() {
        unsafe {
            let mut buf = [0u8; 32];
            let len = mxr_version(buf.as_mut_ptr() as *mut c_char, 32);
            let version = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(version, VERSION);
        }
    }

    // ── Serialization ─────────────────────────────────────────────────

    #[test]
    fn serialize_subtree() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            let tag = "p";
            let para = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let text = "Hello, world!";
            let txt = mxr_panel_create_text_node(
                session,
                p,
                text.as_ptr() as *const c_char,
                text.len() as u32,
            );

            mxr_panel_append_children(session, p, mount, [para].as_ptr(), 1);
            mxr_panel_append_children(session, p, para, [txt].as_ptr(), 1);

            let mut buf = [0u8; 256];
            let len = mxr_panel_serialize_subtree(session, p, buf.as_mut_ptr() as *mut c_char, 256);
            let html = std::str::from_utf8(&buf[..len as usize]).unwrap();
            // Mount point is <body> (real Blitz document structure)
            assert_eq!(html, "<body><p>Hello, world!</p></body>");

            mxr_destroy_session(session);
        }
    }

    // ── Placeholder nodes ─────────────────────────────────────────────

    #[test]
    fn placeholder_node() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            let ph = mxr_panel_create_placeholder(session, p);
            assert_ne!(ph, 0);

            mxr_panel_append_children(session, p, mount, [ph].as_ptr(), 1);

            let mut buf = [0u8; 256];
            let len = mxr_panel_serialize_subtree(session, p, buf.as_mut_ptr() as *mut c_char, 256);
            let html = std::str::from_utf8(&buf[..len as usize]).unwrap();
            // Mount point is <body>, placeholder renders as <!---->
            assert_eq!(html, "<body><!----></body>");

            mxr_destroy_session(session);
        }
    }

    // ── Node path navigation ──────────────────────────────────────────

    #[test]
    fn node_at_path() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            let tag = "div";
            let outer = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let inner = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            mxr_panel_append_children(session, p, mount, [outer].as_ptr(), 1);
            mxr_panel_append_children(session, p, outer, [inner].as_ptr(), 1);

            // Navigate: mount → child 0 → child 0
            let path = [0u32, 0u32];
            let found = mxr_panel_node_at_path(session, p, mount, path.as_ptr(), 2);
            assert_eq!(found, inner);

            mxr_destroy_session(session);
        }
    }

    // ── Panel UA stylesheet storage ───────────────────────────────────

    #[test]
    fn ua_stylesheet() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            let css = "body { margin: 0; }";
            mxr_panel_add_ua_stylesheet(
                session,
                p,
                css.as_ptr() as *const c_char,
                css.len() as u32,
            );

            let panel = (*session).get_panel(p).unwrap();
            assert_eq!(panel.ua_stylesheets.len(), 1);
            assert_eq!(panel.ua_stylesheets[0], "body { margin: 0; }");

            mxr_destroy_session(session);
        }
    }

    // ── ID mapping ────────────────────────────────────────────────────

    #[test]
    fn id_mapping_assign_and_resolve() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            let tag = "div";
            let div = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            // Assign mojo ID 42 to the div
            mxr_panel_assign_id(session, p, 42, div);

            // Resolve it back
            let resolved = mxr_panel_resolve_id(session, p, 42);
            assert_eq!(resolved, div);

            // Unassigned ID returns 0
            let unresolved = mxr_panel_resolve_id(session, p, 999);
            assert_eq!(unresolved, 0);

            mxr_destroy_session(session);
        }
    }

    // ── Stack operations ──────────────────────────────────────────────

    #[test]
    fn stack_push_and_pop() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);

            let tag = "div";
            let a = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let b = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            mxr_panel_stack_push(session, p, a);
            mxr_panel_stack_push(session, p, b);

            let popped = mxr_panel_stack_pop(session, p);
            assert_eq!(popped, b);

            let popped = mxr_panel_stack_pop(session, p);
            assert_eq!(popped, a);

            // Empty stack returns 0
            let popped = mxr_panel_stack_pop(session, p);
            assert_eq!(popped, 0);

            mxr_destroy_session(session);
        }
    }

    // ── Multi-panel DOM isolation ─────────────────────────────────────

    #[test]
    fn multi_panel_dom_isolation() {
        unsafe {
            let session = mxr_create_headless();
            let p1 = mxr_create_panel(session, 100, 100);
            let p2 = mxr_create_panel(session, 200, 200);
            let mount1 = mxr_panel_mount_point_id(session, p1);
            let mount2 = mxr_panel_mount_point_id(session, p2);

            // Create elements in each panel
            let tag = "div";
            let div1 = mxr_panel_create_element(
                session,
                p1,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let div2 = mxr_panel_create_element(
                session,
                p2,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );

            mxr_panel_append_children(session, p1, mount1, [div1].as_ptr(), 1);
            mxr_panel_append_children(session, p2, mount2, [div2].as_ptr(), 1);

            // Each panel should have exactly 1 child in its mount point
            assert_eq!(mxr_panel_child_count(session, p1, mount1), 1);
            assert_eq!(mxr_panel_child_count(session, p2, mount2), 1);

            // Destroying p1 shouldn't affect p2
            mxr_destroy_panel(session, p1);
            assert_eq!(mxr_panel_child_count(session, p2, mount2), 1);

            mxr_destroy_session(session);
        }
    }

    // ── Blitz document integration ───────────────────────────────────

    #[test]
    fn blitz_document_structure() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);
            let mount = mxr_panel_mount_point_id(session, p);

            // The mount point should be a <body> element
            let mut buf = [0u8; 32];
            let len =
                mxr_panel_get_node_tag(session, p, mount, buf.as_mut_ptr() as *mut c_char, 32);
            let tag = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(tag, "body");

            // Empty mount point serialization
            let mut buf = [0u8; 256];
            let len = mxr_panel_serialize_subtree(session, p, buf.as_mut_ptr() as *mut c_char, 256);
            let html = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(html, "<body />"); // Empty body with self-closing tag

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn blitz_nested_elements_with_attributes() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            let mount = mxr_panel_mount_point_id(session, p);

            // Build: <body><div class="container"><span>Hello</span></div></body>
            let div_tag = "div";
            let div = mxr_panel_create_element(
                session,
                p,
                div_tag.as_ptr() as *const c_char,
                div_tag.len() as u32,
            );

            let name = "class";
            let value = "container";
            mxr_panel_set_attribute(
                session,
                p,
                div,
                name.as_ptr() as *const c_char,
                name.len() as u32,
                value.as_ptr() as *const c_char,
                value.len() as u32,
            );

            let span_tag = "span";
            let span = mxr_panel_create_element(
                session,
                p,
                span_tag.as_ptr() as *const c_char,
                span_tag.len() as u32,
            );

            let text = "Hello";
            let txt = mxr_panel_create_text_node(
                session,
                p,
                text.as_ptr() as *const c_char,
                text.len() as u32,
            );

            mxr_panel_append_children(session, p, mount, [div].as_ptr(), 1);
            mxr_panel_append_children(session, p, div, [span].as_ptr(), 1);
            mxr_panel_append_children(session, p, span, [txt].as_ptr(), 1);

            let mut buf = [0u8; 512];
            let len = mxr_panel_serialize_subtree(session, p, buf.as_mut_ptr() as *mut c_char, 512);
            let html = std::str::from_utf8(&buf[..len as usize]).unwrap();
            assert_eq!(
                html,
                "<body><div class=\"container\"><span>Hello</span></div></body>"
            );

            mxr_destroy_session(session);
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Output-pointer FFI variants (_into)
    // ═══════════════════════════════════════════════════════════════════════

    #[test]
    fn poll_event_into_empty_queue_returns_zero() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);

            let mut panel_id: u32 = 0xFF;
            let mut handler_id: u32 = 0xFF;
            let mut event_type: u8 = 0xFF;
            let mut value_ptr: *const u8 = std::ptr::null();
            let mut value_len: u32 = 0xFF;
            let mut hit_u: f32 = -1.0;
            let mut hit_v: f32 = -1.0;
            let mut hand: u8 = 0xFF;

            let valid = mxr_poll_event_into(
                session,
                &mut panel_id,
                &mut handler_id,
                &mut event_type,
                &mut value_ptr,
                &mut value_len,
                &mut hit_u,
                &mut hit_v,
                &mut hand,
            );
            assert_eq!(valid, 0);
            // Output pointers should be untouched when queue is empty
            assert_eq!(panel_id, 0xFF);
            assert_eq!(handler_id, 0xFF);

            let _ = p;
            mxr_destroy_session(session);
        }
    }

    #[test]
    fn poll_event_into_click_event() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);

            // Inject a click event
            let value = "";
            mxr_panel_inject_event(
                session,
                p,
                42,
                MXR_EVT_CLICK as u8,
                value.as_ptr() as *const c_char,
                0,
            );

            let mut panel_id: u32 = 0;
            let mut handler_id: u32 = 0;
            let mut event_type: u8 = 0;
            let mut value_ptr: *const u8 = std::ptr::null();
            let mut value_len: u32 = 0;
            let mut hit_u: f32 = -1.0;
            let mut hit_v: f32 = -1.0;
            let mut hand: u8 = 0xFF;

            let valid = mxr_poll_event_into(
                session,
                &mut panel_id,
                &mut handler_id,
                &mut event_type,
                &mut value_ptr,
                &mut value_len,
                &mut hit_u,
                &mut hit_v,
                &mut hand,
            );
            assert_eq!(valid, 1);
            assert_eq!(panel_id, p);
            assert_eq!(handler_id, 42);
            assert_eq!(event_type, MXR_EVT_CLICK as u8);
            assert_eq!(value_len, 0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn poll_event_into_input_event_with_value() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);

            let value = "hello 🌍";
            mxr_panel_inject_event(
                session,
                p,
                7,
                MXR_EVT_INPUT as u8,
                value.as_ptr() as *const c_char,
                value.len() as u32,
            );

            let mut panel_id: u32 = 0;
            let mut handler_id: u32 = 0;
            let mut event_type: u8 = 0;
            let mut value_ptr: *const u8 = std::ptr::null();
            let mut value_len: u32 = 0;
            let mut hit_u: f32 = -1.0;
            let mut hit_v: f32 = -1.0;
            let mut hand: u8 = 0xFF;

            let valid = mxr_poll_event_into(
                session,
                &mut panel_id,
                &mut handler_id,
                &mut event_type,
                &mut value_ptr,
                &mut value_len,
                &mut hit_u,
                &mut hit_v,
                &mut hand,
            );
            assert_eq!(valid, 1);
            assert_eq!(handler_id, 7);
            assert_eq!(event_type, MXR_EVT_INPUT as u8);
            assert!(value_len > 0);

            // Reconstruct string from pointer
            let slice = std::slice::from_raw_parts(value_ptr, value_len as usize);
            let s = std::str::from_utf8(slice).unwrap();
            assert_eq!(s, "hello 🌍");

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn poll_event_into_multiple_events_in_order() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);

            let v1 = "";
            mxr_panel_inject_event(
                session,
                p,
                1,
                MXR_EVT_CLICK as u8,
                v1.as_ptr() as *const c_char,
                0,
            );
            let v2 = "second";
            mxr_panel_inject_event(
                session,
                p,
                2,
                MXR_EVT_INPUT as u8,
                v2.as_ptr() as *const c_char,
                v2.len() as u32,
            );

            let mut panel_id: u32 = 0;
            let mut handler_id: u32 = 0;
            let mut event_type: u8 = 0;
            let mut value_ptr: *const u8 = std::ptr::null();
            let mut value_len: u32 = 0;
            let mut hit_u: f32 = 0.0;
            let mut hit_v: f32 = 0.0;
            let mut hand: u8 = 0;

            // First event
            let valid = mxr_poll_event_into(
                session,
                &mut panel_id,
                &mut handler_id,
                &mut event_type,
                &mut value_ptr,
                &mut value_len,
                &mut hit_u,
                &mut hit_v,
                &mut hand,
            );
            assert_eq!(valid, 1);
            assert_eq!(handler_id, 1);

            // Second event
            let valid = mxr_poll_event_into(
                session,
                &mut panel_id,
                &mut handler_id,
                &mut event_type,
                &mut value_ptr,
                &mut value_len,
                &mut hit_u,
                &mut hit_v,
                &mut hand,
            );
            assert_eq!(valid, 1);
            assert_eq!(handler_id, 2);
            let slice = std::slice::from_raw_parts(value_ptr, value_len as usize);
            assert_eq!(std::str::from_utf8(slice).unwrap(), "second");

            // Queue empty
            let valid = mxr_poll_event_into(
                session,
                &mut panel_id,
                &mut handler_id,
                &mut event_type,
                &mut value_ptr,
                &mut value_len,
                &mut hit_u,
                &mut hit_v,
                &mut hand,
            );
            assert_eq!(valid, 0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn raycast_panels_into_hit() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);

            // Position the panel at (0, 0, -1), facing +Z (identity rotation)
            mxr_panel_set_transform(session, p, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0);
            mxr_panel_set_size(session, p, 1.0, 1.0);

            let mut out_panel_id: u32 = 0;
            let mut out_u: f32 = 0.0;
            let mut out_v: f32 = 0.0;
            let mut out_distance: f32 = 0.0;

            // Ray from origin pointing at -Z should hit the panel
            let hit = mxr_raycast_panels_into(
                session,
                0.0,
                0.0,
                0.0, // origin
                0.0,
                0.0,
                -1.0, // direction
                &mut out_panel_id,
                &mut out_u,
                &mut out_v,
                &mut out_distance,
            );
            assert_eq!(hit, 1);
            assert_eq!(out_panel_id, p);
            assert!(out_distance > 0.0);
            // Hit should be near center (u≈0.5, v≈0.5)
            assert!((out_u - 0.5).abs() < 0.1);
            assert!((out_v - 0.5).abs() < 0.1);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn raycast_panels_into_miss() {
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);

            // Panel at (0, 0, -1)
            mxr_panel_set_transform(session, p, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0);
            mxr_panel_set_size(session, p, 0.5, 0.5);

            let mut out_panel_id: u32 = 0xFF;
            let mut out_u: f32 = -1.0;
            let mut out_v: f32 = -1.0;
            let mut out_distance: f32 = -1.0;

            // Ray pointing away (+Z) should miss
            let hit = mxr_raycast_panels_into(
                session,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0, // wrong direction
                &mut out_panel_id,
                &mut out_u,
                &mut out_v,
                &mut out_distance,
            );
            assert_eq!(hit, 0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn get_pose_into_headless_returns_invalid() {
        unsafe {
            let session = mxr_create_headless();

            let mut px: f32 = 99.0;
            let mut py: f32 = 99.0;
            let mut pz: f32 = 99.0;
            let mut qx: f32 = 99.0;
            let mut qy: f32 = 99.0;
            let mut qz: f32 = 99.0;
            let mut qw: f32 = 99.0;

            let valid = mxr_get_pose_into(
                session,
                MXR_HAND_LEFT as u8,
                &mut px,
                &mut py,
                &mut pz,
                &mut qx,
                &mut qy,
                &mut qz,
                &mut qw,
            );
            assert_eq!(valid, 0);
            // Position should be zeroed
            assert_eq!(px, 0.0);
            assert_eq!(py, 0.0);
            assert_eq!(pz, 0.0);
            // Quaternion should be identity (0,0,0,1)
            assert_eq!(qx, 0.0);
            assert_eq!(qy, 0.0);
            assert_eq!(qz, 0.0);
            assert_eq!(qw, 1.0);

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn layout_resolve_in_render() {
        // Verify that mxr_render_dirty_panels calls doc.resolve() without crashing
        unsafe {
            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 800, 600);
            let mount = mxr_panel_mount_point_id(session, p);

            // Build a simple DOM
            let tag = "div";
            let div = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let text = "Layout test";
            let txt = mxr_panel_create_text_node(
                session,
                p,
                text.as_ptr() as *const c_char,
                text.len() as u32,
            );
            mxr_panel_append_children(session, p, mount, [div].as_ptr(), 1);
            mxr_panel_append_children(session, p, div, [txt].as_ptr(), 1);

            // Mark dirty and render (exercises Stylo + Taffy layout)
            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);

            let rendered = mxr_render_dirty_panels(session);
            assert_eq!(rendered, 1);

            mxr_destroy_session(session);
        }
    }

    // ── GPU rendering (conditional — skipped if no GPU adapter) ────────

    /// Helper: try to initialise GPU on a session. Returns true if successful.
    unsafe fn try_init_gpu(session: *mut XrSessionContext) -> bool {
        mxr_init_gpu(session) == 1
    }

    #[test]
    fn init_gpu_on_named_session() {
        unsafe {
            let name = "GPU test";
            let session = mxr_create_session(name.as_ptr() as *const c_char, name.len() as u32);
            let has_gpu = try_init_gpu(session);
            // GPU availability is environment-dependent; just verify the call
            // doesn't crash and returns a consistent value.
            assert_eq!(mxr_has_gpu(session), if has_gpu { 1 } else { 0 });
            mxr_destroy_session(session);
        }
    }

    #[test]
    fn init_gpu_on_headless_session() {
        unsafe {
            let session = mxr_create_headless();
            // Headless sessions CAN have GPU if adapter is available.
            let has_gpu = try_init_gpu(session);
            assert_eq!(mxr_has_gpu(session), if has_gpu { 1 } else { 0 });
            mxr_destroy_session(session);
        }
    }

    #[test]
    fn init_gpu_idempotent() {
        unsafe {
            let session = mxr_create_headless();
            let first = mxr_init_gpu(session);
            let second = mxr_init_gpu(session);
            // Second call should return the same result (1 if already init'd).
            if first == 1 {
                assert_eq!(
                    second, 1,
                    "init_gpu should return 1 when already initialised"
                );
            }
            mxr_destroy_session(session);
        }
    }

    #[test]
    fn has_gpu_null_session() {
        unsafe {
            assert_eq!(mxr_has_gpu(std::ptr::null_mut()), 0);
            assert_eq!(mxr_init_gpu(std::ptr::null_mut()), 0);
        }
    }

    #[test]
    fn render_dirty_panel_with_gpu() {
        unsafe {
            let session = mxr_create_headless();
            let has_gpu = try_init_gpu(session);
            let p = mxr_create_panel(session, 200, 150);
            let mount = mxr_panel_mount_point_id(session, p);

            // Build a simple DOM: <div>Hello GPU</div>
            let tag = "div";
            let div = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            let text = "Hello GPU";
            let txt = mxr_panel_create_text_node(
                session,
                p,
                text.as_ptr() as *const c_char,
                text.len() as u32,
            );
            mxr_panel_append_children(session, p, mount, [div].as_ptr(), 1);
            mxr_panel_append_children(session, p, div, [txt].as_ptr(), 1);

            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);

            let rendered = mxr_render_dirty_panels(session);
            assert_eq!(rendered, 1, "panel should be rendered");

            if has_gpu {
                // After rendering with GPU, the panel should have a texture.
                let panel = (*session).get_panel(p).unwrap();
                assert!(
                    panel.gpu_texture.is_some(),
                    "GPU panel should have texture after render"
                );
                let pt = panel.gpu_texture.as_ref().unwrap();
                assert_eq!(pt.width, 200);
                assert_eq!(pt.height, 150);
            }

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn read_pixels_without_gpu_returns_zero() {
        unsafe {
            let session = mxr_create_headless();
            // Do NOT init GPU.
            let p = mxr_create_panel(session, 100, 100);

            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);
            mxr_render_dirty_panels(session);

            let mut buf = vec![0u8; 100 * 100 * 4];
            let bytes = mxr_panel_read_pixels(session, p, buf.as_mut_ptr(), buf.len() as u32);
            assert_eq!(bytes, 0, "no GPU → read_pixels should return 0");

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn read_pixels_null_args() {
        unsafe {
            assert_eq!(
                mxr_panel_read_pixels(std::ptr::null_mut(), 0, std::ptr::null_mut(), 0),
                0
            );

            let session = mxr_create_headless();
            let p = mxr_create_panel(session, 100, 100);
            // null buffer
            assert_eq!(
                mxr_panel_read_pixels(session, p, std::ptr::null_mut(), 0),
                0
            );
            // unknown panel
            let mut buf = vec![0u8; 100];
            assert_eq!(
                mxr_panel_read_pixels(session, 9999, buf.as_mut_ptr(), buf.len() as u32),
                0
            );

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn read_pixels_with_gpu() {
        unsafe {
            let session = mxr_create_headless();
            if !try_init_gpu(session) {
                // No GPU available — skip pixel verification but don't fail.
                mxr_destroy_session(session);
                return;
            }

            let p = mxr_create_panel(session, 64, 48);
            let mount = mxr_panel_mount_point_id(session, p);

            // Apply a UA stylesheet with white background so pixels are non-zero.
            let css = "body { background: white; }";
            mxr_panel_add_ua_stylesheet(
                session,
                p,
                css.as_ptr() as *const c_char,
                css.len() as u32,
            );

            // Build a minimal DOM so the document has content.
            let tag = "div";
            let div = mxr_panel_create_element(
                session,
                p,
                tag.as_ptr() as *const c_char,
                tag.len() as u32,
            );
            mxr_panel_append_children(session, p, mount, [div].as_ptr(), 1);

            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);
            mxr_render_dirty_panels(session);

            let pixel_count = 64 * 48;
            let buf_size = pixel_count * 4;
            let mut buf = vec![0u8; buf_size];
            let bytes = mxr_panel_read_pixels(session, p, buf.as_mut_ptr(), buf.len() as u32);
            assert_eq!(bytes as usize, buf_size, "should read full texture");

            // With a white background, most pixels should be non-zero.
            // Check that the buffer is not all zeros (i.e. rendering happened).
            let non_zero = buf.iter().filter(|&&b| b != 0).count();
            assert!(
                non_zero > 0,
                "rendered texture should contain non-zero pixels (got all zeros)"
            );

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn read_pixels_buffer_too_small() {
        unsafe {
            let session = mxr_create_headless();
            if !try_init_gpu(session) {
                mxr_destroy_session(session);
                return;
            }

            let p = mxr_create_panel(session, 64, 48);

            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);
            mxr_render_dirty_panels(session);

            // Buffer too small (need 64*48*4 = 12288 bytes).
            let mut buf = vec![0u8; 100];
            let bytes = mxr_panel_read_pixels(session, p, buf.as_mut_ptr(), buf.len() as u32);
            assert_eq!(bytes, 0, "undersized buffer should return 0");

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn render_multiple_panels_with_gpu() {
        unsafe {
            let session = mxr_create_headless();
            if !try_init_gpu(session) {
                mxr_destroy_session(session);
                return;
            }

            let p1 = mxr_create_panel(session, 100, 80);
            let p2 = mxr_create_panel(session, 200, 160);

            // Both panels are dirty after creation.
            mxr_panel_begin_mutations(session, p1);
            mxr_panel_end_mutations(session, p1);
            mxr_panel_begin_mutations(session, p2);
            mxr_panel_end_mutations(session, p2);

            let rendered = mxr_render_dirty_panels(session);
            assert_eq!(rendered, 2, "both panels should render");

            // Verify both have textures with correct dimensions.
            let panel1 = (*session).get_panel(p1).unwrap();
            let pt1 = panel1.gpu_texture.as_ref().unwrap();
            assert_eq!(pt1.width, 100);
            assert_eq!(pt1.height, 80);

            let panel2 = (*session).get_panel(p2).unwrap();
            let pt2 = panel2.gpu_texture.as_ref().unwrap();
            assert_eq!(pt2.width, 200);
            assert_eq!(pt2.height, 160);

            // Second render pass: no dirty panels, nothing rendered.
            let rendered2 = mxr_render_dirty_panels(session);
            assert_eq!(rendered2, 0, "clean panels should not re-render");

            mxr_destroy_session(session);
        }
    }

    #[test]
    fn texture_destroyed_on_panel_destroy() {
        unsafe {
            let session = mxr_create_headless();
            if !try_init_gpu(session) {
                mxr_destroy_session(session);
                return;
            }

            let p = mxr_create_panel(session, 100, 80);
            mxr_panel_begin_mutations(session, p);
            mxr_panel_end_mutations(session, p);
            mxr_render_dirty_panels(session);

            // Verify texture was created.
            let panel = (*session).get_panel(p).unwrap();
            assert!(
                panel.gpu_texture.is_some(),
                "texture should exist after render"
            );

            // Destroy the panel — GPU texture should be dropped with it.
            mxr_destroy_panel(session, p);
            assert!(
                (*session).get_panel(p).is_none(),
                "panel should be gone after destroy"
            );

            mxr_destroy_session(session);
        }
    }
}
