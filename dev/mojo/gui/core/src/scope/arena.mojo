# ScopeArena — Slab-based scope storage.
#
# Manages a pool of ScopeState instances identified by UInt32 IDs.
# Uses the same slab pattern as SignalStore and ElementIdAllocator:
# an intrusive free list provides O(1) alloc/free with automatic
# reuse of freed IDs.
#
# Scope ID 0 is valid (unlike ElementId where 0 is reserved for root).
# The arena does not impose any hierarchy — parent/child relationships
# are tracked by ScopeState.parent_id and ScopeState.height.

from memory import UnsafePointer
from .scope import ScopeState


# ── Slot state ───────────────────────────────────────────────────────────────


@fieldwise_init
struct _ScopeSlotState(Copyable, Equatable, Writable):
    """Tracks whether a scope slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only meaningful when not occupied; -1 = end of free list.


# ── ScopeArena ───────────────────────────────────────────────────────────────


struct ScopeArena(Movable):
    """Slab-based storage for ScopeState instances.

    Each scope is identified by a UInt32 key returned from `create`.
    Freed keys are recycled for future allocations.

    Usage:
        var arena = ScopeArena()
        var id = arena.create(height=0, parent_id=-1)  # root scope
        arena.get(id).begin_render()
        arena.destroy(id)
    """

    var _scopes: List[ScopeState]
    var _states: List[_ScopeSlotState]
    var _free_head: Int
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        self._scopes = List[ScopeState]()
        self._states = List[_ScopeSlotState]()
        self._free_head = -1
        self._count = 0

    fn __init__(out self, *, capacity: Int):
        """Create an arena with pre-allocated capacity."""
        self._scopes = List[ScopeState](capacity=capacity)
        self._states = List[_ScopeSlotState](capacity=capacity)
        self._free_head = -1
        self._count = 0

    fn __moveinit__(out self, deinit other: Self):
        self._scopes = other._scopes^
        self._states = other._states^
        self._free_head = other._free_head
        self._count = other._count

    # ── Create / Destroy ─────────────────────────────────────────────

    fn create(mut self, height: UInt32, parent_id: Int) -> UInt32:
        """Create a new scope and return its ID.

        Args:
            height: Depth in the component tree (root = 0).
            parent_id: Parent scope ID as Int, or -1 for root scopes.

        Returns:
            The UInt32 scope ID.
        """
        if self._free_head != -1:
            # Reuse a freed slot
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._scopes[idx] = ScopeState(UInt32(idx), height, parent_id)
            self._states[idx] = _ScopeSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            # Append a new slot
            var idx = len(self._scopes)
            self._scopes.append(ScopeState(UInt32(idx), height, parent_id))
            self._states.append(_ScopeSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    fn create_child(mut self, parent_id: UInt32) -> UInt32:
        """Create a child scope whose height is parent.height + 1.

        Convenience method that reads the parent's height automatically.

        Args:
            parent_id: The parent scope's ID.

        Returns:
            The UInt32 scope ID of the new child.
        """
        var parent_height = self._scopes[Int(parent_id)].height
        return self.create(parent_height + 1, Int(parent_id))

    fn destroy(mut self, id: UInt32):
        """Destroy the scope at `id`, freeing its slot for reuse.

        Destroying a non-existent or already-freed scope is a no-op.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._scopes):
            return
        if not self._states[idx].occupied:
            return
        # Replace with a dummy scope — the old scope's storage is released
        self._scopes[idx] = ScopeState(UInt32(0), UInt32(0), -1)
        self._states[idx] = _ScopeSlotState(
            occupied=False, next_free=self._free_head
        )
        self._free_head = idx
        self._count -= 1

    # ── Access ───────────────────────────────────────────────────────

    fn get_ptr(
        self, id: UInt32
    ) -> UnsafePointer[ScopeState, MutExternalOrigin]:
        """Return a pointer to the ScopeState at `id`.

        The caller must ensure `id` refers to a live scope.
        The pointer is valid until the next mutation of the arena.
        """
        return self._scopes.unsafe_ptr() + Int(id)

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of live scopes."""
        return self._count

    fn contains(self, id: UInt32) -> Bool:
        """Check whether `id` is a live scope."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._states):
            return False
        return self._states[idx].occupied

    fn height(self, id: UInt32) -> UInt32:
        """Return the height (depth) of the scope at `id`."""
        return self._scopes[Int(id)].height

    fn parent_id(self, id: UInt32) -> Int:
        """Return the parent ID of the scope at `id`, or -1 if root."""
        return self._scopes[Int(id)].parent_id

    fn is_dirty(self, id: UInt32) -> Bool:
        """Check whether the scope at `id` is dirty (needs re-render)."""
        return self._scopes[Int(id)].dirty

    fn render_count(self, id: UInt32) -> UInt32:
        """Return how many times the scope at `id` has been rendered."""
        return self._scopes[Int(id)].render_count

    fn hook_count(self, id: UInt32) -> Int:
        """Return the number of hooks in the scope at `id`."""
        return self._scopes[Int(id)].hook_count()

    # ── Mutators (delegating to ScopeState) ──────────────────────────

    fn set_dirty(mut self, id: UInt32, dirty: Bool):
        """Set the dirty flag on the scope at `id`."""
        if dirty:
            self._scopes[Int(id)].mark_dirty()
        else:
            self._scopes[Int(id)].clear_dirty()

    fn begin_render(mut self, id: UInt32):
        """Begin a render pass on the scope at `id`.

        Resets hook cursor, increments render count, clears dirty flag.
        """
        self._scopes[Int(id)].begin_render()

    fn push_hook(mut self, id: UInt32, tag: UInt8, value: UInt32):
        """Push a new hook onto the scope at `id`."""
        self._scopes[Int(id)].push_hook(tag, value)

    fn next_hook(mut self, id: UInt32) -> UInt32:
        """Advance the hook cursor on scope `id` and return the current value.
        """
        return self._scopes[Int(id)].next_hook()

    fn has_more_hooks(self, id: UInt32) -> Bool:
        """Check if scope `id` has more hooks to process."""
        return self._scopes[Int(id)].has_more_hooks()

    fn is_first_render(self, id: UInt32) -> Bool:
        """Check if scope `id` is on its first render."""
        return self._scopes[Int(id)].is_first_render()

    fn hook_value_at(self, id: UInt32, index: Int) -> UInt32:
        """Return the hook value at position `index` in scope `id`."""
        return self._scopes[Int(id)].hook_value_at(index)

    fn hook_tag_at(self, id: UInt32, index: Int) -> UInt8:
        """Return the hook tag at position `index` in scope `id`."""
        return self._scopes[Int(id)].hook_tag_at(index)

    fn hook_cursor(self, id: UInt32) -> Int:
        """Return the current hook cursor position for scope `id`."""
        return self._scopes[Int(id)].hook_cursor

    # ── Bulk operations ──────────────────────────────────────────────

    fn clear(mut self):
        """Destroy all scopes."""
        self._scopes.clear()
        self._states.clear()
        self._free_head = -1
        self._count = 0

    # ── Context (Dependency Injection) ───────────────────────────────

    fn provide_context(mut self, scope_id: UInt32, key: UInt32, value: Int32):
        """Provide a context value at the given scope.

        If the key already exists in the scope's context map, the value
        is updated.  Otherwise, a new entry is appended.

        Args:
            scope_id: The scope to provide the context on.
            key: A unique identifier for the context type.
            value: The Int32 value to provide.
        """
        self._scopes[Int(scope_id)].provide_context(key, value)

    fn consume_context(
        self, scope_id: UInt32, key: UInt32
    ) -> Tuple[Bool, Int32]:
        """Look up a context value by walking up the scope tree.

        Starts at `scope_id` and checks each ancestor scope until a
        matching key is found or the root is reached.

        Args:
            scope_id: The scope to start searching from.
            key: The context key to look up.

        Returns:
            A tuple of (found: Bool, value: Int32).
        """
        var current = Int(scope_id)
        while current != -1:
            var idx = current
            if idx < 0 or idx >= len(self._scopes):
                break
            if not self._states[idx].occupied:
                break
            var result = self._scopes[idx].get_context(key)
            if result[0]:
                return result
            current = self._scopes[idx].parent_id
        return Tuple(False, Int32(0))

    fn has_context_local(self, scope_id: UInt32, key: UInt32) -> Bool:
        """Check whether the scope itself provides a context for `key`.

        Does NOT walk up the parent chain.
        """
        return self._scopes[Int(scope_id)].has_context(key)

    fn context_count(self, scope_id: UInt32) -> Int:
        """Return the number of context entries in the given scope."""
        return self._scopes[Int(scope_id)].context_count()

    fn remove_context(mut self, scope_id: UInt32, key: UInt32) -> Bool:
        """Remove a context entry from the given scope.

        Returns True if the entry was found and removed.
        """
        return self._scopes[Int(scope_id)].remove_context(key)

    # ── Error Boundaries ─────────────────────────────────────────────

    fn set_error_boundary(mut self, scope_id: UInt32, enabled: Bool):
        """Mark or unmark a scope as an error boundary.

        An error boundary catches errors from descendant scopes.
        """
        self._scopes[Int(scope_id)].set_error_boundary(enabled)

    fn is_error_boundary(self, scope_id: UInt32) -> Bool:
        """Check whether the scope is an error boundary."""
        return self._scopes[Int(scope_id)].is_error_boundary

    fn set_error(mut self, scope_id: UInt32, message: String):
        """Set an error on the scope (marks it as having an error)."""
        self._scopes[Int(scope_id)].set_error(message)

    fn clear_error(mut self, scope_id: UInt32):
        """Clear the error state on the scope."""
        self._scopes[Int(scope_id)].clear_error()

    fn has_error(self, scope_id: UInt32) -> Bool:
        """Check whether the scope has a captured error."""
        return self._scopes[Int(scope_id)].has_error

    fn get_error_message(self, scope_id: UInt32) -> String:
        """Return the error message on the scope, or empty string."""
        return self._scopes[Int(scope_id)].get_error_message()

    fn find_error_boundary(self, scope_id: UInt32) -> Int:
        """Walk up from `scope_id` to find the nearest error boundary.

        Returns the boundary scope's ID as Int, or -1 if none found.
        Does NOT check `scope_id` itself — only ancestors.
        """
        var current = self._scopes[Int(scope_id)].parent_id
        while current != -1:
            var idx = current
            if idx < 0 or idx >= len(self._scopes):
                break
            if not self._states[idx].occupied:
                break
            if self._scopes[idx].is_error_boundary:
                return current
            current = self._scopes[idx].parent_id
        return -1

    fn propagate_error(mut self, scope_id: UInt32, message: String) -> Int:
        """Propagate an error from `scope_id` to its nearest error boundary.

        Sets the error on the boundary scope and returns its ID.
        If no boundary is found, returns -1 (error is unhandled).

        Args:
            scope_id: The scope where the error originated.
            message: Description of the error.

        Returns:
            The boundary scope ID as Int, or -1 if unhandled.
        """
        var boundary = self.find_error_boundary(scope_id)
        if boundary != -1:
            self._scopes[boundary].set_error(message)
        return boundary

    # ── Suspense ─────────────────────────────────────────────────────

    fn set_suspense_boundary(mut self, scope_id: UInt32, enabled: Bool):
        """Mark or unmark a scope as a suspense boundary.

        A suspense boundary shows a fallback while any descendant
        scope is in a pending state.
        """
        self._scopes[Int(scope_id)].set_suspense_boundary(enabled)

    fn is_suspense_boundary(self, scope_id: UInt32) -> Bool:
        """Check whether the scope is a suspense boundary."""
        return self._scopes[Int(scope_id)].is_suspense_boundary

    fn set_pending(mut self, scope_id: UInt32, pending: Bool):
        """Set the pending (async loading) state on a scope."""
        self._scopes[Int(scope_id)].set_pending(pending)

    fn is_pending(self, scope_id: UInt32) -> Bool:
        """Check whether the scope is in a pending state."""
        return self._scopes[Int(scope_id)].is_pending

    fn find_suspense_boundary(self, scope_id: UInt32) -> Int:
        """Walk up from `scope_id` to find the nearest suspense boundary.

        Returns the boundary scope's ID as Int, or -1 if none found.
        Does NOT check `scope_id` itself — only ancestors.
        """
        var current = self._scopes[Int(scope_id)].parent_id
        while current != -1:
            var idx = current
            if idx < 0 or idx >= len(self._scopes):
                break
            if not self._states[idx].occupied:
                break
            if self._scopes[idx].is_suspense_boundary:
                return current
            current = self._scopes[idx].parent_id
        return -1

    fn has_pending_descendant(self, scope_id: UInt32) -> Bool:
        """Check if any live scope has `scope_id` as an ancestor and is pending.

        This is an O(n) scan over all live scopes.  For small trees
        this is fine; a future optimisation could maintain a pending-count
        per suspense boundary.
        """
        for i in range(len(self._scopes)):
            if not self._states[i].occupied:
                continue
            if not self._scopes[i].is_pending:
                continue
            # Walk up from this pending scope to see if scope_id is an ancestor
            var current = self._scopes[i].parent_id
            while current != -1:
                if current == Int(scope_id):
                    return True
                if current < 0 or current >= len(self._scopes):
                    break
                if not self._states[current].occupied:
                    break
                current = self._scopes[current].parent_id
        return False

    fn resolve_pending(mut self, scope_id: UInt32) -> Int:
        """Mark a scope as no longer pending and return its suspense boundary.

        Clears the pending flag and returns the nearest suspense boundary
        scope ID (as Int), or -1 if none.  The caller should re-render
        the boundary to replace the fallback with actual content.
        """
        self._scopes[Int(scope_id)].set_pending(False)
        return self.find_suspense_boundary(scope_id)
