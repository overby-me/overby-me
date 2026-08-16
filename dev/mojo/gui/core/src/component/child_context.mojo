# ChildComponentContext — Self-rendering child component context.
#
# Wraps a ChildComponent and provides signal/memo creation under the
# child's scope, context consumption from ancestors, context provision
# at the child scope, and a render_builder() for self-rendering.
#
# Does NOT own the AppShell — the parent ComponentContext does.  Holds
# non-owning pointers to the shared Runtime, VNodeStore, and
# ElementIdAllocator.
#
# Usage:
#
#     var child_ctx = parent_ctx.create_child_context(
#         el_p(dyn_text()), String("display"),
#     )
#     var local_state = child_ctx.use_signal(0)
#     var prop = child_ctx.consume_signal_i32(PROP_COUNT)
#
# Signal creation:
#
#     Signals are created directly on the Runtime, targeting the child
#     scope ID.  Unlike ComponentContext.use_signal() which uses the
#     hook system (begin_render/end_render cursor), ChildComponentContext
#     always creates new signals because child components don't re-run
#     their setup.
#
# Ownership:
#
#     ChildComponentContext does NOT own the AppShell or its subsystems.
#     It holds non-owning pointers identical to the reactive handles
#     (SignalI32, etc.).  The parent ComponentContext is responsible for
#     destroying the AppShell.  The child scope, its signals, and its
#     handlers are destroyed via destroy().

from memory import UnsafePointer
from signals import Runtime
from signals.handle import (
    SignalI32,
    SignalBool,
    SignalString,
    MemoI32,
    MemoBool,
    MemoString,
)
from signals.runtime import StringStore
from bridge import MutationWriter
from arena import ElementIdAllocator
from vdom import VNodeStore
from html import VNodeBuilder
from .child import (
    ChildComponent,
    ChildRenderBuilder,
)
from .lifecycle import (
    ConditionalSlot,
    flush_conditional,
    flush_conditional_empty,
)


struct ChildComponentContext(Movable):
    """Context for a self-rendering child component.

    Wraps a ChildComponent and provides signal/memo creation under
    the child's scope, context consumption from ancestors, and a
    render_builder() for self-rendering.

    Does NOT own the AppShell — the parent ComponentContext does.
    Holds non-owning pointers to the shared Runtime, VNodeStore,
    and ElementIdAllocator.
    """

    var child: ChildComponent
    var scope_id: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]
    var store: UnsafePointer[VNodeStore, MutExternalOrigin]
    var eid_alloc: UnsafePointer[ElementIdAllocator, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        var child: ChildComponent,
        scope_id: UInt32,
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
        store: UnsafePointer[VNodeStore, MutExternalOrigin],
        eid_alloc: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    ):
        """Create a ChildComponentContext from an existing ChildComponent.

        Args:
            child: The underlying ChildComponent (moved in).
            scope_id: The child scope ID.
            runtime: Non-owning pointer to the shared Runtime.
            store: Non-owning pointer to the shared VNodeStore.
            eid_alloc: Non-owning pointer to the shared ElementIdAllocator.
        """
        self.scope_id = scope_id
        self.runtime = runtime
        self.store = store
        self.eid_alloc = eid_alloc
        self.child = child^

    fn __moveinit__(out self, deinit other: Self):
        self.child = other.child^
        self.scope_id = other.scope_id
        self.runtime = other.runtime
        self.store = other.store
        self.eid_alloc = other.eid_alloc

    # ── Signal creation (under child scope) ──────────────────────────

    fn use_signal(mut self, initial: Int32) -> SignalI32:
        """Create an Int32 signal under the child scope.

        The signal is created directly on the Runtime (no hook cursor).
        Writing to this signal will mark the child scope dirty, not the
        parent scope.

        Args:
            initial: The initial Int32 value.

        Returns:
            A SignalI32 handle for the child-owned signal.
        """
        var key = self.runtime[0].create_signal[Int32](initial)
        # Subscribe the child scope to this signal so that writes
        # mark the child scope dirty.
        self.runtime[0].signals.subscribe(key, self.scope_id)
        return SignalI32(key, self.runtime)

    fn use_signal_bool(mut self, initial: Bool) -> SignalBool:
        """Create a Bool signal under the child scope.

        Stored as Int32 (1 for True, 0 for False) internally.

        Args:
            initial: The initial Bool value.

        Returns:
            A SignalBool handle for the child-owned signal.
        """
        var init_val: Int32
        if initial:
            init_val = 1
        else:
            init_val = 0
        var key = self.runtime[0].create_signal[Int32](init_val)
        self.runtime[0].signals.subscribe(key, self.scope_id)
        return SignalBool(key, self.runtime)

    fn use_signal_string(mut self, initial: String) -> SignalString:
        """Create a String signal under the child scope.

        Creates both the string entry in the StringStore and a companion
        version signal for reactivity.

        Args:
            initial: The initial String value.

        Returns:
            A SignalString handle for the child-owned signal.
        """
        var keys = self.runtime[0].create_signal_string(initial)
        # Subscribe the child scope to the version signal
        self.runtime[0].signals.subscribe(keys[1], self.scope_id)
        return SignalString(keys[0], keys[1], self.runtime)

    fn use_memo(mut self, initial: Int32) -> MemoI32:
        """Create a memo under the child scope.

        The memo's reactive context is separate from the child scope.
        When memo inputs change, the memo is marked dirty and its
        output signal notifies the child scope.

        Args:
            initial: The initial cached Int32 value.

        Returns:
            A MemoI32 handle.
        """
        var memo_id = self.runtime[0].create_memo_i32(self.scope_id, initial)
        return MemoI32(memo_id, self.runtime)

    fn use_memo_bool(mut self, initial: Bool) -> MemoBool:
        """Create a Bool memo under the child scope.

        The memo's reactive context is separate from the child scope.
        When memo inputs change, the memo is marked dirty and its
        output signal notifies the child scope.

        Args:
            initial: The initial cached Bool value.

        Returns:
            A MemoBool handle.
        """
        var memo_id = self.runtime[0].create_memo_bool(self.scope_id, initial)
        return MemoBool(memo_id, self.runtime)

    fn use_memo_string(mut self, initial: String) -> MemoString:
        """Create a String memo under the child scope.

        The memo's reactive context is separate from the child scope.
        When memo inputs change, the memo is marked dirty and its
        output signal notifies the child scope.

        Args:
            initial: The initial cached String value.

        Returns:
            A MemoString handle.
        """
        var memo_id = self.runtime[0].create_memo_string(self.scope_id, initial)
        return MemoString(memo_id, self.runtime)

    # ── Context consumption ──────────────────────────────────────────

    fn consume_context(self, key: UInt32) -> Tuple[Bool, Int32]:
        """Look up a context value walking up from the child scope.

        Starts at the child scope and walks up the parent chain until
        a matching key is found or the root is reached.

        Args:
            key: The context key to look up.

        Returns:
            A tuple of (found: Bool, value: Int32).
        """
        return self.runtime[0].scopes.consume_context(self.scope_id, key)

    fn has_context(self, key: UInt32) -> Bool:
        """Check whether a context value is reachable from the child scope.

        Args:
            key: The context key to check.

        Returns:
            True if a value for the key is found in the scope tree.
        """
        return self.consume_context(key)[0]

    fn consume_signal_i32(self, key: UInt32) -> SignalI32:
        """Look up a SignalI32 from an ancestor's context.

        Retrieves the signal key stored by `provide_signal_i32()` and
        reconstructs a SignalI32 handle pointing at the same underlying
        signal in the shared Runtime.

        Args:
            key: The context key used in `provide_signal_i32()`.

        Returns:
            A SignalI32 handle for the shared signal.
        """
        var result = self.consume_context(key)
        return SignalI32(UInt32(result[1]), self.runtime)

    fn consume_signal_bool(self, key: UInt32) -> SignalBool:
        """Look up a SignalBool from an ancestor's context.

        Args:
            key: The context key used in `provide_signal_bool()`.

        Returns:
            A SignalBool handle for the shared signal.
        """
        var result = self.consume_context(key)
        return SignalBool(UInt32(result[1]), self.runtime)

    fn consume_signal_string(self, key: UInt32) -> SignalString:
        """Look up a SignalString from an ancestor's context.

        Retrieves both the string key (at `key`) and version key
        (at `key + 1`) stored by `provide_signal_string()`.

        Args:
            key: The context key used in `provide_signal_string()`.

        Returns:
            A SignalString handle for the shared signal.
        """
        var str_result = self.consume_context(key)
        var ver_result = self.consume_context(key + 1)
        return SignalString(
            UInt32(str_result[1]),
            UInt32(ver_result[1]),
            self.runtime,
        )

    # ── Context provision (at child scope) ───────────────────────────

    fn provide_context(mut self, key: UInt32, value: Int32):
        """Provide a context value at the child scope.

        Grandchild scopes (if any) can consume this value by walking
        up the scope tree.

        Args:
            key: A unique UInt32 identifier for the context entry.
            value: The Int32 value to provide.
        """
        self.runtime[0].scopes.provide_context(self.scope_id, key, value)

    fn provide_signal_i32(mut self, key: UInt32, signal: SignalI32):
        """Provide a SignalI32 to descendants via child scope context.

        Args:
            key: A unique UInt32 context key for this signal prop.
            signal: The SignalI32 to share.
        """
        self.provide_context(key, Int32(signal.key))

    fn provide_signal_bool(mut self, key: UInt32, signal: SignalBool):
        """Provide a SignalBool to descendants via child scope context.

        Args:
            key: A unique UInt32 context key for this signal prop.
            signal: The SignalBool to share.
        """
        self.provide_context(key, Int32(signal.key))

    fn provide_signal_string(mut self, key: UInt32, signal: SignalString):
        """Provide a SignalString to descendants via child scope context.

        Stores the string key at `key` and the version key at `key + 1`.

        Args:
            key: A unique UInt32 context key for this signal prop.
            signal: The SignalString to share.
        """
        self.provide_context(key, Int32(signal.string_key))
        self.provide_context(key + 1, Int32(signal.version_key))

    # ── Rendering ────────────────────────────────────────────────────

    fn render_builder(self) -> ChildRenderBuilder:
        """Create a ChildRenderBuilder for this child's template.

        The caller fills in dynamic text values and calls `build()` to
        get the VNode index, which is then passed to `flush()`.

        Returns:
            A ChildRenderBuilder with auto-binding support.
        """
        return self.child.render_builder(self.store, self.runtime)

    # ── Slot / flush delegation ──────────────────────────────────────

    fn init_slot(mut self, anchor_id: UInt32):
        """Initialize the ConditionalSlot after parent mount.

        After the parent mounts, extract the anchor ElementId from the
        mounted VNode's `dyn_node_ids[N]` and pass it here.

        Args:
            anchor_id: ElementId of the placeholder comment node in the
                DOM that occupies the child's dyn_node slot.
        """
        self.child.init_slot(anchor_id)

    fn flush(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        new_vnode_idx: UInt32,
    ):
        """Flush the child: create or diff its VNode in the DOM.

        Delegates to the underlying ChildComponent.flush().

        Does NOT finalize the mutation buffer — the parent is responsible
        for calling finalize() after flushing all children.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            new_vnode_idx: Index of the new child VNode from build().
        """
        self.child.flush(
            writer_ptr,
            self.eid_alloc,
            self.runtime,
            self.store,
            new_vnode_idx,
        )

    fn flush_empty(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ):
        """Hide the child: remove its DOM content, restore placeholder.

        Delegates to the underlying ChildComponent.flush_empty().

        Does NOT finalize the mutation buffer.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
        """
        self.child.flush_empty(
            writer_ptr, self.eid_alloc, self.runtime, self.store
        )

    # ── State queries ────────────────────────────────────────────────

    fn is_dirty(self) -> Bool:
        """Check if the child scope is dirty.

        Checks the Runtime's dirty_scopes queue for this scope ID.
        Signal writes add scope IDs to this queue (they do NOT set
        the scope's own `dirty` flag directly — that flag is set
        during the scheduler's collect phase).

        Returns:
            True if the child scope needs re-rendering.
        """
        for i in range(len(self.runtime[0].dirty_scopes)):
            if self.runtime[0].dirty_scopes[i] == self.scope_id:
                return True
        return False

    fn is_mounted(self) -> Bool:
        """Check whether the child's VNode is currently in the DOM."""
        return self.child.is_mounted()

    fn has_rendered(self) -> Bool:
        """Check whether this child has been rendered at least once."""
        return self.child.has_rendered()

    fn is_slot_initialized(self) -> Bool:
        """Check whether the slot has been initialized with an anchor."""
        return self.child.is_slot_initialized()

    # ── Template / scope accessors ───────────────────────────────────

    fn template_id(self) -> UInt32:
        """Return the child's registered template ID."""
        return self.child.template_id

    fn event_handler_id(self, index: Int) -> UInt32:
        """Return the handler ID for the Nth event in this child.

        Args:
            index: Zero-based index into the event binding list.

        Returns:
            The handler ID at the given index.
        """
        return self.child.event_handler_id(index)

    fn event_count(self) -> Int:
        """Return the number of event bindings on this child."""
        return self.child.event_count()

    fn auto_binding_count(self) -> Int:
        """Return the number of auto-bindings on this child."""
        return self.child.auto_binding_count()

    # ── Error boundary ───────────────────────────────────────────────

    fn use_error_boundary(mut self):
        """Mark this child scope as an error boundary.

        When a descendant scope reports an error via ``report_error()``,
        this scope captures it.  Check ``has_error()`` during flush to
        switch between normal content and fallback UI.

        Example::

            var child_ctx = parent_ctx.create_child_context(view, name)
            child_ctx.use_error_boundary()
        """
        self.runtime[0].scopes.set_error_boundary(self.scope_id, True)

    fn has_error(self) -> Bool:
        """Check whether this child scope (as a boundary) has captured an error.

        Returns:
            True if an error has been propagated to this boundary.
        """
        return self.runtime[0].scopes.has_error(self.scope_id)

    fn error_message(self) -> String:
        """Get the error message captured by this boundary.

        Returns:
            The error message string, or empty if no error.
        """
        return self.runtime[0].scopes.get_error_message(self.scope_id)

    fn clear_error(mut self):
        """Clear the error state on this boundary scope.

        After clearing, the next flush should render normal children
        instead of fallback UI.  Marks the scope dirty so the flush
        cycle processes the state change.
        """
        self.runtime[0].scopes.clear_error(self.scope_id)
        self.runtime[0].mark_scope_dirty(self.scope_id)

    # ── Error reporting ──────────────────────────────────────────────

    fn report_error(self, message: String) -> Int:
        """Propagate an error from this child scope to the nearest boundary.

        First checks whether this child scope itself is a boundary —
        if so, the error is set directly on it.  Otherwise walks up
        the parent chain via ``propagate_error()``.  In both cases
        the boundary scope is marked dirty so the next flush picks
        up the state change.

        Args:
            message: Description of the error.

        Returns:
            The boundary scope ID as Int, or -1 if no boundary found.
        """
        # If this scope is itself a boundary, set the error directly
        # (propagate_error only checks ancestors, not self).
        if self.runtime[0].scopes.is_error_boundary(self.scope_id):
            self.runtime[0].scopes.set_error(self.scope_id, message)
            self.runtime[0].mark_scope_dirty(self.scope_id)
            return Int(self.scope_id)
        var boundary_id = self.runtime[0].scopes.propagate_error(
            self.scope_id, message
        )
        if boundary_id != -1:
            self.runtime[0].mark_scope_dirty(UInt32(boundary_id))
        return boundary_id

    # ── Suspense ─────────────────────────────────────────────────────

    fn use_suspense_boundary(mut self):
        """Mark this child scope as a suspense boundary."""
        self.runtime[0].scopes.set_suspense_boundary(self.scope_id, True)

    fn set_pending(self, pending: Bool):
        """Set the pending (loading) state on this child scope.

        Marks the nearest suspense boundary ancestor dirty.

        Args:
            pending: True to enter pending state, False to resolve.
        """
        self.runtime[0].scopes.set_pending(self.scope_id, pending)
        var boundary_id = self.runtime[0].scopes.find_suspense_boundary(
            self.scope_id
        )
        if boundary_id != -1:
            self.runtime[0].mark_scope_dirty(UInt32(boundary_id))
        elif self.runtime[0].scopes.is_suspense_boundary(self.scope_id):
            self.runtime[0].mark_scope_dirty(self.scope_id)

    fn has_pending(self) -> Bool:
        """Check whether any descendant of this child scope is pending."""
        return self.runtime[0].scopes.has_pending_descendant(self.scope_id)

    fn is_pending(self) -> Bool:
        """Check whether this child scope itself is pending."""
        return self.runtime[0].scopes.is_pending(self.scope_id)

    # ── Destroy ──────────────────────────────────────────────────────

    fn destroy(self):
        """Destroy the child scope, its signals, and handlers.

        Delegates to the underlying ChildComponent.destroy() with the
        shared Runtime pointer.  Does NOT destroy the AppShell — that
        is the parent ComponentContext's responsibility.
        """
        self.child.destroy(self.runtime)
