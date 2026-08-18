/*
 * mojo_xr.h — C API for the mojo-gui XR renderer (OpenXR + Blitz)
 *
 * This header defines the flat C ABI between Mojo FFI bindings
 * (xr/native/src/xr/xr_blitz.mojo) and the Rust cdylib shim
 * (xr/native/shim/src/lib.rs).
 *
 * The XR shim extends the desktop Blitz renderer with:
 *   - Multi-document support (one Blitz DOM per XR panel)
 *   - Offscreen Vello rendering to GPU textures (per panel)
 *   - OpenXR session lifecycle and frame loop
 *   - Quad layer compositing (panel textures → OpenXR swapchain)
 *   - Controller pose tracking and panel raycasting
 *
 * Architecture:
 *
 *   Mojo (xr_blitz.mojo)
 *     │ DLHandle FFI calls (this header)
 *     ▼
 *   libmojo_xr (Rust cdylib)
 *     │ Rust API calls
 *     ├── blitz-dom   — Per-panel DOM tree
 *     ├── Stylo       — CSS parsing & style resolution
 *     ├── Taffy       — Flexbox, grid, block layout
 *     ├── Vello       — GPU rendering → offscreen textures
 *     ├── wgpu        — GPU abstraction (Vulkan/Metal/DX12)
 *     ├── openxr      — XR session, swapchain, input, reference spaces
 *     └── AccessKit   — Accessibility (per-panel)
 *
 * The API follows the same conventions as the desktop shim (mojo_blitz.h):
 *   - Opaque context pointer (MxrSession*)
 *   - Polling-based event model (no callbacks across FFI)
 *   - Internal ID mapping (mojo element IDs ↔ Blitz node IDs, per panel)
 *   - Mutation batching (begin/end) per panel
 *   - Flat C ABI — no C++ types, no templates, no exceptions
 *
 * Ownership:
 *   - The Mojo side owns the GuiApp instances and mutation buffers.
 *   - The Rust side owns the OpenXR session, GPU resources, Blitz
 *     documents, and offscreen textures.
 *   - Panel IDs are assigned by the Rust side and returned to Mojo.
 *
 * Thread model:
 *   - All functions must be called from the same thread that created
 *     the session (the XR/render thread). This matches the OpenXR
 *     requirement that session calls happen on a single thread.
 */

#ifndef MOJO_XR_H
#define MOJO_XR_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif


/* ══════════════════════════════════════════════════════════════════════════════
 * Opaque types
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Opaque handle to the XR session context.
 *
 * Owns the OpenXR instance, session, swapchain, wgpu device, Vello
 * renderer, and all panel state. Created by mxr_create_session() and
 * destroyed by mxr_destroy_session().
 */
typedef struct MxrSessionImpl *MxrSession;


/* ══════════════════════════════════════════════════════════════════════════════
 * Event types — same values as the desktop shim (mojo_blitz.h) for
 * consistency. XR-specific event types start at 0x80.
 * ══════════════════════════════════════════════════════════════════════════════ */

/* Standard DOM event types (shared with desktop renderer) */
#define MXR_EVT_CLICK       1
#define MXR_EVT_INPUT       2
#define MXR_EVT_CHANGE      3
#define MXR_EVT_KEYDOWN     4
#define MXR_EVT_KEYUP       5
#define MXR_EVT_FOCUS       6
#define MXR_EVT_BLUR        7
#define MXR_EVT_SUBMIT      8
#define MXR_EVT_MOUSEDOWN   9
#define MXR_EVT_MOUSEUP     10
#define MXR_EVT_MOUSEMOVE   11

/* XR-specific event types */
#define MXR_EVT_XR_SELECT       0x80  /* Controller trigger / hand pinch */
#define MXR_EVT_XR_SQUEEZE      0x81  /* Controller grip / hand grab */
#define MXR_EVT_XR_HOVER_ENTER  0x82  /* Pointer ray enters a panel */
#define MXR_EVT_XR_HOVER_EXIT   0x83  /* Pointer ray leaves a panel */


/* ══════════════════════════════════════════════════════════════════════════════
 * Hand/controller identifiers
 * ══════════════════════════════════════════════════════════════════════════════ */

#define MXR_HAND_LEFT   0
#define MXR_HAND_RIGHT  1
#define MXR_HAND_HEAD   2  /* Gaze-based input (head pointer) */


/* ══════════════════════════════════════════════════════════════════════════════
 * Reference space types
 * ══════════════════════════════════════════════════════════════════════════════ */

#define MXR_SPACE_LOCAL     0  /* Head-relative (seated) */
#define MXR_SPACE_STAGE     1  /* Room-scale floor origin */
#define MXR_SPACE_VIEW      2  /* Eye-relative (HMD view) */
#define MXR_SPACE_UNBOUNDED 3  /* Large-scale (if supported) */


/* ══════════════════════════════════════════════════════════════════════════════
 * Session state (mirrors XrSessionState)
 * ══════════════════════════════════════════════════════════════════════════════ */

#define MXR_STATE_IDLE       0
#define MXR_STATE_READY      1
#define MXR_STATE_FOCUSED    2
#define MXR_STATE_VISIBLE    3
#define MXR_STATE_STOPPING   4
#define MXR_STATE_EXITING    5


/* ══════════════════════════════════════════════════════════════════════════════
 * XR event struct — extends the desktop BlitzEvent with panel targeting
 * and XR-specific hit information.
 * ══════════════════════════════════════════════════════════════════════════════ */

typedef struct {
    /* Non-zero if this event contains valid data. */
    int32_t valid;

    /* Panel this event targets (assigned by mxr_create_panel). */
    uint32_t panel_id;

    /* Handler ID in the panel's HandlerRegistry (from the mutation protocol). */
    uint32_t handler_id;

    /* Event type tag (MXR_EVT_CLICK, MXR_EVT_INPUT, etc.). */
    uint8_t event_type;

    /* String payload (e.g., input field value). NULL if not applicable.
     * The pointer is valid until the next mxr_poll_event() call. */
    const char *value_ptr;
    uint32_t value_len;

    /* Panel-local hit coordinates (0.0–1.0 UV range).
     * (0,0) = top-left, (1,1) = bottom-right.
     * Set to -1.0 if not a pointer event. */
    float hit_u;
    float hit_v;

    /* Which hand/controller produced this event (MXR_HAND_*). */
    uint8_t hand;
} MxrEvent;


/* ══════════════════════════════════════════════════════════════════════════════
 * Pose struct — position + orientation in 3D space
 * ══════════════════════════════════════════════════════════════════════════════ */

typedef struct {
    /* Position (meters, in the active reference space). */
    float px, py, pz;

    /* Orientation as a unit quaternion (x, y, z, w). */
    float qx, qy, qz, qw;

    /* Non-zero if the pose is valid (tracking is active). */
    int32_t valid;
} MxrPose;


/* ══════════════════════════════════════════════════════════════════════════════
 * Raycast hit result
 * ══════════════════════════════════════════════════════════════════════════════ */

typedef struct {
    /* Non-zero if a panel was hit. */
    int32_t hit;

    /* ID of the hit panel. */
    uint32_t panel_id;

    /* Hit point in panel-local UV coordinates (0.0–1.0). */
    float u, v;

    /* Distance from ray origin to hit point, in meters. */
    float distance;
} MxrRaycastHit;


/* ══════════════════════════════════════════════════════════════════════════════
 * Session lifecycle
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Create an XR session with the default OpenXR runtime.
 *
 * Initializes the OpenXR instance, creates a session with Vulkan/GL
 * graphics binding, allocates the wgpu device, and creates the Vello
 * renderer. The session starts in IDLE state; panels can be created
 * immediately.
 *
 * Parameters:
 *   app_name     — Application name (shown in OpenXR runtime UI).
 *                  UTF-8 string, null-terminated.
 *   app_name_len — Length of app_name in bytes (excluding null).
 *
 * Returns:
 *   Opaque session handle, or NULL if the OpenXR runtime is unavailable
 *   or initialization fails.
 */
MxrSession mxr_create_session(const char *app_name, uint32_t app_name_len);

/*
 * Create a headless XR session for testing (no OpenXR runtime needed).
 *
 * Allocates Blitz documents and performs DOM operations, but does not
 * create an OpenXR instance or GPU resources. Useful for integration
 * tests and CI environments.
 *
 * Returns:
 *   Opaque session handle (never NULL — headless always succeeds).
 */
MxrSession mxr_create_headless(void);

/*
 * Query the current session state.
 *
 * Returns one of the MXR_STATE_* constants.
 */
int32_t mxr_session_state(MxrSession session);

/*
 * Check if the session is still alive (not exiting or destroyed).
 *
 * Returns non-zero if the session is alive (state < MXR_STATE_EXITING).
 */
int32_t mxr_is_alive(MxrSession session);

/*
 * Destroy the XR session and release all resources.
 *
 * Destroys all panels, GPU textures, the Vello renderer, wgpu device,
 * OpenXR session, and OpenXR instance. The session handle is invalid
 * after this call.
 */
void mxr_destroy_session(MxrSession session);


/* ══════════════════════════════════════════════════════════════════════════════
 * Panel lifecycle
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Create a new XR panel with the given pixel dimensions.
 *
 * Allocates a new Blitz document, an offscreen wgpu texture, and a
 * mutation interpreter for this panel. The panel starts visible and
 * unmounted (no DOM content until mutations are applied).
 *
 * The physical size (in meters) and placement are set separately via
 * mxr_panel_set_transform(). The pixel dimensions determine the texture
 * resolution and the Blitz document's viewport size.
 *
 * Parameters:
 *   session      — The XR session.
 *   width_px     — Texture width in pixels.
 *   height_px    — Texture height in pixels.
 *
 * Returns:
 *   Panel ID (non-zero), or 0 on failure.
 */
uint32_t mxr_create_panel(MxrSession session,
                           uint32_t width_px, uint32_t height_px);

/*
 * Destroy a panel and free its Blitz document and GPU texture.
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Panel to destroy.
 */
void mxr_destroy_panel(MxrSession session, uint32_t panel_id);

/*
 * Set a panel's 3D transform (position + orientation) in world space.
 *
 * The transform is specified in the session's reference space (default:
 * STAGE). Position is in meters; rotation is a unit quaternion (x,y,z,w).
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Panel to transform.
 *   px,py,pz — Position of the panel center, in meters.
 *   qx,qy,qz,qw — Orientation as a unit quaternion.
 */
void mxr_panel_set_transform(MxrSession session, uint32_t panel_id,
                              float px, float py, float pz,
                              float qx, float qy, float qz, float qw);

/*
 * Set a panel's physical size in meters.
 *
 * This determines how large the panel appears in the XR scene. It does
 * NOT change the texture resolution (set at creation time).
 *
 * Parameters:
 *   session   — The XR session.
 *   panel_id  — Panel to resize.
 *   width_m   — Physical width in meters.
 *   height_m  — Physical height in meters.
 */
void mxr_panel_set_size(MxrSession session, uint32_t panel_id,
                         float width_m, float height_m);

/*
 * Show or hide a panel.
 *
 * Hidden panels are not rendered, not raycasted, and not submitted
 * as quad layers. They retain their DOM state.
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Panel to show/hide.
 *   visible  — Non-zero to show, zero to hide.
 */
void mxr_panel_set_visible(MxrSession session, uint32_t panel_id,
                            int32_t visible);

/*
 * Query whether a panel is visible.
 *
 * Returns non-zero if visible.
 */
int32_t mxr_panel_is_visible(MxrSession session, uint32_t panel_id);

/*
 * Set the curved display flag and curvature radius for a panel.
 *
 * When curved, the panel is rendered as a cylindrical surface instead
 * of a flat quad. The radius is in meters.
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Panel to configure.
 *   curved   — Non-zero for curved, zero for flat.
 *   radius   — Curvature radius in meters (ignored if curved=0).
 */
void mxr_panel_set_curved(MxrSession session, uint32_t panel_id,
                           int32_t curved, float radius);

/*
 * Query the number of active panels.
 */
uint32_t mxr_panel_count(MxrSession session);

/*
 * Add a user-agent stylesheet to a panel's Blitz document.
 *
 * Should be called before applying mount mutations, so the stylesheet
 * is available during initial style resolution.
 *
 * Parameters:
 *   session      — The XR session.
 *   panel_id     — Panel to style.
 *   css_ptr      — UTF-8 CSS text.
 *   css_len      — Length of the CSS text in bytes.
 */
void mxr_panel_add_ua_stylesheet(MxrSession session, uint32_t panel_id,
                                  const char *css_ptr, uint32_t css_len);


/* ══════════════════════════════════════════════════════════════════════════════
 * Mutations — apply the binary mutation protocol to a panel's DOM
 *
 * These functions operate on a specific panel's Blitz document. The
 * binary protocol is identical to the desktop renderer — the same
 * opcodes (LOAD_TEMPLATE, SET_ATTRIBUTE, SET_TEXT, APPEND_CHILDREN,
 * etc.) are interpreted by the panel's MutationInterpreter.
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Begin a mutation batch for a panel.
 *
 * Must be called before applying mutations. The batch ensures that
 * style resolution and layout are deferred until mxr_panel_end_mutations().
 */
void mxr_panel_begin_mutations(MxrSession session, uint32_t panel_id);

/*
 * Apply a binary mutation buffer to a panel's DOM.
 *
 * The buffer contains the same binary opcodes as the desktop renderer.
 * The shim's per-panel MutationInterpreter reads the opcodes and
 * translates them into Blitz DOM operations.
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Target panel.
 *   buf      — Pointer to the mutation buffer.
 *   len      — Number of valid bytes in the buffer.
 */
void mxr_panel_apply_mutations(MxrSession session, uint32_t panel_id,
                                const uint8_t *buf, uint32_t len);

/*
 * End a mutation batch for a panel.
 *
 * Triggers style resolution and layout computation. Marks the panel's
 * texture as dirty (needs re-render by Vello).
 */
void mxr_panel_end_mutations(MxrSession session, uint32_t panel_id);


/* ══════════════════════════════════════════════════════════════════════════════
 * Per-panel DOM operations — direct element manipulation
 *
 * These mirror the desktop shim's DOM operations (mblitz_create_element,
 * mblitz_set_attribute, etc.) but scoped to a specific panel. They are
 * used by the Mojo-side MutationInterpreter when processing opcodes.
 *
 * In practice, most mutation application goes through
 * mxr_panel_apply_mutations() which uses the shim-internal interpreter.
 * These per-element functions are exposed for testing, debugging, and
 * advanced use cases where Mojo-side interpretation is preferred.
 * ══════════════════════════════════════════════════════════════════════════════ */

uint32_t mxr_panel_create_element(MxrSession session, uint32_t panel_id,
                                   const char *tag_ptr, uint32_t tag_len);

uint32_t mxr_panel_create_text_node(MxrSession session, uint32_t panel_id,
                                     const char *text_ptr, uint32_t text_len);

uint32_t mxr_panel_create_placeholder(MxrSession session, uint32_t panel_id);

void mxr_panel_register_template(MxrSession session, uint32_t panel_id,
                                  const uint8_t *buf, uint32_t len);

uint32_t mxr_panel_clone_template(MxrSession session, uint32_t panel_id,
                                   uint32_t template_id);

void mxr_panel_append_children(MxrSession session, uint32_t panel_id,
                                uint32_t parent_id,
                                const uint32_t *child_ids, uint32_t count);

void mxr_panel_insert_before(MxrSession session, uint32_t panel_id,
                              uint32_t reference_id,
                              const uint32_t *node_ids, uint32_t count);

void mxr_panel_insert_after(MxrSession session, uint32_t panel_id,
                             uint32_t reference_id,
                             const uint32_t *node_ids, uint32_t count);

void mxr_panel_replace_with(MxrSession session, uint32_t panel_id,
                             uint32_t old_id,
                             const uint32_t *new_ids, uint32_t count);

void mxr_panel_remove_node(MxrSession session, uint32_t panel_id,
                            uint32_t node_id);

void mxr_panel_set_attribute(MxrSession session, uint32_t panel_id,
                              uint32_t node_id,
                              const char *name_ptr, uint32_t name_len,
                              const char *value_ptr, uint32_t value_len);

void mxr_panel_remove_attribute(MxrSession session, uint32_t panel_id,
                                 uint32_t node_id,
                                 const char *name_ptr, uint32_t name_len);

void mxr_panel_set_text_content(MxrSession session, uint32_t panel_id,
                                 uint32_t node_id,
                                 const char *text_ptr, uint32_t text_len);

uint32_t mxr_panel_node_at_path(MxrSession session, uint32_t panel_id,
                                 uint32_t root_id,
                                 const uint32_t *path, uint32_t path_len);

uint32_t mxr_panel_child_at(MxrSession session, uint32_t panel_id,
                             uint32_t parent_id, uint32_t index);

uint32_t mxr_panel_child_count(MxrSession session, uint32_t panel_id,
                                uint32_t parent_id);


/* ══════════════════════════════════════════════════════════════════════════════
 * Events — input from XR controllers, dispatched to panel handlers
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Add a DOM event listener to a node in a panel.
 *
 * Registers a handler for the given event type on the specified DOM node.
 * When the event fires (via controller raycast hit), the handler_id is
 * included in the MxrEvent returned by mxr_poll_event().
 *
 * Parameters:
 *   session      — The XR session.
 *   panel_id     — Panel containing the target node.
 *   node_id      — DOM node to listen on.
 *   handler_id   — Mojo-side handler ID (from HandlerRegistry).
 *   event_type   — Event type (MXR_EVT_CLICK, etc.).
 */
void mxr_panel_add_event_listener(MxrSession session, uint32_t panel_id,
                                   uint32_t node_id, uint32_t handler_id,
                                   uint8_t event_type);

/*
 * Remove a DOM event listener from a node in a panel.
 */
void mxr_panel_remove_event_listener(MxrSession session, uint32_t panel_id,
                                      uint32_t node_id, uint32_t handler_id,
                                      uint8_t event_type);

/*
 * Poll the next event from the XR input queue.
 *
 * Returns the next buffered event. If no events are pending, returns
 * an event with valid=0. Events are generated by:
 *   - Controller raycasts hitting panel DOM elements
 *   - OpenXR input actions (select, squeeze)
 *   - Keyboard/text input forwarded to the focused panel
 *
 * The event's value_ptr (if non-NULL) is valid until the next call
 * to mxr_poll_event().
 */
MxrEvent mxr_poll_event(MxrSession session);

/*
 * Return the number of pending events.
 */
uint32_t mxr_event_count(MxrSession session);

/*
 * Clear all pending events.
 */
void mxr_event_clear(MxrSession session);


/* ══════════════════════════════════════════════════════════════════════════════
 * Raycasting — controller pointer → panel hit testing
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Raycast against all visible, interactive panels.
 *
 * Tests the given ray against every visible panel's quad (or cylinder
 * for curved panels) and returns the closest hit. If no panel is hit,
 * the result's hit field is 0.
 *
 * This is called each frame by the Mojo-side XR launcher to determine
 * which panel the controller is pointing at, and to translate the hit
 * into DOM pointer events.
 *
 * Parameters:
 *   session — The XR session.
 *   ox,oy,oz — Ray origin in world space (meters).
 *   dx,dy,dz — Ray direction in world space (normalized).
 *
 * Returns:
 *   Raycast hit result.
 */
MxrRaycastHit mxr_raycast_panels(MxrSession session,
                                  float ox, float oy, float oz,
                                  float dx, float dy, float dz);

/*
 * Set the focused panel (receives keyboard/text input).
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Panel to focus, or 0 to clear focus.
 */
void mxr_set_focused_panel(MxrSession session, uint32_t panel_id);

/*
 * Query which panel currently has input focus.
 *
 * Returns the focused panel's ID, or 0 if no panel has focus.
 */
uint32_t mxr_get_focused_panel(MxrSession session);


/* ══════════════════════════════════════════════════════════════════════════════
 * Frame loop — OpenXR frame timing and composition
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Wait for the next frame from the OpenXR runtime.
 *
 * Blocks until the runtime signals that a new frame should be rendered.
 * This synchronizes the app to the HMD's refresh rate. Returns the
 * predicted display time (nanoseconds, OpenXR convention).
 *
 * Must be called once per frame, before mxr_begin_frame().
 *
 * Returns:
 *   Predicted display time in nanoseconds, or 0 if the session is not
 *   in a renderable state.
 */
int64_t mxr_wait_frame(MxrSession session);

/*
 * Begin a new frame.
 *
 * Call after mxr_wait_frame(). Acquires the OpenXR swapchain image
 * and prepares for rendering.
 *
 * Returns non-zero on success, 0 if the frame should be skipped
 * (e.g., session not visible).
 */
int32_t mxr_begin_frame(MxrSession session);

/*
 * Render all dirty panel textures.
 *
 * For each panel marked dirty, runs Vello to re-render the panel's
 * Blitz DOM to its offscreen GPU texture. Clean panels retain their
 * cached texture.
 *
 * Call between mxr_begin_frame() and mxr_end_frame().
 *
 * Returns the number of panels that were re-rendered.
 */
uint32_t mxr_render_dirty_panels(MxrSession session);

/*
 * End the frame and submit composition layers to OpenXR.
 *
 * Submits one quad layer per visible panel to the OpenXR compositor.
 * Each quad layer references the panel's GPU texture and is placed
 * at the panel's world-space transform.
 *
 * Call after mxr_render_dirty_panels().
 */
void mxr_end_frame(MxrSession session);


/* ══════════════════════════════════════════════════════════════════════════════
 * GPU initialisation — Vello offscreen rendering pipeline
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Try to initialise the GPU renderer (wgpu + Vello) for offscreen panel
 * texture rendering.
 *
 * This is intentionally separate from session creation so that headless
 * tests keep working without a GPU. Call once after mxr_create_session()
 * or mxr_create_headless().
 *
 * When GPU is available, mxr_render_dirty_panels() will paint each panel's
 * Blitz DOM to an offscreen GPU texture via Vello. Without GPU, it falls
 * back to layout-only resolution (Stylo + Taffy, no pixel output).
 *
 * Returns:
 *   1 on success (GPU renderer initialised).
 *   0 on failure (no compatible GPU adapter found) or if already initialised.
 *   Idempotent: returns 1 on subsequent calls if the first succeeded.
 */
int32_t mxr_init_gpu(MxrSession session);

/*
 * Check whether the GPU renderer is available.
 *
 * Returns:
 *   1 if mxr_init_gpu() has been called successfully.
 *   0 otherwise (no GPU, not initialised, or null session).
 */
int32_t mxr_has_gpu(MxrSession session);

/*
 * Copy a panel's most-recently-rendered texture to a CPU buffer.
 *
 * The buffer receives RGBA8 pixel data in row-major order (top-left origin).
 * The required buffer size is panel_width * panel_height * 4 bytes.
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — The panel whose texture to read.
 *   buf      — Destination buffer (caller-allocated).
 *   buf_len  — Size of buf in bytes.
 *
 * Returns:
 *   Number of bytes written on success (== width * height * 4).
 *   0 on failure (no GPU, no texture, buffer too small, panel not found).
 */
uint32_t mxr_panel_read_pixels(MxrSession session, uint32_t panel_id,
                                uint8_t *buf, uint32_t buf_len);


/* ══════════════════════════════════════════════════════════════════════════════
 * Input — controller and head pose tracking
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Get the current pose of a controller or the head.
 *
 * Parameters:
 *   session — The XR session.
 *   hand    — MXR_HAND_LEFT, MXR_HAND_RIGHT, or MXR_HAND_HEAD.
 *
 * Returns:
 *   The pose in the session's reference space. If the controller is not
 *   tracked, the pose's valid field is 0.
 */
MxrPose mxr_get_pose(MxrSession session, uint8_t hand);

/*
 * Get the aim ray for a controller (origin + direction from the pose).
 *
 * This is a convenience that returns the controller's aim pose —
 * typically offset from the grip pose to represent the pointing direction.
 * Use this for raycasting rather than the grip pose.
 *
 * Parameters:
 *   session         — The XR session.
 *   hand            — MXR_HAND_LEFT or MXR_HAND_RIGHT.
 *   out_ox, out_oy, out_oz — Output: ray origin (meters).
 *   out_dx, out_dy, out_dz — Output: ray direction (normalized).
 *
 * Returns:
 *   Non-zero if the aim pose is valid.
 */
int32_t mxr_get_aim_ray(MxrSession session, uint8_t hand,
                         float *out_ox, float *out_oy, float *out_oz,
                         float *out_dx, float *out_dy, float *out_dz);


/* ══════════════════════════════════════════════════════════════════════════════
 * Reference spaces
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Set the session's reference space type.
 *
 * All panel transforms and pose queries are relative to this space.
 * Default is MXR_SPACE_STAGE (room-scale floor origin).
 *
 * Parameters:
 *   session    — The XR session.
 *   space_type — MXR_SPACE_LOCAL, MXR_SPACE_STAGE, MXR_SPACE_VIEW,
 *                or MXR_SPACE_UNBOUNDED.
 *
 * Returns:
 *   Non-zero on success, 0 if the space type is not supported by the
 *   runtime.
 */
int32_t mxr_set_reference_space(MxrSession session, uint8_t space_type);

/*
 * Query the current reference space type.
 */
uint8_t mxr_get_reference_space(MxrSession session);


/* ══════════════════════════════════════════════════════════════════════════════
 * Capabilities — runtime feature detection
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Check if a specific OpenXR extension is available.
 *
 * Parameters:
 *   session          — The XR session.
 *   ext_name_ptr     — Extension name (e.g., "XR_EXT_hand_tracking").
 *   ext_name_len     — Length of the name string.
 *
 * Returns:
 *   Non-zero if the extension is available and enabled.
 */
int32_t mxr_has_extension(MxrSession session,
                           const char *ext_name_ptr, uint32_t ext_name_len);

/*
 * Check if hand tracking is available.
 * Shorthand for mxr_has_extension(session, "XR_EXT_hand_tracking", ...).
 */
int32_t mxr_has_hand_tracking(MxrSession session);

/*
 * Check if passthrough (AR) is available.
 * Shorthand for mxr_has_extension(session, "XR_FB_passthrough", ...).
 */
int32_t mxr_has_passthrough(MxrSession session);


/* ══════════════════════════════════════════════════════════════════════════════
 * Debug & inspection
 * ══════════════════════════════════════════════════════════════════════════════ */

/*
 * Print a panel's DOM tree to stderr (for debugging).
 */
void mxr_panel_print_tree(MxrSession session, uint32_t panel_id);

/*
 * Serialize a panel's DOM subtree to a string.
 *
 * Writes the serialized HTML into the provided buffer. Returns the
 * number of bytes written (excluding null terminator), or the required
 * buffer size if the buffer is too small (pass buf=NULL, buf_len=0 to
 * query the size).
 *
 * Parameters:
 *   session  — The XR session.
 *   panel_id — Panel to serialize.
 *   buf      — Output buffer, or NULL to query the required size.
 *   buf_len  — Size of the output buffer.
 *
 * Returns:
 *   Number of bytes written, or required size if buf is too small.
 */
uint32_t mxr_panel_serialize_subtree(MxrSession session, uint32_t panel_id,
                                      char *buf, uint32_t buf_len);

/*
 * Get the tag name of a node in a panel (for testing).
 *
 * Writes the tag name into the provided buffer. Returns the length.
 */
uint32_t mxr_panel_get_node_tag(MxrSession session, uint32_t panel_id,
                                 uint32_t node_id,
                                 char *buf, uint32_t buf_len);

/*
 * Get the text content of a node in a panel (for testing).
 */
uint32_t mxr_panel_get_text_content(MxrSession session, uint32_t panel_id,
                                     uint32_t node_id,
                                     char *buf, uint32_t buf_len);

/*
 * Get the value of an attribute on a node in a panel (for testing).
 */
uint32_t mxr_panel_get_attribute_value(MxrSession session, uint32_t panel_id,
                                        uint32_t node_id,
                                        const char *name_ptr, uint32_t name_len,
                                        char *buf, uint32_t buf_len);

/*
 * Inject a synthetic event into a panel (for testing).
 *
 * Bypasses raycasting and directly enqueues an event for the given
 * panel and handler.
 */
void mxr_panel_inject_event(MxrSession session, uint32_t panel_id,
                             uint32_t handler_id, uint8_t event_type,
                             const char *value_ptr, uint32_t value_len);

/*
 * Get the shim version string.
 *
 * Writes the version string (e.g., "0.1.0") into the provided buffer.
 * Returns the number of bytes written.
 */
uint32_t mxr_version(char *buf, uint32_t buf_len);

/*
 * Query the mount point node ID for a panel.
 *
 * Returns the Blitz-internal node ID of the panel's mount point
 * (typically the <body> element or its first child).
 */
uint32_t mxr_panel_mount_point_id(MxrSession session, uint32_t panel_id);

/*
 * Get the mojo element ID of a child node at a given index (for testing).
 */
uint32_t mxr_panel_get_child_mojo_id(MxrSession session, uint32_t panel_id,
                                      uint32_t parent_id, uint32_t index);

/* ═══════════════════════════════════════════════════════════════════════════
 * ID Mapping (AssignId opcode support)
 * ═══════════════════════════════════════════════════════════════════════════ */

/*
 * Assign a mojo-gui element ID to a Blitz node ID within a panel.
 * Used by the mutation interpreter when processing the AssignId opcode.
 */
void mxr_panel_assign_id(MxrSession session, uint32_t panel_id,
                          uint32_t mojo_id, uint32_t node_id);

/*
 * Resolve a mojo-gui element ID to a Blitz node ID within a panel.
 * Returns 0 if the mojo ID is not mapped.
 */
uint32_t mxr_panel_resolve_id(MxrSession session, uint32_t panel_id,
                                uint32_t mojo_id);

/* ═══════════════════════════════════════════════════════════════════════════
 * Stack Operations (mutation interpreter support)
 * ═══════════════════════════════════════════════════════════════════════════ */

/*
 * Push a node ID onto the panel's interpreter stack.
 */
void mxr_panel_stack_push(MxrSession session, uint32_t panel_id,
                           uint32_t node_id);

/*
 * Pop a node ID from the panel's interpreter stack.
 * Returns 0 if the stack is empty.
 */
uint32_t mxr_panel_stack_pop(MxrSession session, uint32_t panel_id);


/* ═══════════════════════════════════════════════════════════════════════════
 * Output-pointer FFI variants — avoid struct-return ABI issues
 *
 * Mojo's DLHandle cannot reliably return C structs larger than 16 bytes
 * (on x86_64 SysV ABI, large structs use a hidden first pointer that
 * DLHandle doesn't handle). These _into() variants write each field to
 * caller-provided output pointers instead.
 * ═══════════════════════════════════════════════════════════════════════════ */

/*
 * Poll the next event, writing each field to caller-provided output pointers.
 *
 * Returns non-zero if an event was available, 0 if the queue was empty.
 * When returning 0, output pointers are NOT modified.
 *
 * The out_value_ptr / out_value_len pair points into an internal buffer
 * that stays alive until the next mxr_poll_event_into() call.
 *
 * Parameters:
 *   session        — The XR session.
 *   out_panel_id   — Output: panel this event targets.
 *   out_handler_id — Output: handler ID in the panel's HandlerRegistry.
 *   out_event_type — Output: event type tag (MXR_EVT_*).
 *   out_value_ptr  — Output: pointer to string payload (NULL if none).
 *   out_value_len  — Output: length of string payload in bytes.
 *   out_hit_u      — Output: panel-local U coordinate (0.0–1.0, or -1.0).
 *   out_hit_v      — Output: panel-local V coordinate (0.0–1.0, or -1.0).
 *   out_hand       — Output: which hand produced this event (MXR_HAND_*).
 */
int32_t mxr_poll_event_into(MxrSession session,
                             uint32_t *out_panel_id,
                             uint32_t *out_handler_id,
                             uint8_t *out_event_type,
                             const uint8_t **out_value_ptr,
                             uint32_t *out_value_len,
                             float *out_hit_u,
                             float *out_hit_v,
                             uint8_t *out_hand);

/*
 * Raycast against all visible, interactive panels, writing the result
 * to caller-provided output pointers.
 *
 * Returns non-zero if a panel was hit, 0 if the ray missed all panels.
 * When returning 0, output pointers are zeroed.
 *
 * Parameters:
 *   session       — The XR session.
 *   ox,oy,oz      — Ray origin in world space (meters).
 *   dx,dy,dz      — Ray direction in world space (normalized).
 *   out_panel_id  — Output: ID of the hit panel.
 *   out_u         — Output: hit U coordinate (0.0–1.0).
 *   out_v         — Output: hit V coordinate (0.0–1.0).
 *   out_distance  — Output: distance from ray origin to hit point (meters).
 */
int32_t mxr_raycast_panels_into(MxrSession session,
                                 float ox, float oy, float oz,
                                 float dx, float dy, float dz,
                                 uint32_t *out_panel_id,
                                 float *out_u,
                                 float *out_v,
                                 float *out_distance);

/*
 * Get a controller/head pose, writing the result to caller-provided
 * output pointers.
 *
 * Returns non-zero if the pose is valid (tracking active), 0 otherwise.
 * In headless mode, always returns 0 and writes identity pose (position
 * zeroed, quaternion = (0,0,0,1)).
 *
 * Parameters:
 *   session          — The XR session.
 *   hand             — MXR_HAND_LEFT, MXR_HAND_RIGHT, or MXR_HAND_HEAD.
 *   out_px,py,pz     — Output: position in meters.
 *   out_qx,qy,qz,qw — Output: orientation as unit quaternion.
 */
int32_t mxr_get_pose_into(MxrSession session, uint8_t hand,
                           float *out_px, float *out_py, float *out_pz,
                           float *out_qx, float *out_qy, float *out_qz,
                           float *out_qw);


#ifdef __cplusplus
}
#endif

#endif /* MOJO_XR_H */