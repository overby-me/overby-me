# AppShell — Common runtime infrastructure for UI applications.
#
# Bundles the subsystems that every app needs:
#   - Runtime (signals, scopes, handlers, templates)
#   - VNodeStore (virtual DOM node storage)
#   - ElementIdAllocator (unique DOM element IDs)
#   - Scheduler (height-ordered dirty scope processing)
#
# Provides lifecycle helpers:
#   - mount(): initial render via CreateEngine → append to root
#   - update(): diff old/new VNodes via DiffEngine
#   - flush(): collect dirty scopes, yield them for re-rendering
#   - destroy(): free all resources
#
# Apps compose an AppShell instead of manually wiring subsystems:
#
#     struct MyApp:
#         var shell: AppShell
#         var my_signal: UInt32
#         ...
#
#         fn init(mut self):
#             self.shell = app_shell_create()
#             self.my_signal = self.shell.create_signal_i32(42)
#             ...

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from arena import ElementIdAllocator
from signals import Runtime, create_runtime, destroy_runtime
from mutations import CreateEngine, DiffEngine
from vdom import VNodeStore, VNode
from scheduler import Scheduler
from .lifecycle import FragmentSlot, flush_fragment as _flush_fragment_raw


struct AppShell(Movable):
    """Common runtime infrastructure shared by all applications.

    Owns the reactive runtime, VNode store, element ID allocator,
    and scheduler.  Provides convenience methods for the standard
    mount/update/flush lifecycle so individual apps don't need to
    manually wire CreateEngine, DiffEngine, etc.
    """

    var runtime: UnsafePointer[Runtime, MutExternalOrigin]
    var store: UnsafePointer[VNodeStore, MutExternalOrigin]
    var eid_alloc: UnsafePointer[ElementIdAllocator, MutExternalOrigin]
    var scheduler: Scheduler
    var _alive: Bool

    fn __init__(out self):
        """Create an uninitialized shell.  Call `setup()` to allocate."""
        self.runtime = UnsafePointer[Runtime, MutExternalOrigin]()
        self.store = UnsafePointer[VNodeStore, MutExternalOrigin]()
        self.eid_alloc = UnsafePointer[ElementIdAllocator, MutExternalOrigin]()
        self.scheduler = Scheduler()
        self._alive = False

    fn __moveinit__(out self, deinit other: Self):
        self.runtime = other.runtime
        self.store = other.store
        self.eid_alloc = other.eid_alloc
        self.scheduler = other.scheduler^
        self._alive = other._alive

    # ── Setup / Teardown ─────────────────────────────────────────────

    fn setup(mut self):
        """Allocate all subsystems.  Must be called once before use."""
        self.runtime = create_runtime()

        self.store = alloc[VNodeStore](1)
        self.store.init_pointee_move(VNodeStore())

        self.eid_alloc = alloc[ElementIdAllocator](1)
        self.eid_alloc.init_pointee_move(ElementIdAllocator())

        self.scheduler = Scheduler()
        self._alive = True

    fn destroy(mut self):
        """Free all resources.  Safe to call multiple times."""
        if not self._alive:
            return
        self._alive = False

        if self.store != UnsafePointer[VNodeStore, MutExternalOrigin]():
            self.store.destroy_pointee()
            self.store.free()
            self.store = UnsafePointer[VNodeStore, MutExternalOrigin]()

        if (
            self.eid_alloc
            != UnsafePointer[ElementIdAllocator, MutExternalOrigin]()
        ):
            self.eid_alloc.destroy_pointee()
            self.eid_alloc.free()
            self.eid_alloc = UnsafePointer[
                ElementIdAllocator, MutExternalOrigin
            ]()

        if self.runtime != UnsafePointer[Runtime, MutExternalOrigin]():
            destroy_runtime(self.runtime)
            self.runtime = UnsafePointer[Runtime, MutExternalOrigin]()

    fn is_alive(self) -> Bool:
        """Check whether the shell has been set up and not yet destroyed."""
        return self._alive

    # ── Scope helpers ────────────────────────────────────────────────

    fn create_root_scope(mut self) -> UInt32:
        """Create a root scope (height 0, no parent).  Returns scope ID."""
        return self.runtime[0].create_scope(0, -1)

    fn create_child_scope(mut self, parent_id: UInt32) -> UInt32:
        """Create a child scope.  Returns scope ID."""
        return self.runtime[0].create_child_scope(parent_id)

    fn destroy_child_scopes(mut self, scope_ids: List[UInt32]):
        """Destroy a list of child scopes and clean up their handlers.

        For each scope ID in the list:
          1. Remove all event handlers registered under that scope
             (via HandlerRegistry.remove_for_scope).
          2. Destroy the scope itself (via ScopeArena.destroy).

        This is the correct cleanup sequence for ephemeral per-item or
        per-row scopes whose handlers are re-registered on every rebuild.
        Permanent handlers (registered under the parent scope) are
        unaffected.

        Args:
            scope_ids: List of child scope IDs to destroy.
        """
        for i in range(len(scope_ids)):
            var sid = scope_ids[i]
            self.runtime[0].handlers.remove_for_scope(sid)
            self.runtime[0].destroy_scope(sid)

    fn begin_render(mut self, scope_id: UInt32) -> Int:
        """Begin rendering a scope.  Returns previous scope ID (or -1)."""
        return self.runtime[0].begin_scope_render(scope_id)

    fn end_render(mut self, prev_scope: Int):
        """End rendering and restore the previous scope."""
        self.runtime[0].end_scope_render(prev_scope)

    # ── Signal helpers ───────────────────────────────────────────────

    fn create_signal_i32(mut self, initial: Int32) -> UInt32:
        """Create an Int32 signal.  Returns its key."""
        return self.runtime[0].create_signal[Int32](initial)

    fn read_signal_i32(mut self, key: UInt32) -> Int32:
        """Read an Int32 signal (with context tracking)."""
        return self.runtime[0].read_signal[Int32](key)

    fn peek_signal_i32(self, key: UInt32) -> Int32:
        """Read an Int32 signal without subscribing."""
        return self.runtime[0].peek_signal[Int32](key)

    fn write_signal_i32(mut self, key: UInt32, value: Int32):
        """Write a new value to an Int32 signal."""
        self.runtime[0].write_signal[Int32](key, value)

    fn use_signal_i32(mut self, initial: Int32) -> UInt32:
        """Hook: create or retrieve an Int32 signal for the current scope."""
        return self.runtime[0].use_signal_i32(initial)

    # ── Memo helpers ─────────────────────────────────────────────────

    fn create_memo_i32(mut self, scope_id: UInt32, initial: Int32) -> UInt32:
        """Create an Int32 memo.  Returns its ID."""
        return self.runtime[0].create_memo_i32(scope_id, initial)

    fn memo_begin_compute(mut self, memo_id: UInt32):
        """Begin memo computation (sets memo's context as current)."""
        self.runtime[0].memo_begin_compute(memo_id)

    fn memo_end_compute_i32(mut self, memo_id: UInt32, value: Int32):
        """End memo computation and cache the result."""
        self.runtime[0].memo_end_compute_i32(memo_id, value)

    fn memo_read_i32(mut self, memo_id: UInt32) -> Int32:
        """Read a memo's cached value (with context tracking)."""
        return self.runtime[0].memo_read_i32(memo_id)

    fn memo_is_dirty(self, memo_id: UInt32) -> Bool:
        """Check whether the memo needs recomputation."""
        return self.runtime[0].memo_is_dirty(memo_id)

    fn use_memo_i32(mut self, initial: Int32) -> UInt32:
        """Hook: create or retrieve an Int32 memo for the current scope."""
        return self.runtime[0].use_memo_i32(initial)

    # ── MemoBool helpers ─────────────────────────────────────────────

    fn create_memo_bool(mut self, scope_id: UInt32, initial: Bool) -> UInt32:
        """Create a Bool memo.  Returns its ID."""
        return self.runtime[0].create_memo_bool(scope_id, initial)

    fn memo_end_compute_bool(mut self, memo_id: UInt32, value: Bool):
        """End Bool memo computation and cache the result."""
        self.runtime[0].memo_end_compute_bool(memo_id, value)

    fn memo_read_bool(mut self, memo_id: UInt32) -> Bool:
        """Read a Bool memo's cached value (with context tracking)."""
        return self.runtime[0].memo_read_bool(memo_id)

    fn use_memo_bool(mut self, initial: Bool) -> UInt32:
        """Hook: create or retrieve a Bool memo for the current scope."""
        return self.runtime[0].use_memo_bool(initial)

    # ── MemoString helpers ───────────────────────────────────────────

    fn create_memo_string(
        mut self, scope_id: UInt32, initial: String
    ) -> UInt32:
        """Create a String memo.  Returns its ID."""
        return self.runtime[0].create_memo_string(scope_id, initial)

    fn memo_end_compute_string(mut self, memo_id: UInt32, value: String):
        """End String memo computation and cache the result."""
        self.runtime[0].memo_end_compute_string(memo_id, value)

    fn memo_read_string(mut self, memo_id: UInt32) -> String:
        """Read a String memo's cached value (with context tracking)."""
        return self.runtime[0].memo_read_string(memo_id)

    fn memo_peek_string(self, memo_id: UInt32) -> String:
        """Read a String memo's cached value without subscribing."""
        return self.runtime[0].memo_peek_string(memo_id)

    fn use_memo_string(mut self, initial: String) -> UInt32:
        """Hook: create or retrieve a String memo for the current scope."""
        return self.runtime[0].use_memo_string(initial)

    # ── Effect helpers ───────────────────────────────────────────────

    fn create_effect(mut self, scope_id: UInt32) -> UInt32:
        """Create an effect with a reactive context.  Returns its ID."""
        return self.runtime[0].create_effect(scope_id)

    fn effect_begin_run(mut self, effect_id: UInt32):
        """Begin effect execution (sets effect's context as current)."""
        self.runtime[0].effect_begin_run(effect_id)

    fn effect_end_run(mut self, effect_id: UInt32):
        """End effect execution (clears pending, restores context)."""
        self.runtime[0].effect_end_run(effect_id)

    fn effect_is_pending(self, effect_id: UInt32) -> Bool:
        """Check whether the effect needs re-execution."""
        return self.runtime[0].effect_is_pending(effect_id)

    fn use_effect(mut self) -> UInt32:
        """Hook: create or retrieve an effect for the current scope."""
        return self.runtime[0].use_effect()

    fn drain_pending_effects(self) -> List[UInt32]:
        """Return a list of effect IDs that are currently pending.

        Does NOT clear pending flags — the caller must begin_run/end_run
        each effect to clear them.
        """
        return self.runtime[0].drain_pending_effects()

    fn pending_effect_count(self) -> Int:
        """Return the number of pending effects."""
        return self.runtime[0].pending_effect_count()

    fn pending_effect_at(self, index: Int) -> UInt32:
        """Return the effect ID at the given index in the pending list."""
        return self.runtime[0].pending_effect_at(index)

    # ── Mount lifecycle ──────────────────────────────────────────────

    fn mount(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        vnode_idx: UInt32,
    ) -> Int32:
        """Initial render: create mutations for a VNode, append to root (id 0),
        and finalize the mutation buffer.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            vnode_idx: Index of the VNode to mount in the store.

        Returns:
            Byte length of the mutation data written.
        """
        var engine = CreateEngine(
            writer_ptr, self.eid_alloc, self.runtime, self.store
        )
        var num_roots = engine.create_node(vnode_idx)

        # Append to root element (id 0)
        writer_ptr[0].append_children(0, num_roots)

        # Finalize
        writer_ptr[0].finalize()
        return Int32(writer_ptr[0].offset)

    fn mount_with_templates(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        vnode_idx: UInt32,
    ) -> Int32:
        """Initial render with template emission prepended.

        Emits `RegisterTemplate` mutations for all registered templates,
        then creates the VNode tree, appends to root, and finalizes.
        The JS interpreter can build DOM templates and instantiate them
        in a single `applyMutations()` pass.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            vnode_idx: Index of the VNode to mount in the store.

        Returns:
            Byte length of the mutation data written.
        """
        # 1. Emit all registered templates
        self.emit_templates(writer_ptr)

        # 2. Create + mount (without finalize — we do it once at the end)
        var engine = CreateEngine(
            writer_ptr, self.eid_alloc, self.runtime, self.store
        )
        var num_roots = engine.create_node(vnode_idx)

        # 3. Append to root element (id 0)
        writer_ptr[0].append_children(0, num_roots)

        # 4. Finalize
        writer_ptr[0].finalize()
        return Int32(writer_ptr[0].offset)

    # ── Update lifecycle ─────────────────────────────────────────────

    fn diff(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        old_idx: UInt32,
        new_idx: UInt32,
    ):
        """Diff two VNodes and emit mutations.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            old_idx: Index of the old (current) VNode in the store.
            new_idx: Index of the new (updated) VNode in the store.
        """
        var engine = DiffEngine(
            writer_ptr, self.eid_alloc, self.runtime, self.store
        )
        engine.diff_node(old_idx, new_idx)

    fn finalize(
        self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]
    ) -> Int32:
        """Write the End sentinel and return the byte length."""
        writer_ptr[0].finalize()
        return Int32(writer_ptr[0].offset)

    # ── Flush lifecycle ──────────────────────────────────────────────

    fn has_dirty(self) -> Bool:
        """Check whether any scopes need re-rendering."""
        return self.runtime[0].has_dirty()

    fn collect_dirty(mut self):
        """Drain the runtime's dirty queue into the scheduler."""
        self.scheduler.collect(self.runtime)

    fn next_dirty(mut self) -> UInt32:
        """Return the next scope to render (lowest height first).

        Precondition: scheduler is not empty (call collect_dirty first).
        """
        return self.scheduler.next()

    fn scheduler_empty(self) -> Bool:
        """Check if the scheduler has no more pending scopes."""
        return self.scheduler.is_empty()

    fn consume_dirty(mut self) -> Bool:
        """Collect all dirty scopes via the scheduler and consume them.

        Routes dirty scope processing through the height-ordered
        scheduler instead of raw `runtime.drain_dirty()`.  This
        ensures correct render order when multiple scopes are dirty
        (parent before child) and deduplicates scope IDs.

        For single-scope apps this is equivalent to a simple drain,
        but correctly prepares for multi-scope support.

        Returns:
            True if any scopes were dirty, False otherwise.
        """
        if not self.has_dirty():
            return False
        self.collect_dirty()
        while not self.scheduler_empty():
            _ = self.next_dirty()
        return True

    fn settle_scopes(mut self):
        """Remove dirty scopes with no actual signal changes.

        After memo recomputation, some scopes may have been eagerly
        marked dirty (Phase 36) but none of their subscribed signals
        actually changed value.  This method checks each dirty scope
        and removes those where no subscribed signal changed.

        Call after run_memos() and before render() to skip unnecessary
        re-renders.
        """
        self.runtime[0].settle_scopes()

    # ── Batch signal writes (Phase 38) ───────────────────────────────

    fn begin_batch(mut self):
        """Enter batch mode for signal writes."""
        self.runtime[0].begin_batch()

    fn end_batch(mut self):
        """Exit batch mode and propagate all deferred writes."""
        self.runtime[0].end_batch()

    fn is_batching(self) -> Bool:
        """Return True if currently inside a batch."""
        return self.runtime[0].is_batching()

    # ── Fragment flush ───────────────────────────────────────────────

    fn flush_fragment(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        slot: FragmentSlot,
        new_frag_idx: UInt32,
    ) -> FragmentSlot:
        """Flush a fragment slot using the shell's own subsystem pointers.

        Convenience wrapper around the lifecycle `flush_fragment()` helper
        that avoids passing `eid_alloc`, `runtime`, and `store` pointers
        individually.  See `lifecycle.flush_fragment()` for full docs.

        Handles three transitions:
          1. empty → populated (CreateEngine + ReplaceWith anchor)
          2. populated → populated (DiffEngine keyed diff)
          3. populated → empty (new anchor + remove old items)

        Does NOT finalize — the caller must call `writer_ptr[0].finalize()`
        or `self.finalize(writer_ptr)` after this returns.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            slot: Current FragmentSlot state.
            new_frag_idx: Index of the new Fragment VNode in the store.

        Returns:
            Updated FragmentSlot with new state.
        """
        var mut_slot = slot.copy()
        return _flush_fragment_raw(
            writer_ptr,
            self.eid_alloc,
            self.runtime,
            self.store,
            mut_slot,
            new_frag_idx,
        )

    # ── Template emission ────────────────────────────────────────────

    fn emit_templates(
        self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]
    ):
        """Serialize all registered templates into the mutation buffer.

        Iterates the template registry and emits a `RegisterTemplate`
        mutation for each template.  Call this before `CreateEngine`
        so that the JS interpreter can build DOM templates from the
        same mutation buffer pass.
        """
        var count = self.runtime[0].templates.count()
        for i in range(count):
            var tmpl_ptr = self.runtime[0].templates.get_ptr(UInt32(i))
            writer_ptr[0].register_template(tmpl_ptr[0])

    # ── Event dispatch ───────────────────────────────────────────────

    fn dispatch_event(mut self, handler_id: UInt32, event_type: UInt8) -> Bool:
        """Dispatch an event to a handler.  Returns True if executed."""
        return self.runtime[0].dispatch_event(handler_id, event_type)

    fn dispatch_event_with_i32(
        mut self, handler_id: UInt32, event_type: UInt8, value: Int32
    ) -> Bool:
        """Dispatch an event with an Int32 payload."""
        return self.runtime[0].dispatch_event_with_i32(
            handler_id, event_type, value
        )

    fn dispatch_event_with_string(
        mut self, handler_id: UInt32, event_type: UInt8, value: String
    ) -> Bool:
        """Dispatch an event with a String payload (Phase 20).

        For ACTION_SIGNAL_SET_STRING handlers, writes the string value
        to the target SignalString.  Falls back to normal dispatch for
        other action types.
        """
        return self.runtime[0].dispatch_event_with_string(
            handler_id, event_type, value
        )


# ── Module-level factory ─────────────────────────────────────────────────────


fn app_shell_create() -> AppShell:
    """Create and set up a new AppShell with all subsystems allocated."""
    var shell = AppShell()
    shell.setup()
    return shell^
