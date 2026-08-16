# MemoStore — Slab-allocated storage for memo (computed/derived signal) entries.
#
# MEMO_NO_STRING is a sentinel value indicating that a memo does NOT store
# a string in the StringStore.  We use 0xFFFFFFFF (max UInt32) because
# StringStore keys are sequential integers starting from 0, so a real key
# will never reach this value in practice.
#
# A memo is a cached derived value with its own reactive context that
# auto-tracks input signal dependencies and notifies downstream subscribers
# when dirty.
#
# Each memo has:
#   - context_id:  A reactive context ID.  When set as the current context
#     during computation, signal reads are auto-subscribed to this context.
#   - output_key:  A signal key that stores the cached result.  Other
#     scopes/memos can subscribe to this signal via the normal signal
#     subscription mechanism.
#   - scope_id:    The owning scope (for cleanup when the scope is destroyed).
#   - dirty:       Whether the memo needs recomputation.
#   - computing:   Whether the memo is currently inside a begin/end compute
#     bracket (guards against re-entrant computation).
#
# Lifecycle:
#   1. create → allocates context + output signal, marked dirty
#   2. begin_compute → sets memo's context as current reactive context
#   3. (app reads input signals — auto-subscribed to memo's context)
#   4. end_compute(value) → stores result in output signal, clears dirty
#   5. read → returns cached value from output signal
#   6. (input signal written → memo's context notified → memo marked dirty
#      → output signal's subscribers notified)
#
# The store uses a slab allocator (free-list) identical to SignalStore,
# HandlerRegistry, and ScopeArena, so memo IDs are stable and reusable.

from memory import UnsafePointer

comptime MEMO_NO_STRING: UInt32 = UInt32(0xFFFFFFFF)
"""Sentinel: this memo has no StringStore entry."""


# ── MemoEntry ────────────────────────────────────────────────────────────────


struct MemoEntry(Copyable, Equatable, Writable):
    """A cached derived value with dependency tracking.

    A memo has its own reactive context (context_id) that records which
    signals it reads during computation.  The cached result is stored in
    a dedicated signal (output_key) so that other scopes/memos can
    subscribe to it via the normal signal subscription mechanism.
    """

    var context_id: UInt32  # reactive context for dependency tracking
    var output_key: UInt32  # signal key that stores the cached result
    var string_key: UInt32  # StringStore key (0 for non-string memos)
    var scope_id: UInt32  # owning scope (for cleanup)
    var dirty: Bool  # needs recomputation
    var computing: Bool  # currently inside begin/end compute bracket
    var value_changed: Bool  # True if last end_compute wrote a different value

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create an empty (default) memo entry."""
        self.context_id = 0
        self.output_key = 0
        self.string_key = MEMO_NO_STRING
        self.scope_id = 0
        self.dirty = False
        self.computing = False
        self.value_changed = True

    fn __init__(
        out self,
        context_id: UInt32,
        output_key: UInt32,
        scope_id: UInt32,
    ):
        """Create a memo entry with the given IDs.

        The memo starts dirty (needs first computation) and not computing.

        Args:
            context_id: The reactive context ID for dependency tracking.
            output_key: The signal key for the cached result.
            scope_id: The owning scope ID.
        """
        self.context_id = context_id
        self.output_key = output_key
        self.string_key = MEMO_NO_STRING
        self.scope_id = scope_id
        self.dirty = True
        self.computing = False
        self.value_changed = True

    fn __init__(
        out self,
        context_id: UInt32,
        output_key: UInt32,
        string_key: UInt32,
        scope_id: UInt32,
    ):
        """Create a memo entry with string storage support.

        The memo starts dirty (needs first computation) and not computing.
        For non-string memos, pass string_key=0.

        Args:
            context_id: The reactive context ID for dependency tracking.
            output_key: The signal key for the cached result (or version signal for strings).
            string_key: The StringStore key (0 for non-string memos).
            scope_id: The owning scope ID.
        """
        self.context_id = context_id
        self.output_key = output_key
        self.string_key = string_key
        self.scope_id = scope_id
        self.dirty = True
        self.computing = False
        self.value_changed = True

    fn __copyinit__(out self, other: Self):
        self.context_id = other.context_id
        self.output_key = other.output_key
        self.string_key = other.string_key
        self.scope_id = other.scope_id
        self.dirty = other.dirty
        self.computing = other.computing
        self.value_changed = other.value_changed

    fn __moveinit__(out self, deinit other: Self):
        self.context_id = other.context_id
        self.output_key = other.output_key
        self.string_key = other.string_key
        self.scope_id = other.scope_id
        self.dirty = other.dirty
        self.computing = other.computing
        self.value_changed = other.value_changed


# ── Slot state for the memo store ────────────────────────────────────────────


@fieldwise_init
struct MemoSlotState(Copyable, Equatable, Writable):
    """Tracks whether a memo slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only valid when not occupied; -1 = end of free list.


# ── MemoStore ────────────────────────────────────────────────────────────────


struct MemoStore(Movable):
    """Slab-allocated storage for memo entries.

    Mirrors SignalStore and HandlerRegistry: uses a free-list for
    stable ID reuse after destruction.

    Usage:
        var store = MemoStore()
        var id = store.create(context_id=10, output_key=5, scope_id=0)
        store.mark_dirty(id)
        assert store.is_dirty(id)
        store.clear_dirty(id)
        store.destroy(id)
    """

    var _entries: List[MemoEntry]
    var _states: List[MemoSlotState]
    var _free_head: Int
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        self._entries = List[MemoEntry]()
        self._states = List[MemoSlotState]()
        self._free_head = -1
        self._count = 0

    fn __moveinit__(out self, deinit other: Self):
        self._entries = other._entries^
        self._states = other._states^
        self._free_head = other._free_head
        self._count = other._count

    # ── Create / Destroy ─────────────────────────────────────────────

    fn create(
        mut self,
        context_id: UInt32,
        output_key: UInt32,
        scope_id: UInt32,
    ) -> UInt32:
        """Create a new memo entry.  Returns its stable ID.

        The memo starts dirty (needs first computation).

        Args:
            context_id: The reactive context ID for dependency tracking.
            output_key: The signal key for the cached result.
            scope_id: The owning scope ID.

        Returns:
            The UInt32 memo ID.
        """
        var entry = MemoEntry(context_id, output_key, scope_id)

        if self._free_head != -1:
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._entries[idx] = entry^
            self._states[idx] = MemoSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            var idx = len(self._entries)
            self._entries.append(entry^)
            self._states.append(MemoSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    fn create(
        mut self,
        context_id: UInt32,
        output_key: UInt32,
        string_key: UInt32,
        scope_id: UInt32,
    ) -> UInt32:
        """Create a new memo entry with string storage.  Returns its stable ID.

        The memo starts dirty (needs first computation).
        For string memos, string_key is the StringStore key; output_key
        is the version signal for subscriber tracking.

        Args:
            context_id: The reactive context ID for dependency tracking.
            output_key: The version signal key (for string change tracking).
            string_key: The StringStore key for the cached string.
            scope_id: The owning scope ID.

        Returns:
            The UInt32 memo ID.
        """
        var entry = MemoEntry(context_id, output_key, string_key, scope_id)

        if self._free_head != -1:
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._entries[idx] = entry^
            self._states[idx] = MemoSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            var idx = len(self._entries)
            self._entries.append(entry^)
            self._states.append(MemoSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    fn destroy(mut self, id: UInt32):
        """Remove the memo at `id`, freeing its slot for reuse.

        Destroying a non-existent or already-freed memo is a no-op.
        Does NOT clean up the underlying context or output signal —
        the caller (Runtime) is responsible for that.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx] = MemoEntry()
        self._states[idx] = MemoSlotState(
            occupied=False, next_free=self._free_head
        )
        self._free_head = idx
        self._count -= 1

    # ── Access ───────────────────────────────────────────────────────

    fn get(self, id: UInt32) -> MemoEntry:
        """Return a copy of the memo entry at `id`.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].copy()

    fn get_ptr(
        mut self, id: UInt32
    ) -> UnsafePointer[MemoEntry, MutExternalOrigin]:
        """Return a pointer to the memo entry at `id`.

        The pointer is valid until the next mutation of the store.
        Precondition: `contains(id)` is True.
        """
        return UnsafePointer.address_of(self._entries[Int(id)])

    # ── Dirty tracking ───────────────────────────────────────────────

    fn mark_dirty(mut self, id: UInt32):
        """Mark the memo as needing recomputation.

        Called when an input signal that the memo depends on is written.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].dirty = True

    fn clear_dirty(mut self, id: UInt32):
        """Clear the dirty flag (called after successful recomputation)."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].dirty = False

    fn is_dirty(self, id: UInt32) -> Bool:
        """Check whether the memo needs recomputation.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].dirty

    # ── Computing state ──────────────────────────────────────────────

    fn set_computing(mut self, id: UInt32, computing: Bool):
        """Set the computing flag on the memo.

        True while the memo is inside a begin_compute / end_compute bracket.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].computing = computing

    fn is_computing(self, id: UInt32) -> Bool:
        """Check whether the memo is currently being computed.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].computing

    # ── Field accessors ──────────────────────────────────────────────

    fn context_id(self, id: UInt32) -> UInt32:
        """Return the reactive context ID of the memo.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].context_id

    fn output_key(self, id: UInt32) -> UInt32:
        """Return the output signal key of the memo.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].output_key

    fn scope_id(self, id: UInt32) -> UInt32:
        """Return the owning scope ID of the memo.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].scope_id

    fn string_key(self, id: UInt32) -> UInt32:
        """Return the StringStore key of the memo (0 for non-string memos).

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].string_key

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of live memos."""
        return self._count

    fn contains(self, id: UInt32) -> Bool:
        """Check whether `id` is a live memo."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._states):
            return False
        return self._states[idx].occupied

    # ── Bulk operations ──────────────────────────────────────────────

    fn remove_for_scope(mut self, scope_id: UInt32) -> List[UInt32]:
        """Remove all memos belonging to the given scope.

        Returns a list of the destroyed memo IDs so the caller (Runtime)
        can clean up associated contexts and output signals.

        This is called when a scope is destroyed to clean up its memos.
        """
        var destroyed = List[UInt32]()
        for i in range(len(self._entries)):
            if self._states[i].occupied:
                if self._entries[i].scope_id == scope_id:
                    destroyed.append(UInt32(i))
                    self.destroy(UInt32(i))
        return destroyed^

    fn memos_for_scope(self, scope_id: UInt32) -> List[UInt32]:
        """Return a list of memo IDs belonging to the given scope."""
        var result = List[UInt32]()
        for i in range(len(self._entries)):
            if self._states[i].occupied:
                if self._entries[i].scope_id == scope_id:
                    result.append(UInt32(i))
        return result^

    # ── Value-changed tracking ───────────────────────────────────────

    fn set_value_changed(mut self, id: UInt32, changed: Bool):
        """Set the value_changed flag after end_compute.

        Called by the runtime after comparing old vs new value in
        memo_end_compute_*.  True means the memo's output value
        actually changed; False means it was value-stable.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].value_changed = changed

    fn did_value_change(self, id: UInt32) -> Bool:
        """Check whether the last end_compute changed the memo's value.

        Returns True if the memo's output changed on its most recent
        recomputation, False if it was value-stable (new == old).

        Precondition: `contains(id)` is True.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._states):
            return True  # conservative default
        if not self._states[idx].occupied:
            return True  # conservative default
        return self._entries[idx].value_changed

    fn clear(mut self):
        """Remove all memos."""
        self._entries.clear()
        self._states.clear()
        self._free_head = -1
        self._count = 0
