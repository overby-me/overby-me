/* mojo_blitz.h — C API for the Blitz desktop renderer shim.
 *
 * This header defines the FFI interface between Mojo and the Rust cdylib
 * (libmojo_blitz.so / libmojo_blitz.dylib / mojo_blitz.dll) that wraps
 * the Blitz HTML/CSS rendering engine (Stylo + Taffy + Vello + Winit).
 *
 * Architecture:
 *
 *   Mojo (native binary)
 *     │
 *     │ C FFI calls (this header)
 *     ▼
 *   libmojo_blitz (Rust cdylib)
 *     │
 *     │ Rust API calls
 *     ▼
 *   Blitz (blitz-dom, blitz-shell, blitz-paint)
 *     ├── Stylo     — CSS parsing & style resolution
 *     ├── Taffy     — Flexbox, grid, block layout
 *     ├── Parley    — Text layout & shaping
 *     ├── Vello     — GPU-accelerated 2D rendering
 *     ├── Winit     — Cross-platform windowing & input
 *     └── AccessKit — Accessibility
 *
 * Design principles:
 *
 *   1. POLLING-BASED — No function-pointer callbacks. Mojo polls for events
 *      via mblitz_poll_event(). This avoids the complexity of passing managed
 *      closures across the FFI boundary.
 *
 *   2. FLAT C ABI — All functions use C-compatible types (integers, pointers
 *      to UTF-8 strings with explicit lengths). No Rust types cross the
 *      boundary.
 *
 *   3. NODE IDs — DOM nodes are identified by uint32_t IDs (Blitz uses
 *      slab-allocated usize internally; the shim maps to u32 for Mojo
 *      compatibility). ID 0 is the document root.
 *
 *   4. TEMPLATE SUPPORT — Templates are registered as pre-built DOM subtrees
 *      that can be efficiently deep-cloned. This mirrors the web renderer's
 *      DocumentFragment.cloneNode(true) pattern.
 *
 *   5. EVENT RING BUFFER — User interaction events (clicks, input, keyboard)
 *      are buffered in a ring buffer inside the shim. Mojo drains them with
 *      mblitz_poll_event(). Same pattern as the webview shim (mojo_webview.h).
 *
 * Usage from Mojo:
 *
 *   var lib = DLHandle("libmojo_blitz.so")
 *   var ctx = lib.call["mblitz_create", ...]("My App", 7, 800, 600, 0)
 *   # ... mount DOM via mblitz_create_element, mblitz_append_children, etc.
 *   lib.call["mblitz_request_redraw", ...](ctx)
 *   while lib.call["mblitz_is_alive", ...](ctx):
 *       lib.call["mblitz_step", ...](ctx, 0)  # non-blocking
 *       # poll events...
 *       # flush mutations...
 *       lib.call["mblitz_step", ...](ctx, 1)  # blocking if idle
 *   lib.call["mblitz_destroy", ...](ctx)
 *
 * Thread safety: All functions must be called from the SAME thread (the
 * main/UI thread). Winit requires this on most platforms.
 */

#ifndef MOJO_BLITZ_H
#define MOJO_BLITZ_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ═══════════════════════════════════════════════════════════════════════════
 * Opaque context handle
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Opaque handle to the Blitz application context.
 * Created by mblitz_create(), destroyed by mblitz_destroy().
 * All other functions take this as their first argument.
 */
typedef void *MblitzContext;

/* ═══════════════════════════════════════════════════════════════════════════
 * Event types — matches mojo-gui/core event type constants
 * ═══════════════════════════════════════════════════════════════════════════ */

#define MBLITZ_EVT_CLICK       0
#define MBLITZ_EVT_INPUT       1
#define MBLITZ_EVT_CHANGE      2
#define MBLITZ_EVT_KEYDOWN     3
#define MBLITZ_EVT_KEYUP       4
#define MBLITZ_EVT_FOCUS       5
#define MBLITZ_EVT_BLUR        6
#define MBLITZ_EVT_SUBMIT      7
#define MBLITZ_EVT_MOUSEDOWN   8
#define MBLITZ_EVT_MOUSEUP     9
#define MBLITZ_EVT_MOUSEMOVE  10

/* ═══════════════════════════════════════════════════════════════════════════
 * Event structure — returned by mblitz_poll_event()
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * A buffered DOM event ready for dispatch to the Mojo app.
 *
 * Fields:
 *   valid       — 1 if this event is valid, 0 if the queue is empty.
 *   handler_id  — The handler ID registered via mblitz_set_event_handler().
 *                 Corresponds to the handler_id from OP_NEW_EVENT_LISTENER.
 *   event_type  — One of the MBLITZ_EVT_* constants above.
 *   value_ptr   — Pointer to a UTF-8 string value (e.g., input text).
 *                 NULL if the event has no string payload.
 *                 Valid only until the next call to mblitz_poll_event().
 *   value_len   — Length of the value string in bytes (0 if no value).
 */
typedef struct {
    int32_t  valid;
    uint32_t handler_id;
    uint8_t  event_type;
    const char *value_ptr;
    uint32_t value_len;
} MblitzEvent;

/* ═══════════════════════════════════════════════════════════════════════════
 * Lifecycle — create, step, destroy
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Create a Blitz application context with a window.
 *
 * Initializes the Winit event loop, creates a native window with the
 * specified title and dimensions, and sets up the Blitz document and
 * rendering pipeline.
 *
 * @param title      Window title (UTF-8, not null-terminated).
 * @param title_len  Length of the title string in bytes.
 * @param width      Initial window width in logical pixels.
 * @param height     Initial window height in logical pixels.
 * @param debug      Non-zero to enable debug overlays (layout borders, etc.).
 * @return           Opaque context handle, or NULL on failure.
 */
MblitzContext mblitz_create(const char *title, uint32_t title_len,
                            uint32_t width, uint32_t height,
                            int32_t debug);

/**
 * Create a headless Blitz context (no event loop, no window, no GPU).
 *
 * Used for integration testing — DOM operations work normally but
 * windowing/rendering functions are no-ops. Events can be injected via
 * mblitz_inject_event() and polled via mblitz_poll_event().
 *
 * @param width   Viewport width in logical pixels (used for layout).
 * @param height  Viewport height in logical pixels (used for layout).
 * @return        Opaque context handle, or NULL on failure.
 */
MblitzContext mblitz_create_headless(uint32_t width, uint32_t height);

/**
 * Process pending window/input events.
 *
 * Drives the Winit event loop for one iteration. User interaction events
 * (clicks, keyboard, etc.) are captured and buffered internally for
 * retrieval via mblitz_poll_event().
 *
 * @param ctx      Context handle from mblitz_create().
 * @param blocking Non-zero to block until an event arrives (use when idle).
 *                 Zero to return immediately after processing pending events.
 * @return         1 if events were processed, 0 if nothing happened.
 */
int32_t mblitz_step(MblitzContext ctx, int32_t blocking);

/**
 * Check if the window is still open.
 *
 * @param ctx  Context handle from mblitz_create().
 * @return     1 if the window is alive, 0 if it was closed.
 */
int32_t mblitz_is_alive(MblitzContext ctx);

/**
 * Request a redraw of the window.
 *
 * Call this after applying mutations to trigger re-layout and re-paint.
 * Blitz will resolve styles, compute layout (Taffy), and paint (Vello)
 * during the next step.
 *
 * @param ctx  Context handle from mblitz_create().
 */
void mblitz_request_redraw(MblitzContext ctx);

/**
 * Destroy the Blitz application context and close the window.
 *
 * Frees all resources: DOM tree, style engine, layout caches, GPU
 * resources, and the Winit window. The context handle is invalid after
 * this call.
 *
 * @param ctx  Context handle from mblitz_create().
 */
void mblitz_destroy(MblitzContext ctx);

/* ═══════════════════════════════════════════════════════════════════════════
 * Window management
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Set the window title.
 *
 * @param ctx        Context handle.
 * @param title      New title (UTF-8, not null-terminated).
 * @param title_len  Length of the title string in bytes.
 */
void mblitz_set_title(MblitzContext ctx, const char *title, uint32_t title_len);

/**
 * Resize the window.
 *
 * @param ctx     Context handle.
 * @param width   New width in logical pixels.
 * @param height  New height in logical pixels.
 */
void mblitz_set_size(MblitzContext ctx, uint32_t width, uint32_t height);

/* ═══════════════════════════════════════════════════════════════════════════
 * User-agent stylesheet
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Add a user-agent stylesheet (CSS string).
 *
 * Blitz includes a default UA stylesheet, but the mojo-gui framework may
 * want to add additional base styles (e.g., for consistent rendering of
 * the HTML elements used by the DSL).
 *
 * @param ctx      Context handle.
 * @param css      CSS source (UTF-8, not null-terminated).
 * @param css_len  Length of the CSS string in bytes.
 */
void mblitz_add_ua_stylesheet(MblitzContext ctx, const char *css, uint32_t css_len);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM node creation
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Create an HTML element node.
 *
 * The element is created but NOT inserted into the tree. Use
 * mblitz_append_children() or mblitz_insert_before() to attach it.
 *
 * @param ctx      Context handle.
 * @param tag      HTML tag name (UTF-8, e.g. "div", "button").
 * @param tag_len  Length of the tag string in bytes.
 * @return         Node ID of the new element. 0 on failure.
 */
uint32_t mblitz_create_element(MblitzContext ctx,
                               const char *tag, uint32_t tag_len);

/**
 * Create a text node.
 *
 * @param ctx       Context handle.
 * @param text      Text content (UTF-8, not null-terminated).
 * @param text_len  Length of the text in bytes.
 * @return          Node ID of the new text node. 0 on failure.
 */
uint32_t mblitz_create_text_node(MblitzContext ctx,
                                 const char *text, uint32_t text_len);

/**
 * Create a comment/placeholder node.
 *
 * Used for conditional rendering placeholders (ConditionalSlot) and
 * other framework-internal markers.
 *
 * @param ctx  Context handle.
 * @return     Node ID of the new comment node. 0 on failure.
 */
uint32_t mblitz_create_placeholder(MblitzContext ctx);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM node cloning (templates)
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Register a template as a pre-built DOM subtree for efficient cloning.
 *
 * The shim builds the template's static structure as real Blitz DOM nodes
 * (but does NOT insert them into the document tree). Subsequent calls to
 * mblitz_clone_template() deep-clone the template subtree.
 *
 * @param ctx      Context handle.
 * @param tmpl_id  Template ID (from mojo-gui/core's template registry).
 * @param root_id  Node ID of the root of the pre-built template subtree.
 *                 This node must have been created via mblitz_create_element()
 *                 and populated with children/attributes but NOT appended to
 *                 the document tree.
 */
void mblitz_register_template(MblitzContext ctx,
                              uint32_t tmpl_id, uint32_t root_id);

/**
 * Deep-clone a registered template.
 *
 * Creates a complete deep copy of the template subtree (all children,
 * attributes, text content). The cloned tree is NOT inserted into the
 * document — use mblitz_append_children() or similar to attach it.
 *
 * @param ctx      Context handle.
 * @param tmpl_id  Template ID (previously registered).
 * @return         Node ID of the cloned root element. 0 if the template
 *                 ID is not registered.
 */
uint32_t mblitz_clone_template(MblitzContext ctx, uint32_t tmpl_id);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM tree mutations
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Append one or more child nodes to a parent element.
 *
 * @param ctx         Context handle.
 * @param parent_id   Node ID of the parent element.
 * @param child_ids   Array of child node IDs to append (in order).
 * @param child_count Number of child IDs in the array.
 */
void mblitz_append_children(MblitzContext ctx,
                            uint32_t parent_id,
                            const uint32_t *child_ids, uint32_t child_count);

/**
 * Insert nodes before an anchor node.
 *
 * @param ctx          Context handle.
 * @param anchor_id    Node ID of the anchor (existing child in the parent).
 * @param new_ids      Array of node IDs to insert before the anchor.
 * @param new_count    Number of node IDs in the array.
 */
void mblitz_insert_before(MblitzContext ctx,
                          uint32_t anchor_id,
                          const uint32_t *new_ids, uint32_t new_count);

/**
 * Insert nodes after an anchor node.
 *
 * @param ctx          Context handle.
 * @param anchor_id    Node ID of the anchor (existing child in the parent).
 * @param new_ids      Array of node IDs to insert after the anchor.
 * @param new_count    Number of node IDs in the array.
 */
void mblitz_insert_after(MblitzContext ctx,
                         uint32_t anchor_id,
                         const uint32_t *new_ids, uint32_t new_count);

/**
 * Replace a node with one or more new nodes.
 *
 * The old node is removed and dropped. The new nodes take its place
 * in the parent's child list.
 *
 * @param ctx          Context handle.
 * @param old_id       Node ID to replace.
 * @param new_ids      Array of replacement node IDs.
 * @param new_count    Number of replacement node IDs.
 */
void mblitz_replace_with(MblitzContext ctx,
                         uint32_t old_id,
                         const uint32_t *new_ids, uint32_t new_count);

/**
 * Remove a node from the DOM tree and drop it.
 *
 * The node and all its descendants are removed and freed.
 *
 * @param ctx      Context handle.
 * @param node_id  Node ID to remove.
 */
void mblitz_remove_node(MblitzContext ctx, uint32_t node_id);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM node attributes
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Set an attribute on an element.
 *
 * If the attribute already exists, its value is updated. Triggers style
 * invalidation in Blitz (Stylo) for attributes that affect rendering
 * (class, style, id, etc.).
 *
 * @param ctx        Context handle.
 * @param node_id    Node ID of the target element.
 * @param name       Attribute name (UTF-8, e.g. "class", "style", "id").
 * @param name_len   Length of the name string in bytes.
 * @param value      Attribute value (UTF-8).
 * @param value_len  Length of the value string in bytes.
 */
void mblitz_set_attribute(MblitzContext ctx, uint32_t node_id,
                          const char *name, uint32_t name_len,
                          const char *value, uint32_t value_len);

/**
 * Remove an attribute from an element.
 *
 * @param ctx       Context handle.
 * @param node_id   Node ID of the target element.
 * @param name      Attribute name (UTF-8).
 * @param name_len  Length of the name string in bytes.
 */
void mblitz_remove_attribute(MblitzContext ctx, uint32_t node_id,
                             const char *name, uint32_t name_len);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM text content
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Set the text content of a text node.
 *
 * @param ctx       Context handle.
 * @param node_id   Node ID of the text node.
 * @param text      New text content (UTF-8).
 * @param text_len  Length of the text in bytes.
 */
void mblitz_set_text_content(MblitzContext ctx, uint32_t node_id,
                             const char *text, uint32_t text_len);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM tree traversal (for AssignId / path-based node lookup)
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Get the child node at the given path from a starting node.
 *
 * The path is an array of child indices. For example, path [0, 2, 1]
 * means: starting from `start_id`, go to child 0, then child 2 of that,
 * then child 1 of that.
 *
 * This is used by the mutation interpreter to resolve ASSIGN_ID and
 * REPLACE_PLACEHOLDER paths within template instances.
 *
 * @param ctx        Context handle.
 * @param start_id   Node ID to start traversal from.
 * @param path       Array of child indices.
 * @param path_len   Length of the path array.
 * @return           Node ID at the end of the path. 0 on failure (invalid
 *                   index at any step).
 */
uint32_t mblitz_node_at_path(MblitzContext ctx, uint32_t start_id,
                             const uint8_t *path, uint32_t path_len);

/**
 * Get the Nth child of a node.
 *
 * @param ctx       Context handle.
 * @param node_id   Parent node ID.
 * @param index     Zero-based child index.
 * @return          Node ID of the child. 0 if the index is out of bounds.
 */
uint32_t mblitz_child_at(MblitzContext ctx, uint32_t node_id, uint32_t index);

/**
 * Get the number of children of a node.
 *
 * @param ctx      Context handle.
 * @param node_id  Node ID.
 * @return         Number of children.
 */
uint32_t mblitz_child_count(MblitzContext ctx, uint32_t node_id);

/* ═══════════════════════════════════════════════════════════════════════════
 * Event handling
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Register an event handler on a node.
 *
 * When a matching event occurs on this node, an MblitzEvent with the
 * given handler_id will be placed in the event ring buffer, retrievable
 * via mblitz_poll_event().
 *
 * Event type names follow the DOM convention: "click", "input", "change",
 * "keydown", "keyup", "focus", "blur", "submit", "mousedown", "mouseup",
 * "mousemove".
 *
 * @param ctx           Context handle.
 * @param node_id       Node ID of the target element.
 * @param handler_id    Unique handler ID (from mojo-gui HandlerRegistry).
 * @param event_name    Event type name (UTF-8, e.g. "click").
 * @param event_name_len Length of the event name in bytes.
 */
void mblitz_add_event_listener(MblitzContext ctx, uint32_t node_id,
                               uint32_t handler_id,
                               const char *event_name, uint32_t event_name_len);

/**
 * Remove an event handler from a node.
 *
 * @param ctx            Context handle.
 * @param node_id        Node ID of the target element.
 * @param event_name     Event type name (UTF-8).
 * @param event_name_len Length of the event name in bytes.
 */
void mblitz_remove_event_listener(MblitzContext ctx, uint32_t node_id,
                                  const char *event_name,
                                  uint32_t event_name_len);

/**
 * Poll the next event from the ring buffer.
 *
 * Returns the next buffered event, or an event with valid=0 if the
 * buffer is empty. Events are consumed (removed) by polling.
 *
 * The value_ptr in the returned event is valid only until the next call
 * to mblitz_poll_event() or mblitz_step(). Copy the string if you need
 * to keep it.
 *
 * @param ctx  Context handle.
 * @return     The next event, or { .valid = 0 } if empty.
 */
MblitzEvent mblitz_poll_event(MblitzContext ctx);

/**
 * Get the number of buffered events.
 *
 * @param ctx  Context handle.
 * @return     Number of events waiting to be polled.
 */
uint32_t mblitz_event_count(MblitzContext ctx);

/**
 * Clear all buffered events.
 *
 * @param ctx  Context handle.
 */
void mblitz_event_clear(MblitzContext ctx);

/* ═══════════════════════════════════════════════════════════════════════════
 * Layout & rendering
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Force a synchronous layout computation.
 *
 * Normally, layout is computed automatically during the render step
 * triggered by mblitz_request_redraw(). Call this if you need layout
 * information (e.g., node positions) before the next render.
 *
 * @param ctx  Context handle.
 */
void mblitz_resolve_layout(MblitzContext ctx);

/* ═══════════════════════════════════════════════════════════════════════════
 * Mutation buffer — batch application of binary opcodes
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Begin a mutation batch.
 *
 * Acquires a DocumentMutator for the document. All DOM mutations between
 * mblitz_begin_mutations() and mblitz_end_mutations() are batched and
 * flushed together, which is more efficient than individual calls.
 *
 * This is the recommended way to apply mutations from the binary opcode
 * buffer. The Mojo-side mutation interpreter reads opcodes and calls the
 * individual DOM functions (create_element, set_attribute, etc.) within
 * a begin/end pair.
 *
 * @param ctx  Context handle.
 */
void mblitz_begin_mutations(MblitzContext ctx);

/**
 * End a mutation batch and flush deferred operations.
 *
 * Drops the DocumentMutator, which triggers:
 *   - Processing of <style> elements
 *   - Loading of linked stylesheets, images, fonts
 *   - Window title updates from <title> elements
 *   - Form association updates
 *   - Autofocus processing
 *
 * After this call, call mblitz_request_redraw() to trigger re-layout
 * and re-paint.
 *
 * @param ctx  Context handle.
 */
void mblitz_end_mutations(MblitzContext ctx);

/* ═══════════════════════════════════════════════════════════════════════════
 * Document root access
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Get the document root node ID.
 *
 * This is the root of the Blitz DOM tree (analogous to the HTML document
 * root). The mojo-gui framework appends its content as children of this
 * node (via OP_APPEND_CHILDREN with target ID 0).
 *
 * In the mojo-gui mutation protocol, element ID 0 refers to the root
 * mount point. The shim maps this to the Blitz document's root element
 * (or creates an implicit <html><body> structure).
 *
 * @param ctx  Context handle.
 * @return     Node ID of the document root (typically 0 or 1).
 */
uint32_t mblitz_root_node_id(MblitzContext ctx);

/**
 * Get the mount point node ID.
 *
 * Returns the node ID that corresponds to mojo-gui's element ID 0
 * (the root mount point where the app's DOM tree is attached). This
 * is typically the <body> element or a <div id="root"> container.
 *
 * @param ctx  Context handle.
 * @return     Node ID of the mount point.
 */
uint32_t mblitz_mount_point_id(MblitzContext ctx);

/* ═══════════════════════════════════════════════════════════════════════════
 * Debug / diagnostics
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Print the DOM tree to stderr (for debugging).
 *
 * @param ctx  Context handle.
 */
void mblitz_print_tree(MblitzContext ctx);

/**
 * Toggle debug overlay visibility (layout borders, node IDs, etc.).
 *
 * @param ctx      Context handle.
 * @param enabled  Non-zero to enable, zero to disable.
 */
void mblitz_set_debug_overlay(MblitzContext ctx, int32_t enabled);

/**
 * Get the Blitz library version string.
 *
 * @param out_ptr  Pointer to receive the version string pointer (UTF-8,
 *                 null-terminated, statically allocated).
 * @param out_len  Pointer to receive the version string length.
 */
void mblitz_version(const char **out_ptr, uint32_t *out_len);

/* ═══════════════════════════════════════════════════════════════════════════
 * DOM inspection (for testing)
 * ═══════════════════════════════════════════════════════════════════════════ */

/**
 * Get the tag name of an element by mojo-gui element ID.
 *
 * Writes the tag name (e.g. "div", "h1", "button") to the output buffer.
 * For text nodes returns "#text", for comments returns "#comment".
 * Returns 0 if the node doesn't exist.
 *
 * If the buffer is too small, the required length is still returned
 * (caller can retry with a larger buffer).
 *
 * @param ctx           Context handle.
 * @param mojo_id       The mojo-gui element ID.
 * @param out_ptr       Buffer to write the tag string (UTF-8, not null-terminated).
 * @param out_capacity  Size of the output buffer in bytes.
 * @return              Number of bytes in the tag string, or 0 if not found.
 */
uint32_t mblitz_get_node_tag(MblitzContext ctx, uint32_t mojo_id,
                             char *out_ptr, uint32_t out_capacity);

/**
 * Get the concatenated text content of a node and its descendants.
 *
 * For element nodes, recursively collects all descendant text.
 * For text nodes, returns the text directly.
 *
 * @param ctx           Context handle.
 * @param mojo_id       The mojo-gui element ID.
 * @param out_ptr       Buffer to write the text (UTF-8, not null-terminated).
 * @param out_capacity  Size of the output buffer in bytes.
 * @return              Number of bytes in the text, or 0 if empty/not found.
 */
uint32_t mblitz_get_text_content(MblitzContext ctx, uint32_t mojo_id,
                                 char *out_ptr, uint32_t out_capacity);

/**
 * Get the value of an attribute on an element.
 *
 * @param ctx            Context handle.
 * @param mojo_id        The mojo-gui element ID.
 * @param attr_name      Attribute name (UTF-8, not null-terminated).
 * @param attr_name_len  Length of the attribute name in bytes.
 * @param out_ptr        Buffer to write the attribute value (UTF-8).
 * @param out_capacity   Size of the output buffer in bytes.
 * @return               Number of bytes in the value, or 0 if not found.
 */
uint32_t mblitz_get_attribute_value(MblitzContext ctx, uint32_t mojo_id,
                                    const char *attr_name, uint32_t attr_name_len,
                                    char *out_ptr, uint32_t out_capacity);

/**
 * Serialize a subtree rooted at a mojo-gui element ID into a compact
 * HTML-like string for test assertions.
 *
 * Format example:
 *   <div><h1>#text("Hello")</h1><button>#text("Click")</button></div>
 *
 * Comment/placeholder nodes render as "<!---->".
 *
 * @param ctx           Context handle.
 * @param mojo_id       The mojo-gui element ID of the subtree root.
 * @param out_ptr       Buffer to write the serialized HTML (UTF-8).
 * @param out_capacity  Size of the output buffer in bytes.
 * @return              Number of bytes written, or required length if too small.
 */
uint32_t mblitz_serialize_subtree(MblitzContext ctx, uint32_t mojo_id,
                                  char *out_ptr, uint32_t out_capacity);

/**
 * Inject a synthetic event into the event queue for testing.
 *
 * The event will be returned by the next mblitz_poll_event() call.
 * This allows tests to simulate user interactions without a real window.
 *
 * @param ctx         Context handle.
 * @param handler_id  The handler ID to dispatch to.
 * @param event_type  One of the MBLITZ_EVT_* constants.
 * @param value_ptr   String payload (UTF-8, not null-terminated). NULL for no value.
 * @param value_len   Length of the value string in bytes. 0 for no value.
 */
void mblitz_inject_event(MblitzContext ctx, uint32_t handler_id,
                         uint8_t event_type,
                         const char *value_ptr, uint32_t value_len);

/**
 * Get the mojo-gui element ID of a child at a given index within a parent.
 *
 * @param ctx              Context handle.
 * @param parent_mojo_id   The mojo-gui element ID of the parent node.
 * @param index            Zero-based child index.
 * @return                 The child's mojo-gui element ID, or UINT32_MAX if
 *                         out of bounds or unmapped.
 */
uint32_t mblitz_get_child_mojo_id(MblitzContext ctx, uint32_t parent_mojo_id,
                                  uint32_t index);

#ifdef __cplusplus
}
#endif

#endif /* MOJO_BLITZ_H */