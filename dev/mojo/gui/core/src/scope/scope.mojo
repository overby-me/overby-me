# ScopeState — Per-component instance state.
#
# Each mounted component gets a scope that tracks:
#   - Its position in the component tree (height, parent)
#   - Whether it needs re-rendering (dirty flag)
#   - Hook storage (signal keys, memo IDs, etc.)
#   - Render lifecycle (render count, hook cursor)
#   - Context map (key → value dependency injection)
#   - Error boundary state (catch errors from child components)
#   - Suspense state (show fallback while async children load)
#
# Hooks use positional indexing: the first `signal()` call in a component
# always maps to hook slot 0, the second to slot 1, etc.  On first render,
# hooks are created and stored.  On subsequent renders, the hook cursor
# advances through the existing slots, returning stable handles.
#
# The hook_values list stores UInt32 keys that refer to entries in the
# runtime's SignalStore (or future MemoStore, EffectStore).  A tag byte
# in hook_tags distinguishes the hook type.
#
# Context (Phase 8.3):
#   Scopes can provide key→value pairs that descendant scopes consume
#   without prop drilling.  Lookups walk up the parent chain until a
#   matching key is found.  Keys are UInt32 identifiers (the caller
#   chooses a unique key per "context type").  Values are Int32 for
#   simplicity (sufficient for signal keys, enum values, etc.).
#
# Error Boundaries (Phase 8.4):
#   A scope marked as an error boundary catches errors from descendant
#   scopes.  When a child sets an error, the nearest ancestor boundary
#   captures it and can render a fallback.  Clearing the error allows
#   the child to re-mount.
#
# Suspense (Phase 8.5):
#   A scope marked as a suspense boundary shows a fallback while any
#   descendant scope is in a "pending" state (waiting for async data).
#   When the pending scope resolves, the boundary re-renders with the
#   actual content.

from memory import UnsafePointer


# ── Hook type tags ───────────────────────────────────────────────────────────

comptime HOOK_SIGNAL: UInt8 = 0
comptime HOOK_MEMO: UInt8 = 1
comptime HOOK_EFFECT: UInt8 = 2


# ── ScopeState ───────────────────────────────────────────────────────────────


struct ScopeState(Copyable):
    """Per-component instance state.

    Tracks the component's position in the tree, its dirty status,
    hook storage for reactive primitives (signals, memos, effects),
    context map for dependency injection, error boundary state, and
    suspense state.
    """

    var id: UInt32
    var height: UInt32
    var parent_id: Int  # -1 if root scope (no parent)
    var dirty: Bool
    var render_count: UInt32
    var hook_cursor: Int  # current position during render (reset each render)

    # Hook storage: parallel arrays of (tag, value)
    # tag = HOOK_SIGNAL | HOOK_MEMO | HOOK_EFFECT
    # value = key into the appropriate runtime store
    var hook_tags: List[UInt8]
    var hook_values: List[UInt32]

    # Context map: parallel arrays of (key, value)
    # Scopes provide context entries that descendants can consume.
    # Lookups walk up the parent chain (handled by ScopeArena/Runtime).
    var context_keys: List[UInt32]
    var context_values: List[Int32]

    # Error boundary state
    var is_error_boundary: Bool  # True if this scope catches child errors
    var has_error: Bool  # True if an error has been captured
    var error_message: String  # Description of the captured error

    # Suspense state
    var is_suspense_boundary: Bool  # True if this scope shows fallback for pending children
    var is_pending: Bool  # True if this scope is waiting for async data

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self, id: UInt32, height: UInt32, parent_id: Int):
        """Create a new scope state.

        Args:
            id: The scope's unique identifier.
            height: Depth in the component tree (root = 0).
            parent_id: Parent scope ID, or -1 for root scopes.
        """
        self.id = id
        self.height = height
        self.parent_id = parent_id
        self.dirty = False
        self.render_count = 0
        self.hook_cursor = 0
        self.hook_tags = List[UInt8]()
        self.hook_values = List[UInt32]()
        self.context_keys = List[UInt32]()
        self.context_values = List[Int32]()
        self.is_error_boundary = False
        self.has_error = False
        self.error_message = String("")
        self.is_suspense_boundary = False
        self.is_pending = False

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.height = other.height
        self.parent_id = other.parent_id
        self.dirty = other.dirty
        self.render_count = other.render_count
        self.hook_cursor = other.hook_cursor
        self.hook_tags = other.hook_tags.copy()
        self.hook_values = other.hook_values.copy()
        self.context_keys = other.context_keys.copy()
        self.context_values = other.context_values.copy()
        self.is_error_boundary = other.is_error_boundary
        self.has_error = other.has_error
        self.error_message = other.error_message
        self.is_suspense_boundary = other.is_suspense_boundary
        self.is_pending = other.is_pending

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.height = other.height
        self.parent_id = other.parent_id
        self.dirty = other.dirty
        self.render_count = other.render_count
        self.hook_cursor = other.hook_cursor
        self.hook_tags = other.hook_tags^
        self.hook_values = other.hook_values^
        self.context_keys = other.context_keys^
        self.context_values = other.context_values^
        self.is_error_boundary = other.is_error_boundary
        self.has_error = other.has_error
        self.error_message = other.error_message^
        self.is_suspense_boundary = other.is_suspense_boundary
        self.is_pending = other.is_pending

    # ── Render lifecycle ─────────────────────────────────────────────

    fn begin_render(mut self):
        """Begin a render pass.

        Resets the hook cursor to 0 so hooks are accessed in order.
        Increments the render count.  Clears the dirty flag.
        """
        self.hook_cursor = 0
        self.render_count += 1
        self.dirty = False

    fn is_first_render(self) -> Bool:
        """Check if this scope has never been rendered.

        Returns True if render_count is 0 (before first begin_render)
        or 1 (during first render pass).
        """
        return self.render_count <= 1

    # ── Hook management ──────────────────────────────────────────────

    fn hook_count(self) -> Int:
        """Return the number of hooks registered in this scope."""
        return len(self.hook_values)

    fn push_hook(mut self, tag: UInt8, value: UInt32):
        """Register a new hook at the current cursor position.

        Called during first render to create a new hook slot.
        Advances the hook cursor.
        """
        self.hook_tags.append(tag)
        self.hook_values.append(value)
        self.hook_cursor += 1

    fn next_hook(mut self) -> UInt32:
        """Advance the hook cursor and return the value at the current position.

        Called during re-render to retrieve an existing hook's stored value.
        """
        var idx = self.hook_cursor
        self.hook_cursor += 1
        return self.hook_values[idx]

    fn next_hook_tag(self) -> UInt8:
        """Return the tag of the hook at the current cursor position.

        Does NOT advance the cursor — call next_hook() to advance.
        """
        return self.hook_tags[self.hook_cursor]

    fn has_more_hooks(self) -> Bool:
        """Check if there are more hooks to process in this render pass."""
        return self.hook_cursor < len(self.hook_values)

    fn hook_value_at(self, index: Int) -> UInt32:
        """Return the hook value at the given index (for introspection)."""
        return self.hook_values[index]

    fn hook_tag_at(self, index: Int) -> UInt8:
        """Return the hook tag at the given index (for introspection)."""
        return self.hook_tags[index]

    # ── Context (Dependency Injection) ───────────────────────────────

    fn provide_context(mut self, key: UInt32, value: Int32):
        """Provide a context value at this scope.

        If the key already exists in this scope's context map, the value
        is updated.  Otherwise, a new entry is appended.

        Args:
            key: A unique identifier for the context type.
            value: The Int32 value to provide (e.g. a signal key).
        """
        for i in range(len(self.context_keys)):
            if self.context_keys[i] == key:
                self.context_values[i] = value
                return
        self.context_keys.append(key)
        self.context_values.append(value)

    fn get_context(self, key: UInt32) -> Tuple[Bool, Int32]:
        """Look up a context value in THIS scope only.

        Returns (True, value) if found, (False, 0) if not.
        To walk up the parent chain, use ScopeArena.consume_context().

        Args:
            key: The context key to look up.

        Returns:
            A tuple of (found: Bool, value: Int32).
        """
        for i in range(len(self.context_keys)):
            if self.context_keys[i] == key:
                return Tuple(True, self.context_values[i])
        return Tuple(False, Int32(0))

    fn has_context(self, key: UInt32) -> Bool:
        """Check whether this scope provides a context for `key`.

        Does NOT walk up the parent chain.
        """
        for i in range(len(self.context_keys)):
            if self.context_keys[i] == key:
                return True
        return False

    fn context_count(self) -> Int:
        """Return the number of context entries in this scope."""
        return len(self.context_keys)

    fn remove_context(mut self, key: UInt32) -> Bool:
        """Remove a context entry from this scope.

        Returns True if the entry was found and removed, False otherwise.
        """
        for i in range(len(self.context_keys)):
            if self.context_keys[i] == key:
                # Swap-remove for O(1)
                var last = len(self.context_keys) - 1
                if i != last:
                    self.context_keys[i] = self.context_keys[last]
                    self.context_values[i] = self.context_values[last]
                _ = self.context_keys.pop()
                _ = self.context_values.pop()
                return True
        return False

    # ── Error Boundary ───────────────────────────────────────────────

    fn set_error_boundary(mut self, enabled: Bool):
        """Mark or unmark this scope as an error boundary.

        An error boundary catches errors from descendant scopes and can
        render a fallback UI instead of crashing.

        Args:
            enabled: True to mark as a boundary, False to unmark.
        """
        self.is_error_boundary = enabled

    fn set_error(mut self, message: String):
        """Set an error on this scope.

        Typically called by the runtime when a descendant scope reports
        an error and this scope is the nearest error boundary.

        Args:
            message: A description of the error.
        """
        self.has_error = True
        self.error_message = message

    fn clear_error(mut self):
        """Clear the error state on this scope.

        After clearing, the boundary can re-render its children normally
        instead of showing a fallback.
        """
        self.has_error = False
        self.error_message = String("")

    fn get_error_message(self) -> String:
        """Return the current error message, or empty string if no error."""
        return self.error_message

    # ── Suspense ─────────────────────────────────────────────────────

    fn set_suspense_boundary(mut self, enabled: Bool):
        """Mark or unmark this scope as a suspense boundary.

        A suspense boundary shows a fallback while any descendant scope
        is in a pending state (waiting for async data).

        Args:
            enabled: True to mark as a boundary, False to unmark.
        """
        self.is_suspense_boundary = enabled

    fn set_pending(mut self, pending: Bool):
        """Set the pending (async loading) state on this scope.

        When a scope is pending, its nearest suspense boundary ancestor
        shows a fallback instead of the actual content.

        Args:
            pending: True if waiting for async data, False when resolved.
        """
        self.is_pending = pending

    # ── Queries ──────────────────────────────────────────────────────

    fn is_root(self) -> Bool:
        """Check whether this is a root scope (no parent)."""
        return self.parent_id == -1

    fn mark_dirty(mut self):
        """Mark this scope as needing re-render."""
        self.dirty = True

    fn clear_dirty(mut self):
        """Clear the dirty flag (called after re-render)."""
        self.dirty = False
