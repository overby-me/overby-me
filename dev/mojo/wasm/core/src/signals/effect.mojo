# EffectStore — Slab-allocated storage for effect (reactive side effect) entries.
#
# An effect is a side-effectful computation with its own reactive context
# that auto-tracks signal/memo dependencies and is marked "pending" when
# any dependency changes.
#
# Each effect has:
#   - context_id:  A reactive context ID.  When set as the current context
#     during execution, signal reads are auto-subscribed to this context.
#   - scope_id:    The owning scope (for cleanup when the scope is destroyed).
#   - pending:     Whether the effect needs re-execution.
#   - running:     Whether the effect is currently inside a begin/end run
#     bracket (guards against re-entrant execution).
#
# Lifecycle:
#   1. create → allocates context, marked pending (first run needed)
#   2. begin_run → sets effect's context as current reactive context
#   3. (app reads input signals — auto-subscribed to effect's context)
#   4. end_run → clears pending, restores previous context
#   5. (input signal written → effect's context notified → effect marked
#      pending → app checks is_pending and re-runs)
#
# Effects differ from memos:
#   - No output_key / cached value — effects exist purely for side effects
#   - Effects don't propagate dirtiness to subscribers (no output signal)
#   - Effects run AFTER scope re-renders, not during
#
# The store uses a slab allocator (free-list) identical to MemoStore,
# SignalStore, HandlerRegistry, and ScopeArena, so effect IDs are stable
# and reusable.

from memory import UnsafePointer


# ── EffectEntry ──────────────────────────────────────────────────────────────


struct EffectEntry(Copyable, Equatable, Writable):
    """A reactive side effect with dependency tracking.

    An effect has its own reactive context (context_id) that records
    which signals it reads during execution.  Unlike a memo, an effect
    has no cached output value — it exists purely to re-run side-effectful
    code when its dependencies change.
    """

    var context_id: UInt32  # reactive context for dependency tracking
    var scope_id: UInt32  # owning scope (for cleanup)
    var pending: Bool  # needs re-execution
    var running: Bool  # currently inside begin/end run bracket

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create an empty (default) effect entry."""
        self.context_id = 0
        self.scope_id = 0
        self.pending = False
        self.running = False

    fn __init__(out self, context_id: UInt32, scope_id: UInt32):
        """Create an effect entry with the given IDs.

        The effect starts pending (needs first execution) and not running.

        Args:
            context_id: The reactive context ID for dependency tracking.
            scope_id: The owning scope ID.
        """
        self.context_id = context_id
        self.scope_id = scope_id
        self.pending = True
        self.running = False

    fn __copyinit__(out self, other: Self):
        self.context_id = other.context_id
        self.scope_id = other.scope_id
        self.pending = other.pending
        self.running = other.running

    fn __moveinit__(out self, deinit other: Self):
        self.context_id = other.context_id
        self.scope_id = other.scope_id
        self.pending = other.pending
        self.running = other.running


# ── Slot state for the effect store ──────────────────────────────────────────


@fieldwise_init
struct EffectSlotState(Copyable, Equatable, Writable):
    """Tracks whether an effect slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only valid when not occupied; -1 = end of free list.


# ── EffectStore ──────────────────────────────────────────────────────────────


struct EffectStore(Movable):
    """Slab-allocated storage for effect entries.

    Mirrors MemoStore, SignalStore, and HandlerRegistry: uses a free-list
    for stable ID reuse after destruction.

    Usage:
        var store = EffectStore()
        var id = store.create(context_id=10, scope_id=0)
        assert store.is_pending(id)       # starts pending
        store.clear_pending(id)
        assert not store.is_pending(id)
        store.mark_pending(id)
        assert store.is_pending(id)
        store.destroy(id)
    """

    var _entries: List[EffectEntry]
    var _states: List[EffectSlotState]
    var _free_head: Int
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        self._entries = List[EffectEntry]()
        self._states = List[EffectSlotState]()
        self._free_head = -1
        self._count = 0

    fn __moveinit__(out self, deinit other: Self):
        self._entries = other._entries^
        self._states = other._states^
        self._free_head = other._free_head
        self._count = other._count

    # ── Create / Destroy ─────────────────────────────────────────────

    fn create(mut self, context_id: UInt32, scope_id: UInt32) -> UInt32:
        """Create a new effect entry.  Returns its stable ID.

        The effect starts pending (needs first execution).

        Args:
            context_id: The reactive context ID for dependency tracking.
            scope_id: The owning scope ID.

        Returns:
            The UInt32 effect ID.
        """
        var entry = EffectEntry(context_id, scope_id)

        if self._free_head != -1:
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._entries[idx] = entry^
            self._states[idx] = EffectSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            var idx = len(self._entries)
            self._entries.append(entry^)
            self._states.append(EffectSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    fn destroy(mut self, id: UInt32):
        """Remove the effect at `id`, freeing its slot for reuse.

        Destroying a non-existent or already-freed effect is a no-op.
        Does NOT clean up the underlying context signal — the caller
        (Runtime) is responsible for that.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx] = EffectEntry()
        self._states[idx] = EffectSlotState(
            occupied=False, next_free=self._free_head
        )
        self._free_head = idx
        self._count -= 1

    # ── Access ───────────────────────────────────────────────────────

    fn get(self, id: UInt32) -> EffectEntry:
        """Return a copy of the effect entry at `id`.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].copy()

    fn get_ptr(
        mut self, id: UInt32
    ) -> UnsafePointer[EffectEntry, MutExternalOrigin]:
        """Return a pointer to the effect entry at `id`.

        The pointer is valid until the next mutation of the store.
        Precondition: `contains(id)` is True.
        """
        return UnsafePointer.address_of(self._entries[Int(id)])

    # ── Pending tracking ─────────────────────────────────────────────

    fn mark_pending(mut self, id: UInt32):
        """Mark the effect as needing re-execution.

        Called when an input signal that the effect depends on is written.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].pending = True

    fn clear_pending(mut self, id: UInt32):
        """Clear the pending flag (called after successful execution)."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].pending = False

    fn is_pending(self, id: UInt32) -> Bool:
        """Check whether the effect needs re-execution.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].pending

    # ── Running state ────────────────────────────────────────────────

    fn set_running(mut self, id: UInt32, running: Bool):
        """Set the running flag on the effect.

        True while the effect is inside a begin_run / end_run bracket.
        """
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx].running = running

    fn is_running(self, id: UInt32) -> Bool:
        """Check whether the effect is currently executing.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].running

    # ── Field accessors ──────────────────────────────────────────────

    fn context_id(self, id: UInt32) -> UInt32:
        """Return the reactive context ID of the effect.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].context_id

    fn scope_id(self, id: UInt32) -> UInt32:
        """Return the owning scope ID of the effect.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].scope_id

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of live effects."""
        return self._count

    fn contains(self, id: UInt32) -> Bool:
        """Check whether `id` is a live effect."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._states):
            return False
        return self._states[idx].occupied

    # ── Bulk operations ──────────────────────────────────────────────

    fn pending_effects(self) -> List[UInt32]:
        """Return a list of all effect IDs that are currently pending.

        The caller is responsible for running each pending effect via
        begin_run / end_run.  This method does NOT clear pending flags.
        """
        var result = List[UInt32]()
        for i in range(len(self._entries)):
            if self._states[i].occupied and self._entries[i].pending:
                result.append(UInt32(i))
        return result^

    fn remove_for_scope(mut self, scope_id: UInt32) -> List[UInt32]:
        """Remove all effects belonging to the given scope.

        Returns a list of the destroyed effect IDs so the caller (Runtime)
        can clean up associated context signals.

        This is called when a scope is destroyed to clean up its effects.
        """
        var destroyed = List[UInt32]()
        for i in range(len(self._entries)):
            if self._states[i].occupied:
                if self._entries[i].scope_id == scope_id:
                    destroyed.append(UInt32(i))
                    self.destroy(UInt32(i))
        return destroyed^

    fn effects_for_scope(self, scope_id: UInt32) -> List[UInt32]:
        """Return a list of effect IDs belonging to the given scope."""
        var result = List[UInt32]()
        for i in range(len(self._entries)):
            if self._states[i].occupied:
                if self._entries[i].scope_id == scope_id:
                    result.append(UInt32(i))
        return result^

    fn clear(mut self):
        """Remove all effects."""
        self._entries.clear()
        self._states.clear()
        self._free_head = -1
        self._count = 0
