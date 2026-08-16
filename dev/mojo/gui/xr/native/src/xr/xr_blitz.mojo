# XR Blitz FFI Bindings — Mojo wrappers for the libmojo_xr C shim.
#
# This module provides typed Mojo wrappers around the XR renderer C shim
# (libmojo_xr.so / libmojo_xr.dylib). The shim exposes Blitz's HTML/CSS
# rendering engine (Stylo + Taffy + Vello) combined with OpenXR for XR
# panel rendering via a flat C ABI.
#
# Architecture:
#
#   Mojo (this module)
#     │ DLHandle FFI calls
#     ▼
#   libmojo_xr (Rust cdylib)
#     │ Rust API calls
#     ├── blitz-dom   — Per-panel DOM tree
#     ├── Stylo       — CSS parsing & style resolution
#     ├── Taffy       — Flexbox, grid, block layout
#     ├── Vello       — GPU rendering → offscreen textures
#     ├── wgpu        — GPU abstraction (Vulkan/Metal/DX12)
#     ├── openxr      — XR session, swapchain, input, reference spaces
#     └── AccessKit   — Accessibility (per-panel)
#
# The shim uses a polling-based design (no callbacks across FFI) with an
# internal event ring buffer, matching the pattern established by the
# desktop shim (mojo_blitz.h).
#
# Unlike the desktop renderer which manages a single Blitz document, the
# XR shim manages multiple independent Blitz documents — one per XR panel.
# Every DOM operation takes a panel_id parameter to target the correct
# document.
#
# Library search order:
#   1. MOJO_XR_LIB environment variable (explicit path)
#   2. NIX_LDFLAGS (Nix dev shell library paths)
#   3. LD_LIBRARY_PATH / DYLD_LIBRARY_PATH
#   4. Common system paths (/usr/local/lib, /usr/lib)
#
# Usage:
#
#   var xr = XRBlitz.create_headless()
#   var panel_id = xr.create_panel(960, 720)
#   xr.panel_set_transform(panel_id, 0.0, 1.4, -1.0, 0.0, 0.0, 0.0, 1.0)
#   xr.panel_set_size(panel_id, 0.8, 0.6)
#   xr.panel_begin_mutations(panel_id)
#   # ... apply mutations via interpreter ...
#   xr.panel_end_mutations(panel_id)
#   while xr.is_alive():
#       var predicted_time = xr.wait_frame()
#       _ = xr.begin_frame()
#       _ = xr.render_dirty_panels()
#       xr.end_frame()
#       while True:
#           var event = xr.poll_event()
#           if not event.valid:
#               break
#           # dispatch event to panel's GuiApp.handle_event()
#   xr.destroy()

from memory import UnsafePointer, alloc
from os import getenv
from sys.ffi import _DLHandle


# ══════════════════════════════════════════════════════════════════════════════
# Constants — mirror the #defines in mojo_xr.h
# ══════════════════════════════════════════════════════════════════════════════

# Standard DOM event types (shared with desktop renderer)
comptime EVT_CLICK: UInt8 = 1
comptime EVT_INPUT: UInt8 = 2
comptime EVT_CHANGE: UInt8 = 3
comptime EVT_KEYDOWN: UInt8 = 4
comptime EVT_KEYUP: UInt8 = 5
comptime EVT_FOCUS: UInt8 = 6
comptime EVT_BLUR: UInt8 = 7
comptime EVT_SUBMIT: UInt8 = 8
comptime EVT_MOUSEDOWN: UInt8 = 9
comptime EVT_MOUSEUP: UInt8 = 10
comptime EVT_MOUSEMOVE: UInt8 = 11

# XR-specific event types
comptime EVT_XR_SELECT: UInt8 = 0x80
comptime EVT_XR_SQUEEZE: UInt8 = 0x81
comptime EVT_XR_HOVER_ENTER: UInt8 = 0x82
comptime EVT_XR_HOVER_EXIT: UInt8 = 0x83

# Hand/controller identifiers
comptime HAND_LEFT: UInt8 = 0
comptime HAND_RIGHT: UInt8 = 1
comptime HAND_HEAD: UInt8 = 2

# Reference space types
comptime SPACE_LOCAL: UInt8 = 0
comptime SPACE_STAGE: UInt8 = 1
comptime SPACE_VIEW: UInt8 = 2
comptime SPACE_UNBOUNDED: UInt8 = 3

# Session state (mirrors XrSessionState)
comptime STATE_IDLE: Int32 = 0
comptime STATE_READY: Int32 = 1
comptime STATE_FOCUSED: Int32 = 2
comptime STATE_VISIBLE: Int32 = 3
comptime STATE_STOPPING: Int32 = 4
comptime STATE_EXITING: Int32 = 5


# ══════════════════════════════════════════════════════════════════════════════
# XREvent — A buffered event returned by poll_event()
# ══════════════════════════════════════════════════════════════════════════════


struct XREvent(Copyable, Movable):
    """A buffered XR event from the shim's ring buffer.

    Extends the desktop BlitzEvent with panel targeting and XR-specific
    hit information (UV coordinates, hand identifier).

    Fields:
        valid:      True if this event is valid, False if the queue was empty.
        panel_id:   Panel this event targets.
        handler_id: The handler ID registered via add_event_listener().
        event_type: One of the EVT_* constants.
        value:      String payload (e.g., input field value). Empty if N/A.
        hit_u:      Panel-local U coordinate (0.0–1.0). -1.0 if not a pointer event.
        hit_v:      Panel-local V coordinate (0.0–1.0). -1.0 if not a pointer event.
        hand:       Which hand/controller produced this event (HAND_*).
    """

    var valid: Bool
    var panel_id: UInt32
    var handler_id: UInt32
    var event_type: UInt8
    var value: String
    var hit_u: Float32
    var hit_v: Float32
    var hand: UInt8

    fn __init__(out self):
        """Create an invalid (empty) event."""
        self.valid = False
        self.panel_id = 0
        self.handler_id = 0
        self.event_type = 0
        self.value = String("")
        self.hit_u = -1.0
        self.hit_v = -1.0
        self.hand = 0

    fn __init__(
        out self,
        valid: Bool,
        panel_id: UInt32,
        handler_id: UInt32,
        event_type: UInt8,
        value: String,
        hit_u: Float32,
        hit_v: Float32,
        hand: UInt8,
    ):
        self.valid = valid
        self.panel_id = panel_id
        self.handler_id = handler_id
        self.event_type = event_type
        self.value = value
        self.hit_u = hit_u
        self.hit_v = hit_v
        self.hand = hand

    fn __copyinit__(out self, other: Self):
        self.valid = other.valid
        self.panel_id = other.panel_id
        self.handler_id = other.handler_id
        self.event_type = other.event_type
        self.value = other.value
        self.hit_u = other.hit_u
        self.hit_v = other.hit_v
        self.hand = other.hand

    fn __moveinit__(out self, deinit other: Self):
        self.valid = other.valid
        self.panel_id = other.panel_id
        self.handler_id = other.handler_id
        self.event_type = other.event_type
        self.value = other.value^
        self.hit_u = other.hit_u
        self.hit_v = other.hit_v
        self.hand = other.hand


# ══════════════════════════════════════════════════════════════════════════════
# XRPose — Position + orientation in 3D space
# ══════════════════════════════════════════════════════════════════════════════


struct XRPose(Copyable, Movable):
    """Pose in 3D space — position + orientation as a unit quaternion.

    Returned by get_pose() and used for controller/head tracking.

    Fields:
        valid: True if the pose is valid (tracking is active).
        px, py, pz: Position in meters (in the active reference space).
        qx, qy, qz, qw: Orientation as a unit quaternion.
    """

    var valid: Bool
    var px: Float32
    var py: Float32
    var pz: Float32
    var qx: Float32
    var qy: Float32
    var qz: Float32
    var qw: Float32

    fn __init__(out self):
        """Create an invalid pose."""
        self.valid = False
        self.px = 0.0
        self.py = 0.0
        self.pz = 0.0
        self.qx = 0.0
        self.qy = 0.0
        self.qz = 0.0
        self.qw = 1.0

    fn __copyinit__(out self, other: Self):
        self.valid = other.valid
        self.px = other.px
        self.py = other.py
        self.pz = other.pz
        self.qx = other.qx
        self.qy = other.qy
        self.qz = other.qz
        self.qw = other.qw

    fn __moveinit__(out self, deinit other: Self):
        self.valid = other.valid
        self.px = other.px
        self.py = other.py
        self.pz = other.pz
        self.qx = other.qx
        self.qy = other.qy
        self.qz = other.qz
        self.qw = other.qw


# ══════════════════════════════════════════════════════════════════════════════
# XRRaycastHit — Raycast result against panels
# ══════════════════════════════════════════════════════════════════════════════


struct XRRaycastHit(Copyable, Movable):
    """Result of a raycast against visible XR panels.

    Fields:
        hit: True if a panel was hit.
        panel_id: ID of the hit panel (0 if no hit).
        u, v: Hit point in panel-local UV coordinates (0.0–1.0).
        distance: Distance from ray origin to hit point, in meters.
    """

    var hit: Bool
    var panel_id: UInt32
    var u: Float32
    var v: Float32
    var distance: Float32

    fn __init__(out self):
        """Create a miss result."""
        self.hit = False
        self.panel_id = 0
        self.u = 0.0
        self.v = 0.0
        self.distance = 0.0

    fn __copyinit__(out self, other: Self):
        self.hit = other.hit
        self.panel_id = other.panel_id
        self.u = other.u
        self.v = other.v
        self.distance = other.distance

    fn __moveinit__(out self, deinit other: Self):
        self.hit = other.hit
        self.panel_id = other.panel_id
        self.u = other.u
        self.v = other.v
        self.distance = other.distance


# ══════════════════════════════════════════════════════════════════════════════
# Library name constant
# ══════════════════════════════════════════════════════════════════════════════

comptime _LIB_NAME = "libmojo_xr.so"
comptime _LIB_NAME_DYLIB = "libmojo_xr.dylib"


fn _lib_name() -> String:
    """Return the platform-appropriate shared library filename."""
    # XR is Linux-only for now (OpenXR native). macOS/Windows support
    # is future work. Return .so unconditionally.
    return _LIB_NAME


# ══════════════════════════════════════════════════════════════════════════════
# _find_library — search for the shared library
# ══════════════════════════════════════════════════════════════════════════════


fn _find_library() -> String:
    """Search for the XR shim shared library.

    Search order:
      1. MOJO_XR_LIB env var (directory path)
      2. NIX_LDFLAGS (extract -L paths)
      3. LD_LIBRARY_PATH
      4. Fall back to bare library name (let the linker search)

    Returns the full path to the library, or just the library name
    if not found (letting the dynamic linker try its default search).
    """
    var sep = String("/")
    var name = _lib_name()

    # 1. Explicit env var
    var lib_dir = getenv("MOJO_XR_LIB")
    if len(lib_dir) > 0:
        return lib_dir + sep + name

    # 2. NIX_LDFLAGS — parse -L/nix/store/... paths
    var nix_flags = getenv("NIX_LDFLAGS")
    if len(nix_flags) > 0:
        var i = 0
        var flag_start = 0
        while i <= len(nix_flags):
            var at_end = i == len(nix_flags)
            var is_space = False
            if not at_end:
                is_space = nix_flags[byte=i] == " "
            if at_end or is_space:
                if i > flag_start:
                    var token = nix_flags[flag_start:i]
                    if len(token) > 2:
                        if token[byte=0] == "-" and token[byte=1] == "L":
                            var dir_path = token[2:]
                            var candidate = String(dir_path) + sep + name
                            return candidate
                flag_start = i + 1
            i += 1

    # 3. LD_LIBRARY_PATH
    var ld_path = getenv("LD_LIBRARY_PATH")
    if len(ld_path) > 0:
        var i = 0
        var path_start = 0
        while i <= len(ld_path):
            var at_end = i == len(ld_path)
            var is_colon = False
            if not at_end:
                is_colon = ld_path[byte=i] == ":"
            if at_end or is_colon:
                if i > path_start:
                    var dir_path = ld_path[path_start:i]
                    return String(dir_path) + sep + name
                path_start = i + 1
            i += 1

    # 4. Fall back to bare library name (let the linker search)
    return name


# ══════════════════════════════════════════════════════════════════════════════
# Event type name → constant mapping
# ══════════════════════════════════════════════════════════════════════════════


fn _event_type_from_name(name: String) -> UInt8:
    """Convert an event type name to the corresponding MXR_EVT_* constant.

    Args:
        name: Event name (e.g., "click", "input", "xr_select").

    Returns:
        The event type constant, or 0 if unrecognized.
    """
    if name == "click":
        return EVT_CLICK
    elif name == "input":
        return EVT_INPUT
    elif name == "change":
        return EVT_CHANGE
    elif name == "keydown":
        return EVT_KEYDOWN
    elif name == "keyup":
        return EVT_KEYUP
    elif name == "focus":
        return EVT_FOCUS
    elif name == "blur":
        return EVT_BLUR
    elif name == "submit":
        return EVT_SUBMIT
    elif name == "mousedown":
        return EVT_MOUSEDOWN
    elif name == "mouseup":
        return EVT_MOUSEUP
    elif name == "mousemove":
        return EVT_MOUSEMOVE
    elif name == "xr_select":
        return EVT_XR_SELECT
    elif name == "xr_squeeze":
        return EVT_XR_SQUEEZE
    elif name == "xr_hover_enter":
        return EVT_XR_HOVER_ENTER
    elif name == "xr_hover_exit":
        return EVT_XR_HOVER_EXIT
    else:
        return 0


# ══════════════════════════════════════════════════════════════════════════════
# XRBlitz — Typed wrapper around the XR Blitz C shim
# ══════════════════════════════════════════════════════════════════════════════


struct XRBlitz(Movable):
    """Mojo FFI wrapper for the XR Blitz renderer C shim.

    This struct manages the lifecycle of an XR session context, providing
    typed methods for all shim operations: session management, panel
    lifecycle, DOM manipulation (per-panel), event handling, raycasting,
    frame loop, input tracking, and capabilities.

    Unlike the desktop `Blitz` struct which manages a single document,
    `XRBlitz` manages multiple documents — one per XR panel. Every DOM
    operation takes a `panel_id` parameter.

    The session is created via `XRBlitz.create_session()` or
    `XRBlitz.create_headless()` and destroyed via `destroy()`.
    """

    var _lib: _DLHandle
    var _session: UnsafePointer[NoneType, MutAnyOrigin]

    fn __init__(
        out self,
        lib: _DLHandle,
        session: UnsafePointer[NoneType, MutAnyOrigin],
    ):
        """Private initializer. Use XRBlitz.create_session() or
        XRBlitz.create_headless() instead."""
        self._lib = lib
        self._session = session

    fn __moveinit__(out self, deinit other: Self):
        self._lib = other._lib
        self._session = other._session

    # ── Factory methods ──────────────────────────────────────────────────

    @staticmethod
    fn create_session(app_name: String) raises -> Self:
        """Create an XR session with the default OpenXR runtime.

        Initializes the OpenXR instance, creates a session with graphics
        binding, allocates the wgpu device, and creates the Vello renderer.

        Args:
            app_name: Application name (shown in OpenXR runtime UI).

        Returns:
            A new XRBlitz instance.

        Raises:
            If the shared library cannot be loaded or the OpenXR runtime
            is unavailable.
        """
        var lib_path = _find_library()
        var lib = _DLHandle(lib_path)

        var name_ptr = app_name.unsafe_ptr()
        var name_len = UInt32(len(app_name))

        var session = lib.call[
            "mxr_create_session", UnsafePointer[NoneType, MutAnyOrigin]
        ](name_ptr, name_len)

        if not session:
            raise Error(
                "XRBlitz: failed to create OpenXR session — is an XR runtime"
                " available?"
            )

        return Self(lib, session)

    @staticmethod
    fn create_headless() raises -> Self:
        """Create a headless XR session for testing.

        Allocates Blitz documents and performs DOM operations, but does
        not create an OpenXR instance or GPU resources. Useful for
        integration tests and CI environments.

        Returns:
            A new XRBlitz instance (headless mode).

        Raises:
            If the shared library cannot be loaded.
        """
        var lib_path = _find_library()
        var lib = _DLHandle(lib_path)

        var session = lib.call[
            "mxr_create_headless", UnsafePointer[NoneType, MutAnyOrigin]
        ]()

        return Self(lib, session)

    # ══════════════════════════════════════════════════════════════════════
    # Session lifecycle
    # ══════════════════════════════════════════════════════════════════════

    fn session_state(self) -> Int32:
        """Query the current session state.

        Returns one of the STATE_* constants.
        """
        return self._lib.call["mxr_session_state", Int32](self._session)

    fn is_alive(self) -> Bool:
        """Check if the session is still alive (not exiting or destroyed).

        Returns:
            True if the session is alive.
        """
        var result = self._lib.call["mxr_is_alive", Int32](self._session)
        return result != 0

    fn destroy(mut self):
        """Destroy the XR session and release all resources.

        The session is invalid after this call.
        """
        if self._session:
            self._lib.call["mxr_destroy_session", NoneType](self._session)
            self._session = UnsafePointer[NoneType, MutAnyOrigin]()

    # ══════════════════════════════════════════════════════════════════════
    # Panel lifecycle
    # ══════════════════════════════════════════════════════════════════════

    fn create_panel(self, width_px: UInt32, height_px: UInt32) -> UInt32:
        """Create a new XR panel with the given pixel dimensions.

        Allocates a new Blitz document, an offscreen texture, and a
        mutation interpreter for this panel.

        Args:
            width_px: Texture width in pixels.
            height_px: Texture height in pixels.

        Returns:
            Panel ID (non-zero), or 0 on failure.
        """
        return self._lib.call["mxr_create_panel", UInt32](
            self._session, width_px, height_px
        )

    fn destroy_panel(self, panel_id: UInt32):
        """Destroy a panel and free its Blitz document and GPU texture.

        Args:
            panel_id: Panel to destroy.
        """
        self._lib.call["mxr_destroy_panel", NoneType](self._session, panel_id)

    fn panel_count(self) -> UInt32:
        """Query the number of active panels.

        Returns:
            Number of active panels.
        """
        return self._lib.call["mxr_panel_count", UInt32](self._session)

    # ══════════════════════════════════════════════════════════════════════
    # Panel transform & display
    # ══════════════════════════════════════════════════════════════════════

    fn panel_set_transform(
        self,
        panel_id: UInt32,
        px: Float32,
        py: Float32,
        pz: Float32,
        qx: Float32,
        qy: Float32,
        qz: Float32,
        qw: Float32,
    ):
        """Set a panel's 3D transform (position + orientation) in world space.

        Args:
            panel_id: Panel to transform.
            px, py, pz: Position of the panel center, in meters.
            qx, qy, qz, qw: Orientation as a unit quaternion.
        """
        self._lib.call["mxr_panel_set_transform", NoneType](
            self._session, panel_id, px, py, pz, qx, qy, qz, qw
        )

    fn panel_set_size(
        self, panel_id: UInt32, width_m: Float32, height_m: Float32
    ):
        """Set a panel's physical size in meters.

        Args:
            panel_id: Panel to resize.
            width_m: Physical width in meters.
            height_m: Physical height in meters.
        """
        self._lib.call["mxr_panel_set_size", NoneType](
            self._session, panel_id, width_m, height_m
        )

    fn panel_set_visible(self, panel_id: UInt32, visible: Bool):
        """Show or hide a panel.

        Hidden panels are not rendered, not raycasted, and not submitted
        as quad layers. They retain their DOM state.

        Args:
            panel_id: Panel to show/hide.
            visible: True to show, False to hide.
        """
        var flag = Int32(1) if visible else Int32(0)
        self._lib.call["mxr_panel_set_visible", NoneType](
            self._session, panel_id, flag
        )

    fn panel_is_visible(self, panel_id: UInt32) -> Bool:
        """Query whether a panel is visible.

        Returns:
            True if visible.
        """
        var result = self._lib.call["mxr_panel_is_visible", Int32](
            self._session, panel_id
        )
        return result != 0

    fn panel_set_curved(self, panel_id: UInt32, curved: Bool, radius: Float32):
        """Set the curved display flag and curvature radius for a panel.

        Args:
            panel_id: Panel to configure.
            curved: True for curved, False for flat.
            radius: Curvature radius in meters (ignored if curved=False).
        """
        var flag = Int32(1) if curved else Int32(0)
        self._lib.call["mxr_panel_set_curved", NoneType](
            self._session, panel_id, flag, radius
        )

    # ══════════════════════════════════════════════════════════════════════
    # User-agent stylesheet
    # ══════════════════════════════════════════════════════════════════════

    fn panel_add_ua_stylesheet(self, panel_id: UInt32, css: String):
        """Add a user-agent stylesheet to a panel's Blitz document.

        Should be called before applying mount mutations.

        Args:
            panel_id: Panel to style.
            css: CSS text.
        """
        var css_ptr = css.unsafe_ptr()
        self._lib.call["mxr_panel_add_ua_stylesheet", NoneType](
            self._session, panel_id, css_ptr, UInt32(len(css))
        )

    # ══════════════════════════════════════════════════════════════════════
    # Mutation batching
    # ══════════════════════════════════════════════════════════════════════

    fn panel_begin_mutations(self, panel_id: UInt32):
        """Begin a mutation batch for a panel.

        Must be called before applying mutations. Defers style resolution
        and layout until panel_end_mutations().

        Args:
            panel_id: Panel to batch mutations for.
        """
        self._lib.call["mxr_panel_begin_mutations", NoneType](
            self._session, panel_id
        )

    fn panel_end_mutations(self, panel_id: UInt32):
        """End a mutation batch for a panel.

        Triggers style resolution and layout computation. Marks the
        panel's texture as dirty.

        Args:
            panel_id: Panel to end mutations for.
        """
        self._lib.call["mxr_panel_end_mutations", NoneType](
            self._session, panel_id
        )

    fn panel_apply_mutations(
        self,
        panel_id: UInt32,
        buf: UnsafePointer[UInt8, MutAnyOrigin],
        length: UInt32,
    ):
        """Apply a binary mutation buffer to a panel's DOM.

        The buffer contains the same binary opcodes as the desktop
        renderer. The shim's per-panel interpreter reads the opcodes
        and translates them into Blitz DOM operations.

        Note: This uses the shim-side (Rust) binary interpreter. For
        Mojo-side interpretation (calling individual FFI functions),
        use XRMutationInterpreter from xr/renderer.mojo.

        Args:
            panel_id: Target panel.
            buf: Pointer to the mutation buffer.
            length: Number of valid bytes in the buffer.
        """
        self._lib.call["mxr_panel_apply_mutations", NoneType](
            self._session, panel_id, buf, length
        )

    # ══════════════════════════════════════════════════════════════════════
    # DOM node creation (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_create_element(self, panel_id: UInt32, tag: String) -> UInt32:
        """Create an HTML element node (detached) in a panel.

        Args:
            panel_id: Target panel.
            tag: HTML tag name (e.g., "div", "button", "h1").

        Returns:
            Blitz node ID of the new element.
        """
        var tag_ptr = tag.unsafe_ptr()
        return self._lib.call["mxr_panel_create_element", UInt32](
            self._session, panel_id, tag_ptr, UInt32(len(tag))
        )

    fn panel_create_text_node(self, panel_id: UInt32, text: String) -> UInt32:
        """Create a text node (detached) in a panel.

        Args:
            panel_id: Target panel.
            text: Text content.

        Returns:
            Blitz node ID of the new text node.
        """
        var text_ptr = text.unsafe_ptr()
        return self._lib.call["mxr_panel_create_text_node", UInt32](
            self._session, panel_id, text_ptr, UInt32(len(text))
        )

    fn panel_create_placeholder(self, panel_id: UInt32) -> UInt32:
        """Create a comment/placeholder node (detached) in a panel.

        Args:
            panel_id: Target panel.

        Returns:
            Blitz node ID of the placeholder.
        """
        return self._lib.call["mxr_panel_create_placeholder", UInt32](
            self._session, panel_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # Templates (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_register_template(
        self,
        panel_id: UInt32,
        buf: UnsafePointer[UInt8, MutAnyOrigin],
        length: UInt32,
    ):
        """Register a template definition in a panel.

        The buffer contains the serialized template definition in the
        same format as OP_REGISTER_TEMPLATE.

        Args:
            panel_id: Target panel.
            buf: Template definition buffer.
            length: Buffer length in bytes.
        """
        self._lib.call["mxr_panel_register_template", NoneType](
            self._session, panel_id, buf, length
        )

    fn panel_clone_template(
        self, panel_id: UInt32, template_id: UInt32
    ) -> UInt32:
        """Deep-clone a registered template in a panel.

        Args:
            panel_id: Target panel.
            template_id: Template ID (previously registered).

        Returns:
            Blitz node ID of the cloned root. 0 if not registered.
        """
        return self._lib.call["mxr_panel_clone_template", UInt32](
            self._session, panel_id, template_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # DOM tree mutations (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_append_children(
        self,
        panel_id: UInt32,
        parent_id: UInt32,
        child_ids: UnsafePointer[UInt32],
        child_count: UInt32,
    ):
        """Append children to a parent element in a panel.

        Args:
            panel_id: Target panel.
            parent_id: Parent node ID (0 = mount point).
            child_ids: Pointer to array of child node IDs.
            child_count: Number of children.
        """
        self._lib.call["mxr_panel_append_children", NoneType](
            self._session, panel_id, parent_id, child_ids, child_count
        )

    fn panel_insert_before(
        self,
        panel_id: UInt32,
        anchor_id: UInt32,
        new_ids: UnsafePointer[UInt32],
        new_count: UInt32,
    ):
        """Insert nodes before an anchor node in a panel.

        Args:
            panel_id: Target panel.
            anchor_id: Anchor node ID.
            new_ids: Pointer to array of new node IDs.
            new_count: Number of new nodes.
        """
        self._lib.call["mxr_panel_insert_before", NoneType](
            self._session, panel_id, anchor_id, new_ids, new_count
        )

    fn panel_insert_after(
        self,
        panel_id: UInt32,
        anchor_id: UInt32,
        new_ids: UnsafePointer[UInt32],
        new_count: UInt32,
    ):
        """Insert nodes after an anchor node in a panel.

        Args:
            panel_id: Target panel.
            anchor_id: Anchor node ID.
            new_ids: Pointer to array of new node IDs.
            new_count: Number of new nodes.
        """
        self._lib.call["mxr_panel_insert_after", NoneType](
            self._session, panel_id, anchor_id, new_ids, new_count
        )

    fn panel_replace_with(
        self,
        panel_id: UInt32,
        old_id: UInt32,
        new_ids: UnsafePointer[UInt32],
        new_count: UInt32,
    ):
        """Replace a node with new nodes in a panel.

        Args:
            panel_id: Target panel.
            old_id: Node ID to replace.
            new_ids: Pointer to array of replacement node IDs.
            new_count: Number of replacements.
        """
        self._lib.call["mxr_panel_replace_with", NoneType](
            self._session, panel_id, old_id, new_ids, new_count
        )

    fn panel_remove_node(self, panel_id: UInt32, node_id: UInt32):
        """Remove and drop a node from a panel's DOM.

        Args:
            panel_id: Target panel.
            node_id: Node ID to remove.
        """
        self._lib.call["mxr_panel_remove_node", NoneType](
            self._session, panel_id, node_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # DOM node attributes (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_set_attribute(
        self, panel_id: UInt32, node_id: UInt32, name: String, value: String
    ):
        """Set an attribute on an element in a panel.

        Args:
            panel_id: Target panel.
            node_id: Target element node ID.
            name: Attribute name (e.g., "class", "style", "id").
            value: Attribute value.
        """
        var name_ptr = name.unsafe_ptr()
        var value_ptr = value.unsafe_ptr()
        self._lib.call["mxr_panel_set_attribute", NoneType](
            self._session,
            panel_id,
            node_id,
            name_ptr,
            UInt32(len(name)),
            value_ptr,
            UInt32(len(value)),
        )

    fn panel_remove_attribute(
        self, panel_id: UInt32, node_id: UInt32, name: String
    ):
        """Remove an attribute from an element in a panel.

        Args:
            panel_id: Target panel.
            node_id: Target element node ID.
            name: Attribute name to remove.
        """
        var name_ptr = name.unsafe_ptr()
        self._lib.call["mxr_panel_remove_attribute", NoneType](
            self._session,
            panel_id,
            node_id,
            name_ptr,
            UInt32(len(name)),
        )

    # ══════════════════════════════════════════════════════════════════════
    # DOM text content (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_set_text_content(
        self, panel_id: UInt32, node_id: UInt32, text: String
    ):
        """Set the text content of a text node in a panel.

        Args:
            panel_id: Target panel.
            node_id: Text node ID.
            text: New text content.
        """
        var text_ptr = text.unsafe_ptr()
        self._lib.call["mxr_panel_set_text_content", NoneType](
            self._session, panel_id, node_id, text_ptr, UInt32(len(text))
        )

    # ══════════════════════════════════════════════════════════════════════
    # DOM tree traversal (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_node_at_path(
        self,
        panel_id: UInt32,
        root_id: UInt32,
        path: UnsafePointer[UInt32],
        path_len: UInt32,
    ) -> UInt32:
        """Navigate to a child at the given path from a starting node.

        The path is an array of child indices (e.g., [0, 2, 1] means
        child 0 → child 2 → child 1).

        Args:
            panel_id: Target panel.
            root_id: Starting node ID.
            path: Array of child indices.
            path_len: Length of the path array.

        Returns:
            Node ID at the end of the path. 0 on failure.
        """
        return self._lib.call["mxr_panel_node_at_path", UInt32](
            self._session, panel_id, root_id, path, path_len
        )

    fn panel_child_at(
        self, panel_id: UInt32, parent_id: UInt32, index: UInt32
    ) -> UInt32:
        """Get the Nth child of a node in a panel.

        Args:
            panel_id: Target panel.
            parent_id: Parent node ID.
            index: Zero-based child index.

        Returns:
            Child node ID. 0 if index is out of bounds.
        """
        return self._lib.call["mxr_panel_child_at", UInt32](
            self._session, panel_id, parent_id, index
        )

    fn panel_child_count(self, panel_id: UInt32, parent_id: UInt32) -> UInt32:
        """Get the number of children of a node in a panel.

        Args:
            panel_id: Target panel.
            parent_id: Node ID.

        Returns:
            Number of children.
        """
        return self._lib.call["mxr_panel_child_count", UInt32](
            self._session, panel_id, parent_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # Event handling
    # ══════════════════════════════════════════════════════════════════════

    fn panel_add_event_listener(
        self,
        panel_id: UInt32,
        node_id: UInt32,
        handler_id: UInt32,
        event_type: UInt8,
    ):
        """Register an event handler on a node in a panel.

        Args:
            panel_id: Target panel.
            node_id: Target element node ID.
            handler_id: Unique handler ID (from HandlerRegistry).
            event_type: Event type constant (EVT_CLICK, etc.).
        """
        self._lib.call["mxr_panel_add_event_listener", NoneType](
            self._session, panel_id, node_id, handler_id, event_type
        )

    fn panel_add_event_listener_by_name(
        self,
        panel_id: UInt32,
        node_id: UInt32,
        handler_id: UInt32,
        event_name: String,
    ):
        """Register an event handler on a node using a string event name.

        Convenience wrapper that converts the event name to the
        corresponding EVT_* constant.

        Args:
            panel_id: Target panel.
            node_id: Target element node ID.
            handler_id: Unique handler ID (from HandlerRegistry).
            event_name: Event type name (e.g., "click", "input").
        """
        var event_type = _event_type_from_name(event_name)
        if event_type != 0:
            self.panel_add_event_listener(
                panel_id, node_id, handler_id, event_type
            )

    fn panel_remove_event_listener(
        self,
        panel_id: UInt32,
        node_id: UInt32,
        handler_id: UInt32,
        event_type: UInt8,
    ):
        """Remove an event handler from a node in a panel.

        Args:
            panel_id: Target panel.
            node_id: Target element node ID.
            handler_id: Handler ID to remove.
            event_type: Event type constant.
        """
        self._lib.call["mxr_panel_remove_event_listener", NoneType](
            self._session, panel_id, node_id, handler_id, event_type
        )

    fn panel_remove_event_listener_by_name(
        self,
        panel_id: UInt32,
        node_id: UInt32,
        handler_id: UInt32,
        event_name: String,
    ):
        """Remove an event handler using a string event name.

        Args:
            panel_id: Target panel.
            node_id: Target element node ID.
            handler_id: Handler ID to remove.
            event_name: Event type name (e.g., "click", "input").
        """
        var event_type = _event_type_from_name(event_name)
        if event_type != 0:
            self.panel_remove_event_listener(
                panel_id, node_id, handler_id, event_type
            )

    fn poll_event(self) -> XREvent:
        """Poll the next event from the XR input queue.

        Uses `mxr_poll_event_into` which writes event fields to
        caller-provided output pointers, avoiding struct-return ABI
        issues with Mojo's DLHandle.

        Returns:
            An XREvent with valid=True if an event was available,
            or valid=False if the queue is empty.
        """
        # Allocate output slots for each event field.
        var out_panel_id = alloc[UInt32](1)
        var out_handler_id = alloc[UInt32](1)
        var out_event_type = alloc[UInt8](1)
        var out_value_ptr = alloc[Int](
            1
        )  # pointer-sized (same pattern as desktop)
        var out_value_len = alloc[UInt32](1)
        var out_hit_u = alloc[Float32](1)
        var out_hit_v = alloc[Float32](1)
        var out_hand = alloc[UInt8](1)

        out_panel_id[0] = 0
        out_handler_id[0] = 0
        out_event_type[0] = 0
        out_value_ptr[0] = 0
        out_value_len[0] = 0
        out_hit_u[0] = -1.0
        out_hit_v[0] = -1.0
        out_hand[0] = 0

        var valid = self._lib.call["mxr_poll_event_into", Int32](
            self._session,
            out_panel_id,
            out_handler_id,
            out_event_type,
            out_value_ptr,
            out_value_len,
            out_hit_u,
            out_hit_v,
            out_hand,
        )

        if valid == 0:
            out_panel_id.free()
            out_handler_id.free()
            out_event_type.free()
            out_value_ptr.free()
            out_value_len.free()
            out_hit_u.free()
            out_hit_v.free()
            out_hand.free()
            return XREvent()

        var panel_id = out_panel_id[0]
        var handler_id = out_handler_id[0]
        var event_type = out_event_type[0]
        var v_ptr_int = out_value_ptr[0]
        var v_len = Int(out_value_len[0])
        var hit_u = out_hit_u[0]
        var hit_v = out_hit_v[0]
        var hand = out_hand[0]

        out_panel_id.free()
        out_handler_id.free()
        out_event_type.free()
        out_value_ptr.free()
        out_value_len.free()
        out_hit_u.free()
        out_hit_v.free()
        out_hand.free()

        # Build the value string from the pointer + length.
        # The pointer points into the shim's last_polled_value which
        # stays alive until the next poll call.
        var value = String("")
        if v_len > 0 and v_ptr_int != 0:
            var slot = alloc[Int](1)
            slot[0] = v_ptr_int
            var v_ptr = slot.bitcast[UnsafePointer[UInt8, MutAnyOrigin]]()[0]
            slot.free()
            for i in range(v_len):
                value += chr(Int(v_ptr[i]))

        return XREvent(
            valid=True,
            panel_id=panel_id,
            handler_id=handler_id,
            event_type=event_type,
            value=value,
            hit_u=hit_u,
            hit_v=hit_v,
            hand=hand,
        )

    fn event_count(self) -> UInt32:
        """Get the number of buffered events.

        Returns:
            Number of events waiting to be polled.
        """
        return self._lib.call["mxr_event_count", UInt32](self._session)

    fn event_clear(self):
        """Clear all buffered events."""
        self._lib.call["mxr_event_clear", NoneType](self._session)

    # ══════════════════════════════════════════════════════════════════════
    # Raycasting
    # ══════════════════════════════════════════════════════════════════════

    fn raycast_panels(
        self,
        ox: Float32,
        oy: Float32,
        oz: Float32,
        dx: Float32,
        dy: Float32,
        dz: Float32,
    ) -> XRRaycastHit:
        """Raycast against all visible, interactive panels.

        Uses `mxr_raycast_panels_into` which writes result fields to
        caller-provided output pointers, avoiding struct-return ABI
        issues with Mojo's DLHandle.

        Args:
            ox, oy, oz: Ray origin in world space (meters).
            dx, dy, dz: Ray direction in world space (normalized).

        Returns:
            Raycast hit result.
        """
        var out_panel_id = alloc[UInt32](1)
        var out_u = alloc[Float32](1)
        var out_v = alloc[Float32](1)
        var out_distance = alloc[Float32](1)

        out_panel_id[0] = 0
        out_u[0] = 0.0
        out_v[0] = 0.0
        out_distance[0] = 0.0

        var hit = self._lib.call["mxr_raycast_panels_into", Int32](
            self._session,
            ox,
            oy,
            oz,
            dx,
            dy,
            dz,
            out_panel_id,
            out_u,
            out_v,
            out_distance,
        )

        var result = XRRaycastHit()
        if hit != 0:
            result.hit = True
            result.panel_id = out_panel_id[0]
            result.u = out_u[0]
            result.v = out_v[0]
            result.distance = out_distance[0]

        out_panel_id.free()
        out_u.free()
        out_v.free()
        out_distance.free()

        return result

    fn set_focused_panel(self, panel_id: UInt32):
        """Set the focused panel (receives keyboard/text input).

        Args:
            panel_id: Panel to focus, or 0 to clear focus.
        """
        self._lib.call["mxr_set_focused_panel", NoneType](
            self._session, panel_id
        )

    fn get_focused_panel(self) -> UInt32:
        """Query which panel currently has input focus.

        Returns:
            The focused panel's ID, or 0 if no panel has focus.
        """
        return self._lib.call["mxr_get_focused_panel", UInt32](self._session)

    # ══════════════════════════════════════════════════════════════════════
    # Frame loop
    # ══════════════════════════════════════════════════════════════════════

    fn wait_frame(self) -> Int64:
        """Wait for the next frame from the OpenXR runtime.

        Blocks until the runtime signals that a new frame should be
        rendered. Returns the predicted display time in nanoseconds.

        Returns:
            Predicted display time in nanoseconds, or 0 if the session
            is not in a renderable state.
        """
        return self._lib.call["mxr_wait_frame", Int64](self._session)

    fn begin_frame(self) -> Bool:
        """Begin a new frame.

        Call after wait_frame(). Acquires the OpenXR swapchain image.

        Returns:
            True on success, False if the frame should be skipped.
        """
        var result = self._lib.call["mxr_begin_frame", Int32](self._session)
        return result != 0

    fn render_dirty_panels(self) -> UInt32:
        """Render all dirty panel textures.

        For each panel marked dirty, runs Vello to re-render the panel's
        Blitz DOM to its offscreen GPU texture.

        Call between begin_frame() and end_frame().

        Returns:
            The number of panels that were re-rendered.
        """
        return self._lib.call["mxr_render_dirty_panels", UInt32](self._session)

    fn end_frame(self):
        """End the frame and submit composition layers to OpenXR.

        Submits one quad layer per visible panel to the OpenXR compositor.
        """
        self._lib.call["mxr_end_frame", NoneType](self._session)

    # ══════════════════════════════════════════════════════════════════════
    # GPU initialisation — Vello offscreen rendering pipeline
    # ══════════════════════════════════════════════════════════════════════

    fn init_gpu(self) -> Bool:
        """Try to initialise the GPU renderer (wgpu + Vello) for offscreen
        panel texture rendering.

        This is intentionally separate from session creation so that
        headless tests keep working without a GPU. Call once after
        creating the session.

        When GPU is available, render_dirty_panels() will paint each
        panel's Blitz DOM to an offscreen GPU texture via Vello. Without
        GPU, it falls back to layout-only resolution (Stylo + Taffy, no
        pixel output).

        Returns:
            True on success (GPU renderer initialised or already init'd).
            False on failure (no compatible GPU adapter found).
        """
        var result = self._lib.call["mxr_init_gpu", Int32](self._session)
        return result != 0

    fn has_gpu(self) -> Bool:
        """Check whether the GPU renderer is available.

        Returns:
            True if init_gpu() has been called successfully.
            False otherwise.
        """
        var result = self._lib.call["mxr_has_gpu", Int32](self._session)
        return result != 0

    fn read_pixels(
        self,
        panel_id: UInt32,
        buf: UnsafePointer[UInt8],
        buf_len: UInt32,
    ) -> UInt32:
        """Copy a panel's most-recently-rendered texture to a CPU buffer.

        The buffer receives RGBA8 pixel data in row-major order
        (top-left origin). The required buffer size is
        panel_width * panel_height * 4 bytes.

        Args:
            panel_id: The panel whose texture to read.
            buf: Destination buffer (caller-allocated).
            buf_len: Size of buf in bytes.

        Returns:
            Number of bytes written on success (== width * height * 4).
            0 on failure (no GPU, no texture, buffer too small, panel
            not found).
        """
        return self._lib.call["mxr_panel_read_pixels", UInt32](
            self._session, panel_id, buf, buf_len
        )

    # ══════════════════════════════════════════════════════════════════════
    # Input — controller and head pose tracking
    # ══════════════════════════════════════════════════════════════════════

    fn get_pose(self, hand: UInt8) -> XRPose:
        """Get the current pose of a controller or the head.

        Uses `mxr_get_pose_into` which writes pose fields to
        caller-provided output pointers, avoiding struct-return ABI
        issues with Mojo's DLHandle.

        Args:
            hand: HAND_LEFT, HAND_RIGHT, or HAND_HEAD.

        Returns:
            The pose in the session's reference space. If the controller
            is not tracked, the pose's valid field is False.
        """
        var out_px = alloc[Float32](1)
        var out_py = alloc[Float32](1)
        var out_pz = alloc[Float32](1)
        var out_qx = alloc[Float32](1)
        var out_qy = alloc[Float32](1)
        var out_qz = alloc[Float32](1)
        var out_qw = alloc[Float32](1)

        out_px[0] = 0.0
        out_py[0] = 0.0
        out_pz[0] = 0.0
        out_qx[0] = 0.0
        out_qy[0] = 0.0
        out_qz[0] = 0.0
        out_qw[0] = 1.0

        var valid = self._lib.call["mxr_get_pose_into", Int32](
            self._session,
            hand,
            out_px,
            out_py,
            out_pz,
            out_qx,
            out_qy,
            out_qz,
            out_qw,
        )

        var result = XRPose()
        if valid != 0:
            result.valid = True
            result.px = out_px[0]
            result.py = out_py[0]
            result.pz = out_pz[0]
            result.qx = out_qx[0]
            result.qy = out_qy[0]
            result.qz = out_qz[0]
            result.qw = out_qw[0]

        out_px.free()
        out_py.free()
        out_pz.free()
        out_qx.free()
        out_qy.free()
        out_qz.free()
        out_qw.free()

        return result

    fn get_aim_ray(
        self,
        hand: UInt8,
    ) -> Tuple[Bool, Float32, Float32, Float32, Float32, Float32, Float32]:
        """Get the aim ray for a controller (origin + direction).

        Args:
            hand: HAND_LEFT or HAND_RIGHT.

        Returns:
            Tuple of (valid, ox, oy, oz, dx, dy, dz).
        """
        var out_ox = alloc[Float32](1)
        var out_oy = alloc[Float32](1)
        var out_oz = alloc[Float32](1)
        var out_dx = alloc[Float32](1)
        var out_dy = alloc[Float32](1)
        var out_dz = alloc[Float32](1)

        out_ox[0] = 0.0
        out_oy[0] = 0.0
        out_oz[0] = 0.0
        out_dx[0] = 0.0
        out_dy[0] = 0.0
        out_dz[0] = 0.0

        var valid = self._lib.call["mxr_get_aim_ray", Int32](
            self._session, hand, out_ox, out_oy, out_oz, out_dx, out_dy, out_dz
        )

        var result = (
            valid != 0,
            out_ox[0],
            out_oy[0],
            out_oz[0],
            out_dx[0],
            out_dy[0],
            out_dz[0],
        )

        out_ox.free()
        out_oy.free()
        out_oz.free()
        out_dx.free()
        out_dy.free()
        out_dz.free()

        return result

    # ══════════════════════════════════════════════════════════════════════
    # Reference spaces
    # ══════════════════════════════════════════════════════════════════════

    fn set_reference_space(self, space_type: UInt8) -> Bool:
        """Set the session's reference space type.

        Args:
            space_type: SPACE_LOCAL, SPACE_STAGE, SPACE_VIEW, or
                        SPACE_UNBOUNDED.

        Returns:
            True on success, False if not supported by the runtime.
        """
        var result = self._lib.call["mxr_set_reference_space", Int32](
            self._session, space_type
        )
        return result != 0

    fn get_reference_space(self) -> UInt8:
        """Query the current reference space type.

        Returns:
            One of the SPACE_* constants.
        """
        return self._lib.call["mxr_get_reference_space", UInt8](self._session)

    # ══════════════════════════════════════════════════════════════════════
    # Capabilities — runtime feature detection
    # ══════════════════════════════════════════════════════════════════════

    fn has_extension(self, ext_name: String) -> Bool:
        """Check if a specific OpenXR extension is available.

        Args:
            ext_name: Extension name (e.g., "XR_EXT_hand_tracking").

        Returns:
            True if the extension is available and enabled.
        """
        var name_ptr = ext_name.unsafe_ptr()
        var result = self._lib.call["mxr_has_extension", Int32](
            self._session, name_ptr, UInt32(len(ext_name))
        )
        return result != 0

    fn has_hand_tracking(self) -> Bool:
        """Check if hand tracking is available.

        Returns:
            True if XR_EXT_hand_tracking is available.
        """
        var result = self._lib.call["mxr_has_hand_tracking", Int32](
            self._session
        )
        return result != 0

    fn has_passthrough(self) -> Bool:
        """Check if passthrough (AR) is available.

        Returns:
            True if XR_FB_passthrough is available.
        """
        var result = self._lib.call["mxr_has_passthrough", Int32](self._session)
        return result != 0

    fn get_select_state(self) -> Int32:
        """Get the select (trigger) state for both hands.

        Returns a bitfield:
          - bit 0 (0x1): left hand trigger active
          - bit 1 (0x2): right hand trigger active

        With a real OpenXR session, this reflects the current state of the
        select action (trigger press on most controllers). In headless mode,
        always returns 0.

        Returns:
            Bitfield with left (bit 0) and right (bit 1) trigger state.
        """
        return self._lib.call["mxr_get_select_state", Int32](self._session)

    fn get_squeeze_state(self) -> Int32:
        """Get the squeeze (grip) state for both hands.

        Returns a bitfield:
          - bit 0 (0x1): left hand grip active
          - bit 1 (0x2): right hand grip active

        With a real OpenXR session, this reflects the current state of the
        squeeze action (grip button on most controllers). In headless mode,
        always returns 0.

        Returns:
            Bitfield with left (bit 0) and right (bit 1) grip state.
        """
        return self._lib.call["mxr_get_squeeze_state", Int32](self._session)

    # ══════════════════════════════════════════════════════════════════════
    # ID mapping (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_assign_id(
        self, panel_id: UInt32, mojo_id: UInt32, node_id: UInt32
    ):
        """Assign a mojo-gui element ID to a Blitz node ID within a panel.

        Used by the mutation interpreter when processing the AssignId opcode.

        Args:
            panel_id: Target panel.
            mojo_id: The mojo-gui element ID.
            node_id: The Blitz slab node ID.
        """
        self._lib.call["mxr_panel_assign_id", NoneType](
            self._session, panel_id, mojo_id, node_id
        )

    fn panel_resolve_id(self, panel_id: UInt32, mojo_id: UInt32) -> UInt32:
        """Resolve a mojo-gui element ID to a Blitz node ID within a panel.

        Args:
            panel_id: Target panel.
            mojo_id: The mojo-gui element ID.

        Returns:
            The Blitz node ID, or 0 if not mapped.
        """
        return self._lib.call["mxr_panel_resolve_id", UInt32](
            self._session, panel_id, mojo_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # Stack operations (per-panel, for mutation interpreter)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_stack_push(self, panel_id: UInt32, node_id: UInt32):
        """Push a node ID onto the panel's interpreter stack.

        Args:
            panel_id: Target panel.
            node_id: Node ID to push.
        """
        self._lib.call["mxr_panel_stack_push", NoneType](
            self._session, panel_id, node_id
        )

    fn panel_stack_pop(self, panel_id: UInt32) -> UInt32:
        """Pop a node ID from the panel's interpreter stack.

        Args:
            panel_id: Target panel.

        Returns:
            The popped node ID, or 0 if the stack is empty.
        """
        return self._lib.call["mxr_panel_stack_pop", UInt32](
            self._session, panel_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # Document root access (per-panel)
    # ══════════════════════════════════════════════════════════════════════

    fn panel_mount_point_id(self, panel_id: UInt32) -> UInt32:
        """Get the mount point node ID for a panel.

        Returns the Blitz-internal node ID of the panel's mount point
        (typically the <body> element).

        Args:
            panel_id: Target panel.

        Returns:
            Mount point node ID.
        """
        return self._lib.call["mxr_panel_mount_point_id", UInt32](
            self._session, panel_id
        )

    # ══════════════════════════════════════════════════════════════════════
    # Debug & inspection
    # ══════════════════════════════════════════════════════════════════════

    fn panel_print_tree(self, panel_id: UInt32):
        """Print a panel's DOM tree to stderr (for debugging).

        Args:
            panel_id: Panel to print.
        """
        self._lib.call["mxr_panel_print_tree", NoneType](
            self._session, panel_id
        )

    fn panel_serialize_subtree(self, panel_id: UInt32) -> String:
        """Serialize a panel's DOM subtree to an HTML string.

        Args:
            panel_id: Panel to serialize.

        Returns:
            The serialized HTML string.
        """
        # First call with NULL to get the required size.
        var needed = self._lib.call["mxr_panel_serialize_subtree", UInt32](
            self._session,
            panel_id,
            UnsafePointer[UInt8, MutAnyOrigin](),
            UInt32(0),
        )

        if needed == 0:
            return String("")

        # Allocate buffer and read the serialized HTML.
        var buf_size = Int(needed) + 1  # +1 for safety
        var buf = alloc[UInt8](buf_size)
        for i in range(buf_size):
            buf[i] = 0

        var written = self._lib.call["mxr_panel_serialize_subtree", UInt32](
            self._session, panel_id, buf, UInt32(buf_size)
        )

        var result = String("")
        for i in range(Int(written)):
            result += chr(Int(buf[i]))

        buf.free()
        return result

    fn panel_get_node_tag(self, panel_id: UInt32, node_id: UInt32) -> String:
        """Get the tag name of a node in a panel (for testing).

        Args:
            panel_id: Target panel.
            node_id: Node to query.

        Returns:
            Tag name (e.g., "div", "button"), or empty string.
        """
        var buf_size = 64
        var buf = alloc[UInt8](buf_size)
        for i in range(buf_size):
            buf[i] = 0

        var written = self._lib.call["mxr_panel_get_node_tag", UInt32](
            self._session, panel_id, node_id, buf, UInt32(buf_size)
        )

        var result = String("")
        for i in range(Int(written)):
            result += chr(Int(buf[i]))

        buf.free()
        return result

    fn panel_get_text_content(
        self, panel_id: UInt32, node_id: UInt32
    ) -> String:
        """Get the text content of a node in a panel (for testing).

        Args:
            panel_id: Target panel.
            node_id: Node to query.

        Returns:
            Text content, or empty string.
        """
        var buf_size = 1024
        var buf = alloc[UInt8](buf_size)
        for i in range(buf_size):
            buf[i] = 0

        var written = self._lib.call["mxr_panel_get_text_content", UInt32](
            self._session, panel_id, node_id, buf, UInt32(buf_size)
        )

        var result = String("")
        for i in range(Int(written)):
            result += chr(Int(buf[i]))

        buf.free()
        return result

    fn panel_get_attribute_value(
        self, panel_id: UInt32, node_id: UInt32, name: String
    ) -> String:
        """Get the value of an attribute on a node in a panel (for testing).

        Args:
            panel_id: Target panel.
            node_id: Node to query.
            name: Attribute name.

        Returns:
            Attribute value, or empty string if not found.
        """
        var buf_size = 1024
        var buf = alloc[UInt8](buf_size)
        for i in range(buf_size):
            buf[i] = 0

        var name_ptr = name.unsafe_ptr()
        var written = self._lib.call["mxr_panel_get_attribute_value", UInt32](
            self._session,
            panel_id,
            node_id,
            name_ptr,
            UInt32(len(name)),
            buf,
            UInt32(buf_size),
        )

        var result = String("")
        for i in range(Int(written)):
            result += chr(Int(buf[i]))

        buf.free()
        return result

    fn panel_inject_event(
        self,
        panel_id: UInt32,
        handler_id: UInt32,
        event_type: UInt8,
        value: String,
    ):
        """Inject a synthetic event into a panel (for testing).

        Bypasses raycasting and directly enqueues an event for the
        given panel and handler.

        Args:
            panel_id: Target panel.
            handler_id: Handler ID to target.
            event_type: Event type constant (EVT_CLICK, etc.).
            value: String payload (e.g., input text).
        """
        var value_ptr = value.unsafe_ptr()
        self._lib.call["mxr_panel_inject_event", NoneType](
            self._session,
            panel_id,
            handler_id,
            event_type,
            value_ptr,
            UInt32(len(value)),
        )

    fn panel_get_child_mojo_id(
        self, panel_id: UInt32, parent_id: UInt32, index: UInt32
    ) -> UInt32:
        """Get the mojo element ID of a child node at a given index.

        Args:
            panel_id: Target panel.
            parent_id: Parent node ID.
            index: Zero-based child index.

        Returns:
            The mojo element ID, or 0 if not mapped.
        """
        return self._lib.call["mxr_panel_get_child_mojo_id", UInt32](
            self._session, panel_id, parent_id, index
        )

    fn version(self) -> String:
        """Get the shim version string.

        Returns:
            Version string (e.g., "0.2.0").
        """
        var buf_size = 32
        var buf = alloc[UInt8](buf_size)
        for i in range(buf_size):
            buf[i] = 0

        var written = self._lib.call["mxr_version", UInt32](
            buf, UInt32(buf_size)
        )

        var result = String("")
        for i in range(Int(written)):
            result += chr(Int(buf[i]))

        buf.free()
        return result
