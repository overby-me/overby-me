# Blitz FFI Bindings — Mojo wrappers for the libmojo_blitz C shim.
#
# This module provides typed Mojo wrappers around the Blitz desktop renderer
# C shim (libmojo_blitz.so / libmojo_blitz.dylib). The shim exposes Blitz's
# HTML/CSS rendering engine (Stylo + Taffy + Vello + Winit) via a flat C ABI.
#
# Architecture:
#
#   Mojo (this module)
#     │ DLHandle FFI calls
#     ▼
#   libmojo_blitz (Rust cdylib)
#     │ Rust API calls
#     ▼
#   Blitz (blitz-dom, blitz-shell, blitz-paint)
#     ├── Stylo     — CSS parsing & style resolution
#     ├── Taffy     — Flexbox, grid, block layout
#     ├── Parley    — Text layout & shaping
#     ├── Vello     — GPU-accelerated 2D rendering
#     ├── Winit     — Cross-platform windowing & input
#     └── AccessKit — Accessibility
#
# The shim uses a polling-based design (no callbacks across FFI) with an
# internal event ring buffer, matching the pattern established by the
# webview shim (mojo_webview.h).
#
# Library search order:
#   1. MOJO_BLITZ_LIB environment variable (explicit path)
#   2. NIX_LDFLAGS (Nix dev shell library paths)
#   3. LD_LIBRARY_PATH / DYLD_LIBRARY_PATH
#   4. Common system paths (/usr/local/lib, /usr/lib)
#
# Usage:
#
#   var blitz = Blitz.create("My App", 800, 600, debug=False)
#   # ... build DOM via mutation interpreter ...
#   blitz.request_redraw()
#   while blitz.is_alive():
#       _ = blitz.step(blocking=False)
#       while True:
#           var event = blitz.poll_event()
#           if not event.valid:
#               break
#           # dispatch event to GuiApp.handle_event()
#       if has_dirty:
#           # flush mutations via mutation interpreter
#           blitz.request_redraw()
#       else:
#           _ = blitz.step(blocking=True)
#   blitz.destroy()

from memory import UnsafePointer, alloc
from os import getenv
from sys.ffi import _DLHandle


# ══════════════════════════════════════════════════════════════════════════════
# BlitzEvent — A buffered event returned by poll_event()
# ══════════════════════════════════════════════════════════════════════════════


struct BlitzEvent(Copyable, Movable):
    """A buffered DOM event from the Blitz shim's ring buffer.

    Fields:
        valid:      True if this event is valid, False if the queue was empty.
        handler_id: The handler ID registered via add_event_listener().
                    Corresponds to the handler_id from OP_NEW_EVENT_LISTENER.
        event_type: One of the EVT_* constants (0=click, 1=input, etc.).
        value:      String payload (e.g., input text). Empty if not applicable.
    """

    var valid: Bool
    var handler_id: UInt32
    var event_type: UInt8
    var value: String

    fn __init__(out self):
        """Create an invalid (empty) event."""
        self.valid = False
        self.handler_id = 0
        self.event_type = 0
        self.value = String("")

    fn __init__(
        out self,
        valid: Bool,
        handler_id: UInt32,
        event_type: UInt8,
        value: String,
    ):
        self.valid = valid
        self.handler_id = handler_id
        self.event_type = event_type
        self.value = value

    fn __copyinit__(out self, other: Self):
        self.valid = other.valid
        self.handler_id = other.handler_id
        self.event_type = other.event_type
        self.value = other.value

    fn __moveinit__(out self, deinit other: Self):
        self.valid = other.valid
        self.handler_id = other.handler_id
        self.event_type = other.event_type
        self.value = other.value^


# ══════════════════════════════════════════════════════════════════════════════
# Library name constant
# ══════════════════════════════════════════════════════════════════════════════

comptime _LIB_NAME = "libmojo_blitz.so"
comptime _LIB_NAME_DYLIB = "libmojo_blitz.dylib"
comptime _LIB_NAME_DLL = "mojo_blitz.dll"


fn _is_windows_target() -> Bool:
    """Return True if the current compilation target is Windows.

    Uses the MOJO_TARGET_WINDOWS compile-time define, which must be passed
    via `-D MOJO_TARGET_WINDOWS` when cross-compiling for Windows.
    """
    from sys.param_env import is_defined

    return is_defined["MOJO_TARGET_WINDOWS"]()


fn _lib_name() -> String:
    """Return the platform-appropriate shared library filename."""

    @parameter
    if _is_windows_target():
        return _LIB_NAME_DLL
    else:
        return _LIB_NAME


fn _path_sep() -> String:
    """Return the platform-appropriate path separator."""

    @parameter
    if _is_windows_target():
        return "\\"
    else:
        return "/"


# ══════════════════════════════════════════════════════════════════════════════
# _find_library — search for the shared library
# ══════════════════════════════════════════════════════════════════════════════


fn _find_library() -> String:
    """Search for the Blitz shim shared library.

    Search order:
      1. MOJO_BLITZ_LIB env var (directory path)
      2. NIX_LDFLAGS (extract -L paths)
      3. LD_LIBRARY_PATH / DYLD_LIBRARY_PATH
      4. Common system paths

    Returns the full path to the library, or just the library name
    if not found (letting the dynamic linker try its default search).
    """
    # 1. Explicit env var
    var sep = _path_sep()
    var name = _lib_name()

    var lib_dir = getenv("MOJO_BLITZ_LIB")
    if len(lib_dir) > 0:
        return lib_dir + sep + name

    # 2. NIX_LDFLAGS — parse -L/nix/store/... paths
    var nix_flags = getenv("NIX_LDFLAGS")
    if len(nix_flags) > 0:
        # Simple parsing: split on spaces, look for -L flags
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
                            # Check if the library exists in this directory
                            # We can't do filesystem checks easily, so just
                            # try the path. The dynamic linker will reject
                            # it if it doesn't exist.
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
# Blitz — Typed wrapper around the Blitz C shim
# ══════════════════════════════════════════════════════════════════════════════


struct Blitz(Movable):
    """Mojo FFI wrapper for the Blitz desktop renderer C shim.

    This struct manages the lifecycle of a Blitz application context,
    providing typed methods for all shim operations: window management,
    DOM manipulation, event handling, and template management.

    The context is created via `Blitz.create()` and destroyed via
    `destroy()` or when the struct is moved/dropped.
    """

    var _lib: _DLHandle
    var _ctx: UnsafePointer[NoneType, MutAnyOrigin]

    fn __init__(
        out self, lib: _DLHandle, ctx: UnsafePointer[NoneType, MutAnyOrigin]
    ):
        """Private initializer. Use Blitz.create() instead."""
        self._lib = lib
        self._ctx = ctx

    fn __moveinit__(out self, deinit other: Self):
        self._lib = other._lib
        self._ctx = other._ctx

    # ── Factory ──────────────────────────────────────────────────────────

    @staticmethod
    fn create(
        title: String, width: Int, height: Int, debug: Bool = False
    ) raises -> Self:
        """Create a Blitz application context with a native window.

        Loads the libmojo_blitz shared library and initializes a Blitz
        rendering context with the given window configuration.

        Args:
            title: Window title.
            width: Initial window width in logical pixels.
            height: Initial window height in logical pixels.
            debug: Enable debug overlays (layout borders, etc.).

        Returns:
            A new Blitz instance.

        Raises:
            If the shared library cannot be loaded.
        """
        var lib_path = _find_library()
        var lib = _DLHandle(lib_path)

        var title_ptr = title.unsafe_ptr()
        var title_len = UInt32(len(title))
        var debug_flag = Int32(1) if debug else Int32(0)

        var ctx = lib.call[
            "mblitz_create", UnsafePointer[NoneType, MutAnyOrigin]
        ](
            title_ptr,
            title_len,
            UInt32(width),
            UInt32(height),
            debug_flag,
        )

        return Self(lib, ctx)

    # ── Lifecycle ────────────────────────────────────────────────────────

    fn step(self, blocking: Bool = False) -> Bool:
        """Process pending window/input events.

        Args:
            blocking: If True, block until an event arrives (use when idle).

        Returns:
            True if events were processed.
        """
        var blocking_flag = Int32(1) if blocking else Int32(0)
        var result = self._lib.call["mblitz_step", Int32](
            self._ctx, blocking_flag
        )
        return result != 0

    fn is_alive(self) -> Bool:
        """Check if the window is still open.

        Returns:
            True if the window is alive, False if closed.
        """
        var result = self._lib.call["mblitz_is_alive", Int32](self._ctx)
        return result != 0

    fn request_redraw(self):
        """Request a redraw of the window.

        Call after applying mutations to trigger re-layout and re-paint.
        """
        self._lib.call["mblitz_request_redraw", NoneType](self._ctx)

    fn destroy(mut self):
        """Destroy the Blitz context and close the window.

        The context is invalid after this call.
        """
        if self._ctx:
            self._lib.call["mblitz_destroy", NoneType](self._ctx)
            self._ctx = UnsafePointer[NoneType, MutAnyOrigin]()

    # ── Window management ────────────────────────────────────────────────

    fn set_title(self, title: String):
        """Set the window title."""
        var title_ptr = title.unsafe_ptr()
        self._lib.call["mblitz_set_title", NoneType](
            self._ctx, title_ptr, UInt32(len(title))
        )

    fn set_size(self, width: Int, height: Int):
        """Resize the window."""
        self._lib.call["mblitz_set_size", NoneType](
            self._ctx, UInt32(width), UInt32(height)
        )

    # ── User-agent stylesheet ────────────────────────────────────────────

    fn add_ua_stylesheet(self, css: String):
        """Add a user-agent stylesheet (CSS string)."""
        var css_ptr = css.unsafe_ptr()
        self._lib.call["mblitz_add_ua_stylesheet", NoneType](
            self._ctx, css_ptr, UInt32(len(css))
        )

    # ── DOM node creation ────────────────────────────────────────────────

    fn create_element(self, tag: String) -> UInt32:
        """Create an HTML element node (detached).

        Args:
            tag: HTML tag name (e.g., "div", "button", "h1").

        Returns:
            Blitz node ID of the new element.
        """
        var tag_ptr = tag.unsafe_ptr()
        return self._lib.call["mblitz_create_element", UInt32](
            self._ctx, tag_ptr, UInt32(len(tag))
        )

    fn create_text_node(self, text: String) -> UInt32:
        """Create a text node (detached).

        Args:
            text: Text content.

        Returns:
            Blitz node ID of the new text node.
        """
        var text_ptr = text.unsafe_ptr()
        return self._lib.call["mblitz_create_text_node", UInt32](
            self._ctx, text_ptr, UInt32(len(text))
        )

    fn create_placeholder(self) -> UInt32:
        """Create a comment/placeholder node (detached).

        Returns:
            Blitz node ID of the placeholder.
        """
        return self._lib.call["mblitz_create_placeholder", UInt32](self._ctx)

    # ── Templates ────────────────────────────────────────────────────────

    fn register_template(self, tmpl_id: UInt32, root_id: UInt32):
        """Register a template subtree for efficient cloning.

        Args:
            tmpl_id: Template ID from mojo-gui's template registry.
            root_id: Blitz node ID of the pre-built template root.
        """
        self._lib.call["mblitz_register_template", NoneType](
            self._ctx, tmpl_id, root_id
        )

    fn clone_template(self, tmpl_id: UInt32) -> UInt32:
        """Deep-clone a registered template.

        Args:
            tmpl_id: Template ID (previously registered).

        Returns:
            Blitz node ID of the cloned root. 0 if not registered.
        """
        return self._lib.call["mblitz_clone_template", UInt32](
            self._ctx, tmpl_id
        )

    # ── DOM tree mutations ───────────────────────────────────────────────

    fn append_children(
        self,
        parent_id: UInt32,
        child_ids: UnsafePointer[UInt32],
        child_count: UInt32,
    ):
        """Append children to a parent element.

        Args:
            parent_id: Parent node ID (0 = mount point).
            child_ids: Pointer to array of child node IDs.
            child_count: Number of children.
        """
        self._lib.call["mblitz_append_children", NoneType](
            self._ctx, parent_id, child_ids, child_count
        )

    fn insert_before(
        self,
        anchor_id: UInt32,
        new_ids: UnsafePointer[UInt32],
        new_count: UInt32,
    ):
        """Insert nodes before an anchor node.

        Args:
            anchor_id: Anchor node ID.
            new_ids: Pointer to array of new node IDs.
            new_count: Number of new nodes.
        """
        self._lib.call["mblitz_insert_before", NoneType](
            self._ctx, anchor_id, new_ids, new_count
        )

    fn insert_after(
        self,
        anchor_id: UInt32,
        new_ids: UnsafePointer[UInt32],
        new_count: UInt32,
    ):
        """Insert nodes after an anchor node.

        Args:
            anchor_id: Anchor node ID.
            new_ids: Pointer to array of new node IDs.
            new_count: Number of new nodes.
        """
        self._lib.call["mblitz_insert_after", NoneType](
            self._ctx, anchor_id, new_ids, new_count
        )

    fn replace_with(
        self,
        old_id: UInt32,
        new_ids: UnsafePointer[UInt32],
        new_count: UInt32,
    ):
        """Replace a node with new nodes.

        Args:
            old_id: Node ID to replace.
            new_ids: Pointer to array of replacement node IDs.
            new_count: Number of replacements.
        """
        self._lib.call["mblitz_replace_with", NoneType](
            self._ctx, old_id, new_ids, new_count
        )

    fn remove_node(self, node_id: UInt32):
        """Remove and drop a node from the DOM.

        Args:
            node_id: Node ID to remove.
        """
        self._lib.call["mblitz_remove_node", NoneType](self._ctx, node_id)

    # ── DOM node attributes ──────────────────────────────────────────────

    fn set_attribute(self, node_id: UInt32, name: String, value: String):
        """Set an attribute on an element.

        Args:
            node_id: Target element node ID.
            name: Attribute name (e.g., "class", "style", "id").
            value: Attribute value.
        """
        var name_ptr = name.unsafe_ptr()
        var value_ptr = value.unsafe_ptr()
        self._lib.call["mblitz_set_attribute", NoneType](
            self._ctx,
            node_id,
            name_ptr,
            UInt32(len(name)),
            value_ptr,
            UInt32(len(value)),
        )

    fn remove_attribute(self, node_id: UInt32, name: String):
        """Remove an attribute from an element.

        Args:
            node_id: Target element node ID.
            name: Attribute name to remove.
        """
        var name_ptr = name.unsafe_ptr()
        self._lib.call["mblitz_remove_attribute", NoneType](
            self._ctx,
            node_id,
            name_ptr,
            UInt32(len(name)),
        )

    # ── DOM text content ─────────────────────────────────────────────────

    fn set_text_content(self, node_id: UInt32, text: String):
        """Set the text content of a text node.

        Args:
            node_id: Text node ID.
            text: New text content.
        """
        var text_ptr = text.unsafe_ptr()
        self._lib.call["mblitz_set_text_content", NoneType](
            self._ctx, node_id, text_ptr, UInt32(len(text))
        )

    # ── DOM tree traversal ───────────────────────────────────────────────

    fn node_at_path(
        self,
        start_id: UInt32,
        path: UnsafePointer[UInt8],
        path_len: UInt32,
    ) -> UInt32:
        """Navigate to a child at the given path from a starting node.

        The path is an array of child indices (e.g., [0, 2, 1] means
        child 0 → child 2 → child 1).

        Args:
            start_id: Starting node ID.
            path: Array of child indices.
            path_len: Length of the path array.

        Returns:
            Node ID at the end of the path. 0 on failure.
        """
        return self._lib.call["mblitz_node_at_path", UInt32](
            self._ctx, start_id, path, path_len
        )

    fn child_at(self, node_id: UInt32, index: UInt32) -> UInt32:
        """Get the Nth child of a node.

        Args:
            node_id: Parent node ID.
            index: Zero-based child index.

        Returns:
            Child node ID. 0 if index is out of bounds.
        """
        return self._lib.call["mblitz_child_at", UInt32](
            self._ctx, node_id, index
        )

    fn child_count(self, node_id: UInt32) -> UInt32:
        """Get the number of children of a node.

        Args:
            node_id: Node ID.

        Returns:
            Number of children.
        """
        return self._lib.call["mblitz_child_count", UInt32](self._ctx, node_id)

    # ── Event handling ───────────────────────────────────────────────────

    fn add_event_listener(
        self, node_id: UInt32, handler_id: UInt32, event_name: String
    ):
        """Register an event handler on a node.

        Args:
            node_id: Target element node ID.
            handler_id: Unique handler ID (from HandlerRegistry).
            event_name: Event type name (e.g., "click", "input").
        """
        var name_ptr = event_name.unsafe_ptr()
        self._lib.call["mblitz_add_event_listener", NoneType](
            self._ctx,
            node_id,
            handler_id,
            name_ptr,
            UInt32(len(event_name)),
        )

    fn remove_event_listener(self, node_id: UInt32, event_name: String):
        """Remove an event handler from a node.

        Args:
            node_id: Target element node ID.
            event_name: Event type name to remove.
        """
        var name_ptr = event_name.unsafe_ptr()
        self._lib.call["mblitz_remove_event_listener", NoneType](
            self._ctx,
            node_id,
            name_ptr,
            UInt32(len(event_name)),
        )

    fn poll_event(self) -> BlitzEvent:
        """Poll the next event from the ring buffer.

        Uses `mblitz_poll_event_into` which writes event fields to
        caller-provided output pointers, avoiding struct-return ABI
        issues with Mojo's DLHandle.

        Returns:
            A BlitzEvent with valid=True if an event was available,
            or valid=False if the queue is empty.
        """
        # Allocate output slots for each event field.
        # For the value pointer we use alloc[Int] (pointer-sized) because
        # alloc[UnsafePointer[UInt8]] fails to infer the origin parameter
        # in Mojo 26.1. This follows the pattern from launcher.mojo.
        var out_handler_id = alloc[UInt32](1)
        var out_event_type = alloc[UInt8](1)
        var out_value_ptr = alloc[Int](1)
        var out_value_len = alloc[UInt32](1)

        out_handler_id[0] = 0
        out_event_type[0] = 0
        out_value_ptr[0] = 0
        out_value_len[0] = 0

        var valid = self._lib.call["mblitz_poll_event_into", Int32](
            self._ctx,
            out_handler_id,
            out_event_type,
            out_value_ptr,
            out_value_len,
        )

        if valid == 0:
            out_handler_id.free()
            out_event_type.free()
            out_value_ptr.free()
            out_value_len.free()
            return BlitzEvent()

        var handler_id = out_handler_id[0]
        var event_type = out_event_type[0]
        var v_ptr_int = out_value_ptr[0]
        var v_len = Int(out_value_len[0])

        out_handler_id.free()
        out_event_type.free()
        out_value_ptr.free()
        out_value_len.free()

        # Build the value string from the pointer + length.
        # The pointer points into BlitzContext.last_polled_value which
        # stays alive until the next poll call.
        # We use the alloc[Int] + bitcast pattern from launcher.mojo to
        # recover a typed pointer from the raw Int address.
        var value = String("")
        if v_len > 0 and v_ptr_int != 0:
            var slot = alloc[Int](1)
            slot[0] = v_ptr_int
            var v_ptr = slot.bitcast[UnsafePointer[UInt8, MutAnyOrigin]]()[0]
            slot.free()
            for i in range(v_len):
                value += chr(Int(v_ptr[i]))

        return BlitzEvent(
            valid=True,
            handler_id=handler_id,
            event_type=event_type,
            value=value,
        )

    fn event_count(self) -> UInt32:
        """Get the number of buffered events.

        Returns:
            Number of events waiting to be polled.
        """
        return self._lib.call["mblitz_event_count", UInt32](self._ctx)

    fn event_clear(self):
        """Clear all buffered events."""
        self._lib.call["mblitz_event_clear", NoneType](self._ctx)

    # ── Mutation batching ────────────────────────────────────────────────

    fn begin_mutations(self):
        """Begin a mutation batch.

        All DOM mutations between begin_mutations() and end_mutations()
        are batched for efficiency.
        """
        self._lib.call["mblitz_begin_mutations", NoneType](self._ctx)

    fn end_mutations(self):
        """End a mutation batch and flush deferred operations."""
        self._lib.call["mblitz_end_mutations", NoneType](self._ctx)

    # ── Stack operations (for mutation interpreter) ──────────────────────

    fn stack_push(self, node_id: UInt32):
        """Push a node ID onto the interpreter stack.

        Args:
            node_id: Node ID to push (mojo element ID or Blitz node ID).
        """
        self._lib.call["mblitz_stack_push", NoneType](self._ctx, node_id)

    fn stack_pop_append(self, parent_id: UInt32, count: UInt32):
        """Pop N nodes from the stack and append them as children.

        Args:
            parent_id: Parent node ID (0 = mount point).
            count: Number of nodes to pop and append.
        """
        self._lib.call["mblitz_stack_pop_append", NoneType](
            self._ctx, parent_id, count
        )

    fn stack_pop_replace(self, old_id: UInt32, count: UInt32):
        """Pop N nodes from the stack and replace an existing node.

        Args:
            old_id: Node ID to replace.
            count: Number of replacement nodes to pop.
        """
        self._lib.call["mblitz_stack_pop_replace", NoneType](
            self._ctx, old_id, count
        )

    fn stack_pop_insert_before(self, anchor_id: UInt32, count: UInt32):
        """Pop N nodes from the stack and insert before anchor.

        Args:
            anchor_id: Anchor node ID.
            count: Number of nodes to pop and insert.
        """
        self._lib.call["mblitz_stack_pop_insert_before", NoneType](
            self._ctx, anchor_id, count
        )

    fn stack_pop_insert_after(self, anchor_id: UInt32, count: UInt32):
        """Pop N nodes from the stack and insert after anchor.

        Args:
            anchor_id: Anchor node ID.
            count: Number of nodes to pop and insert.
        """
        self._lib.call["mblitz_stack_pop_insert_after", NoneType](
            self._ctx, anchor_id, count
        )

    # ── ID mapping ───────────────────────────────────────────────────────

    fn assign_id(self, mojo_id: UInt32, blitz_node_id: UInt32):
        """Assign a mojo-gui element ID to a Blitz node ID.

        Used by the mutation interpreter when processing OP_ASSIGN_ID.

        Args:
            mojo_id: The mojo-gui element ID.
            blitz_node_id: The Blitz slab node ID.
        """
        self._lib.call["mblitz_assign_id", NoneType](
            self._ctx, mojo_id, blitz_node_id
        )

    # ── Document root access ─────────────────────────────────────────────

    fn root_node_id(self) -> UInt32:
        """Get the document root node ID (always 0 in Blitz)."""
        return self._lib.call["mblitz_root_node_id", UInt32](self._ctx)

    fn mount_point_id(self) -> UInt32:
        """Get the mount point node ID (where app DOM is attached)."""
        return self._lib.call["mblitz_mount_point_id", UInt32](self._ctx)

    # ── Layout ───────────────────────────────────────────────────────────

    fn resolve_layout(self):
        """Force synchronous layout computation."""
        self._lib.call["mblitz_resolve_layout", NoneType](self._ctx)

    # ── User-agent stylesheet ────────────────────────────────────────────

    # (Already defined above under "User-agent stylesheet")

    # ── Debug / diagnostics ──────────────────────────────────────────────

    fn print_tree(self):
        """Print the DOM tree to stderr (for debugging)."""
        self._lib.call["mblitz_print_tree", NoneType](self._ctx)

    fn set_debug_overlay(self, enabled: Bool):
        """Toggle debug overlay visibility.

        Args:
            enabled: True to enable layout borders / node IDs overlay.
        """
        var flag = Int32(1) if enabled else Int32(0)
        self._lib.call["mblitz_set_debug_overlay", NoneType](self._ctx, flag)
