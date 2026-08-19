# ChildComponent — Composable child component handle for parent templates.
#
# A ChildComponent wraps a child scope + template + auto-bindings so that
# a parent component can embed a child's VNode in a `dyn_node()` slot.
# The diff engine handles same-template VNodes with incremental diffing,
# so when only the child's dynamic text changes, only SetText mutations
# are emitted — the parent's unchanged slots produce zero mutations.
#
# DOM lifecycle is managed via an embedded ConditionalSlot.  The parent
# template has a `dyn_node(N)` slot and calls `add_dyn_placeholder()`
# during render.  After mount, the anchor ElementId is extracted from
# `dyn_node_ids[N]` and passed to `init_slot()`.  On each flush, the
# child builds its VNode and calls `flush()` which delegates to
# `flush_conditional()` to create/diff the child content in the DOM.
#
# Lifecycle:
#
#   # Parent setup:
#   var child = ctx.create_child_component(
#       el_p(dyn_text()),
#       String("display"),
#   )
#
#   # Parent render (always emits placeholder for the child slot):
#   var pvb = ctx.render_builder()
#   pvb.add_dyn_placeholder()    # dyn_node[N] — child lives here
#   var parent_idx = pvb.build()
#
#   # After mount — extract anchor:
#   var anchor = vnode_ptr[0].get_dyn_node_id(N)
#   child.init_slot(anchor)
#
#   # Parent flush — build + flush child:
#   var cvb = child.render_builder(ctx.store_ptr(), ctx.runtime_ptr())
#   cvb.add_dyn_text("Count: " + str(count.peek()))
#   var child_idx = cvb.build()
#   child.flush(writer_ptr, ctx.shell.eid_alloc, ctx.runtime_ptr(),
#               ctx.store_ptr(), child_idx)
#
#   # Parent destroy:
#   child.destroy(ctx.runtime_ptr())
#
# Scope hierarchy:
#   The child gets its own ScopeState (via create_child_scope) so that
#   its event handlers can be cleaned up independently.  Signals can be
#   shared between parent and child — the child reads the parent's signal
#   and the diff engine handles the rest.
#
# This is analogous to ConditionalSlot (manages conditional DOM content)
# and FragmentSlot (manages keyed lists), but for reusable sub-components
# with their own template and scope.

from std.memory import UnsafePointer
from bridge import MutationWriter
from arena import ElementIdAllocator
from signals import Runtime
from signals.handle import SignalI32, SignalBool, SignalString
from signals.runtime import StringStore
from events import HandlerEntry
from vdom import (
    VNode,
    VNodeStore,
)
from html import (
    Node,
    NODE_EVENT,
    NODE_ELEMENT,
    NODE_DYN_TEXT,
    NODE_DYN_ATTR,
    NODE_BIND_VALUE,
    DYN_TEXT_AUTO,
    to_template,
    VNodeBuilder,
)
from .lifecycle import (
    ConditionalSlot,
    flush_conditional,
    flush_conditional_empty,
)


# Forward-declare types used from context.mojo to avoid circular imports.
# ChildComponent stores its own copies of the binding lists rather than
# referencing the parent context.


struct _ChildEventInfo(Copyable, Movable):
    """Event handler info collected from a child's view tree."""

    var event_name: String
    var action: UInt8
    var signal_key: UInt32
    var operand: Int32

    def __init__(
        out self,
        event_name: String,
        action: UInt8,
        signal_key: UInt32,
        operand: Int32,
    ):
        self.event_name = event_name
        self.action = action
        self.signal_key = signal_key
        self.operand = operand

    def __init__(out self, *, copy: Self):
        self.event_name = copy.event_name
        self.action = copy.action
        self.signal_key = copy.signal_key
        self.operand = copy.operand

    def __init__(out self, *, deinit move: Self):
        self.event_name = move.event_name^
        self.action = move.action
        self.signal_key = move.signal_key
        self.operand = move.operand


# ══════════════════════════════════════════════════════════════════════════════
# ChildEventBinding — Stored event handler for child component rendering
# ══════════════════════════════════════════════════════════════════════════════


struct ChildEventBinding(Copyable, Movable):
    """A registered event handler binding for a child component.

    Mirrors EventBinding from context.mojo but is stored on the
    ChildComponent itself rather than the parent context.
    """

    var event_name: String
    var handler_id: UInt32

    def __init__(out self, event_name: String, handler_id: UInt32):
        self.event_name = event_name
        self.handler_id = handler_id

    def __init__(out self, *, copy: Self):
        self.event_name = copy.event_name
        self.handler_id = copy.handler_id

    def __init__(out self, *, deinit move: Self):
        self.event_name = move.event_name^
        self.handler_id = move.handler_id


# ══════════════════════════════════════════════════════════════════════════════
# ChildAutoBinding — Tagged union for child component auto-bindings
# ══════════════════════════════════════════════════════════════════════════════

comptime CHILD_BIND_EVENT: UInt8 = 0
comptime CHILD_BIND_VALUE: UInt8 = 1


struct ChildAutoBinding(Copyable, Movable):
    """Auto-populated dynamic attribute for child component rendering.

    Mirrors AutoBinding from context.mojo but is stored on the
    ChildComponent itself.
    """

    var kind: UInt8
    # Event fields
    var event_name: String
    var handler_id: UInt32
    # Value binding fields
    var attr_name: String
    var string_key: UInt32
    var version_key: UInt32

    @staticmethod
    def event(event_name: String, handler_id: UInt32) -> Self:
        """Create an event auto-binding."""
        return Self(
            kind=CHILD_BIND_EVENT,
            event_name=event_name,
            handler_id=handler_id,
            attr_name=String(""),
            string_key=0,
            version_key=0,
        )

    @staticmethod
    def value(
        attr_name: String, string_key: UInt32, version_key: UInt32
    ) -> Self:
        """Create a value binding auto-binding."""
        return Self(
            kind=CHILD_BIND_VALUE,
            event_name=String(""),
            handler_id=0,
            attr_name=attr_name,
            string_key=string_key,
            version_key=version_key,
        )

    def __init__(
        out self,
        kind: UInt8,
        event_name: String,
        handler_id: UInt32,
        attr_name: String,
        string_key: UInt32,
        version_key: UInt32,
    ):
        self.kind = kind
        self.event_name = event_name
        self.handler_id = handler_id
        self.attr_name = attr_name
        self.string_key = string_key
        self.version_key = version_key

    def __init__(out self, *, copy: Self):
        self.kind = copy.kind
        self.event_name = copy.event_name
        self.handler_id = copy.handler_id
        self.attr_name = copy.attr_name
        self.string_key = copy.string_key
        self.version_key = copy.version_key

    def __init__(out self, *, deinit move: Self):
        self.kind = move.kind
        self.event_name = move.event_name^
        self.handler_id = move.handler_id
        self.attr_name = move.attr_name^
        self.string_key = move.string_key
        self.version_key = move.version_key

    def is_event(self) -> Bool:
        """Check whether this is an event binding."""
        return self.kind == CHILD_BIND_EVENT

    def is_value(self) -> Bool:
        """Check whether this is a value binding."""
        return self.kind == CHILD_BIND_VALUE


# ══════════════════════════════════════════════════════════════════════════════
# ChildRenderBuilder — VNodeBuilder wrapper with child auto-bindings
# ══════════════════════════════════════════════════════════════════════════════


struct ChildRenderBuilder(Movable):
    """Ergonomic VNode builder for child components.

    Created by `ChildComponent.render_builder()`.  Works identically to
    the parent's RenderBuilder — the component author adds dynamic text
    and the event handlers are auto-populated on `build()`.

    Usage:
        var vb = child.render_builder(ctx.store_ptr(), ctx.runtime_ptr())
        vb.add_dyn_text("Count: " + str(count.peek()))
        var idx = vb.build()
    """

    var _vb: VNodeBuilder
    var _auto_bindings: List[ChildAutoBinding]
    var _events: List[ChildEventBinding]
    var _runtime: UnsafePointer[Runtime, MutUntrackedOrigin]

    def __init__(
        out self,
        var vb: VNodeBuilder,
        var auto_bindings: List[ChildAutoBinding],
        runtime: UnsafePointer[Runtime, MutUntrackedOrigin],
    ):
        """Construct with auto-bindings (events + value bindings)."""
        self._vb = vb^
        self._auto_bindings = auto_bindings^
        self._events = List[ChildEventBinding]()
        self._runtime = runtime

    def __init__(
        out self,
        var vb: VNodeBuilder,
        var events: List[ChildEventBinding],
    ):
        """Legacy constructor with event-only bindings."""
        self._vb = vb^
        self._auto_bindings = List[ChildAutoBinding]()
        self._events = events^
        self._runtime = UnsafePointer[
            Runtime, MutUntrackedOrigin
        ].unsafe_dangling()

    def __init__(out self, *, deinit move: Self):
        self._vb = move._vb^
        self._auto_bindings = move._auto_bindings^
        self._events = move._events^
        self._runtime = move._runtime

    # ── Dynamic text ─────────────────────────────────────────────────

    def add_dyn_text(mut self, value: String):
        """Add a dynamic text node (fills the next DynamicText slot)."""
        self._vb.add_dyn_text(value)

    def add_dyn_placeholder(mut self):
        """Add a dynamic placeholder node."""
        self._vb.add_dyn_placeholder()

    def add_dyn_text_signal(mut self, signal: SignalString):
        """Add a dynamic text node from a SignalString value."""
        self._vb.add_dyn_text(signal.get())

    # ── Dynamic attributes (manual) ─────────────────────────────────

    def add_dyn_text_attr(mut self, name: String, value: String):
        """Add a dynamic text attribute (e.g. class, id, href)."""
        self._vb.add_dyn_text_attr(name, value)

    def add_dyn_bool_attr(mut self, name: String, value: Bool):
        """Add a dynamic boolean attribute (e.g. disabled, checked)."""
        self._vb.add_dyn_bool_attr(name, value)

    # ── Conditional class helpers ────────────────────────────────────

    def add_class_if(mut self, condition: Bool, class_name: String):
        """Add a conditional CSS class attribute."""
        if condition:
            self._vb.add_dyn_text_attr(String("class"), class_name)
        else:
            self._vb.add_dyn_text_attr(String("class"), String(""))

    def add_class_when(
        mut self,
        condition: Bool,
        true_class: String,
        false_class: String,
    ):
        """Add one of two CSS class names based on a condition."""
        if condition:
            self._vb.add_dyn_text_attr(String("class"), true_class)
        else:
            self._vb.add_dyn_text_attr(String("class"), false_class)

    # ── Build ────────────────────────────────────────────────────────

    def build(mut self) -> UInt32:
        """Finalize the VNode by auto-adding all registered bindings.

        Adds dynamic attributes for each ChildAutoBinding in tree-walk
        order, then returns the VNode index.

        Returns:
            The VNode's index in the VNodeStore.
        """
        if len(self._auto_bindings) > 0:
            for i in range(len(self._auto_bindings)):
                if self._auto_bindings[i].is_event():
                    self._vb.add_dyn_event(
                        self._auto_bindings[i].event_name,
                        self._auto_bindings[i].handler_id,
                    )
                elif self._auto_bindings[i].is_value():
                    var value = self._runtime[0].peek_signal_string(
                        self._auto_bindings[i].string_key
                    )
                    self._vb.add_dyn_text_attr(
                        self._auto_bindings[i].attr_name, value
                    )
        else:
            # Legacy path: event-only
            for i in range(len(self._events)):
                self._vb.add_dyn_event(
                    self._events[i].event_name,
                    self._events[i].handler_id,
                )
        return self._vb.index()


# ══════════════════════════════════════════════════════════════════════════════
# ChildComponent — Composable child component handle
# ══════════════════════════════════════════════════════════════════════════════


struct ChildComponent(Copyable, Movable):
    """A composable child component that plugs into a parent's dyn_node slot.

    Wraps a child scope ID, template ID, current VNode index, a
    ConditionalSlot for DOM lifecycle, and auto-bindings.  Created via
    `ComponentContext.create_child_component()`.

    The parent template has a `dyn_node(N)` slot and always renders a
    placeholder for it.  After mount, the anchor ElementId is extracted
    from `dyn_node_ids[N]` and passed to `init_slot()`.  On each flush,
    the child builds its VNode and calls `flush()` which creates/diffs
    the child content in the DOM via `flush_conditional()`.

    Fields:
        scope_id: The child's scope ID (for handler registration and cleanup).
        template_id: The child's registered template ID.
        current_vnode: Index of the most recently rendered VNode (-1 before
            first render).
        slot: ConditionalSlot managing the child's DOM presence.
        _event_bindings: Event handler bindings registered under the child scope.
        _auto_bindings: Combined event + value bindings for render_builder.
    """

    var scope_id: UInt32
    var template_id: UInt32
    var current_vnode: Int
    var slot: ConditionalSlot
    var _event_bindings: List[ChildEventBinding]
    var _auto_bindings: List[ChildAutoBinding]

    def __init__(out self):
        """Create an uninitialized ChildComponent."""
        self.scope_id = 0
        self.template_id = 0
        self.current_vnode = -1
        self.slot = ConditionalSlot()
        self._event_bindings = List[ChildEventBinding]()
        self._auto_bindings = List[ChildAutoBinding]()

    def __init__(
        out self,
        scope_id: UInt32,
        template_id: UInt32,
        var event_bindings: List[ChildEventBinding],
        var auto_bindings: List[ChildAutoBinding],
    ):
        """Create a fully initialized ChildComponent.

        Args:
            scope_id: The child scope ID from create_child_scope().
            template_id: The registered template ID.
            event_bindings: Event handler bindings for the child.
            auto_bindings: Combined event + value auto-bindings.
        """
        self.scope_id = scope_id
        self.template_id = template_id
        self.current_vnode = -1
        self.slot = ConditionalSlot()
        self._event_bindings = event_bindings^
        self._auto_bindings = auto_bindings^

    def __init__(out self, *, copy: Self):
        self.scope_id = copy.scope_id
        self.template_id = copy.template_id
        self.current_vnode = copy.current_vnode
        self.slot = copy.slot.copy()
        self._event_bindings = copy._event_bindings.copy()
        self._auto_bindings = copy._auto_bindings.copy()

    def __init__(out self, *, deinit move: Self):
        self.scope_id = move.scope_id
        self.template_id = move.template_id
        self.current_vnode = move.current_vnode
        self.slot = move.slot.copy()
        self._event_bindings = move._event_bindings^
        self._auto_bindings = move._auto_bindings^

    # ── Slot initialization ──────────────────────────────────────────

    def init_slot(mut self, anchor_id: UInt32):
        """Initialize the ConditionalSlot with the anchor from dyn_node_ids.

        After the parent mounts, extract the anchor ElementId from the
        mounted VNode's `dyn_node_ids[N]` and pass it here.  This tells
        the ConditionalSlot which placeholder to replace when the child
        is first rendered.

        Args:
            anchor_id: ElementId of the placeholder comment node in the
                DOM that occupies the child's dyn_node slot.
        """
        self.slot = ConditionalSlot(anchor_id)

    def is_slot_initialized(self) -> Bool:
        """Check whether the slot has been initialized with an anchor."""
        return self.slot.anchor_id != 0 or self.slot.mounted

    # ── Render ───────────────────────────────────────────────────────

    def render_builder(
        self,
        store: UnsafePointer[VNodeStore, MutUntrackedOrigin],
        runtime: UnsafePointer[Runtime, MutUntrackedOrigin],
    ) -> ChildRenderBuilder:
        """Create a ChildRenderBuilder for this component's template.

        The caller fills in dynamic text values and calls `build()` to
        get the VNode index, which is then passed to `flush()`.

        Args:
            store: The VNodeStore pointer (shared with parent).
            runtime: The Runtime pointer (shared with parent).

        Returns:
            A ChildRenderBuilder with auto-binding support.
        """
        var vb = VNodeBuilder(self.template_id, store)
        if len(self._auto_bindings) > 0:
            return ChildRenderBuilder(vb^, self._auto_bindings.copy(), runtime)
        else:
            return ChildRenderBuilder(vb^, self._event_bindings.copy())

    def vnode_builder(
        self,
        store: UnsafePointer[VNodeStore, MutUntrackedOrigin],
    ) -> VNodeBuilder:
        """Create a raw VNodeBuilder for this component's template.

        Use this when you need full control over the VNode construction
        (e.g. for keyed children).  For the common case, prefer
        `render_builder()` which auto-adds event handlers.

        Args:
            store: The VNodeStore pointer (shared with parent).

        Returns:
            A VNodeBuilder for the child's template.
        """
        return VNodeBuilder(self.template_id, store)

    # ── Flush (DOM lifecycle) ────────────────────────────────────────

    def flush(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutUntrackedOrigin],
        eid_ptr: UnsafePointer[ElementIdAllocator, MutUntrackedOrigin],
        rt_ptr: UnsafePointer[Runtime, MutUntrackedOrigin],
        store_ptr: UnsafePointer[VNodeStore, MutUntrackedOrigin],
        new_vnode_idx: UInt32,
    ):
        """Flush the child: create or diff its VNode in the DOM.

        Delegates to `flush_conditional()` which handles:
          - **First render (empty → branch)**: CreateEngine builds the
            child VNode's DOM and replaces the placeholder anchor.
          - **Subsequent renders (branch → branch)**: DiffEngine diffs
            old vs new child VNode, emitting only changed mutations.

        Does NOT finalize the mutation buffer — the parent is responsible
        for calling `finalize()` after flushing all children.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            eid_ptr: Pointer to the ElementIdAllocator.
            rt_ptr: Pointer to the reactive Runtime.
            store_ptr: Pointer to the VNodeStore.
            new_vnode_idx: Index of the new child VNode from build().
        """
        self.slot = flush_conditional(
            writer_ptr, eid_ptr, rt_ptr, store_ptr, self.slot, new_vnode_idx
        )
        self.current_vnode = Int(new_vnode_idx)

    def flush_via_context(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutUntrackedOrigin],
        eid_alloc: UnsafePointer[ElementIdAllocator, MutUntrackedOrigin],
        runtime: UnsafePointer[Runtime, MutUntrackedOrigin],
        store: UnsafePointer[VNodeStore, MutUntrackedOrigin],
        new_vnode_idx: UInt32,
    ):
        """Flush the child using context pointers (convenience alias).

        Identical to `flush()` but with parameter names matching what
        AppShell/ComponentContext expose, for readability at call sites.

        Args:
            writer_ptr: Pointer to the MutationWriter.
            eid_alloc: Pointer to the ElementIdAllocator.
            runtime: Pointer to the Runtime.
            store: Pointer to the VNodeStore.
            new_vnode_idx: Index of the new child VNode.
        """
        self.flush(writer_ptr, eid_alloc, runtime, store, new_vnode_idx)

    def flush_empty(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutUntrackedOrigin],
        eid_ptr: UnsafePointer[ElementIdAllocator, MutUntrackedOrigin],
        rt_ptr: UnsafePointer[Runtime, MutUntrackedOrigin],
        store_ptr: UnsafePointer[VNodeStore, MutUntrackedOrigin],
    ):
        """Hide the child: remove its DOM content, restore placeholder.

        Delegates to `flush_conditional_empty()`.  After this call the
        slot returns to the empty state and the next `flush()` will do
        a fresh create.

        Does NOT finalize the mutation buffer.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            eid_ptr: Pointer to the ElementIdAllocator.
            rt_ptr: Pointer to the reactive Runtime.
            store_ptr: Pointer to the VNodeStore.
        """
        self.slot = flush_conditional_empty(
            writer_ptr, eid_ptr, rt_ptr, store_ptr, self.slot
        )
        self.current_vnode = -1

    # ── State queries ────────────────────────────────────────────────

    def is_mounted(self) -> Bool:
        """Check whether the child's VNode is currently in the DOM."""
        return self.slot.mounted

    def has_rendered(self) -> Bool:
        """Check whether this child has been rendered at least once."""
        return self.current_vnode >= 0

    # ── Dirty tracking ───────────────────────────────────────────────

    def is_dirty(
        self,
        runtime: UnsafePointer[Runtime, MutUntrackedOrigin],
    ) -> Bool:
        """Check if the child's scope is dirty.

        Returns True if the child scope has been marked dirty (one of
        its subscribed signals changed).

        Args:
            runtime: The Runtime pointer (shared with parent).

        Returns:
            True if the child scope needs re-rendering.
        """
        return runtime[0].scopes.is_dirty(self.scope_id)

    # ── Handler query ────────────────────────────────────────────────

    def event_handler_id(self, index: Int) -> UInt32:
        """Return the handler ID for the Nth event in this child.

        Args:
            index: Zero-based index into the event binding list.

        Returns:
            The handler ID at the given index.
        """
        return self._event_bindings[index].handler_id

    def event_count(self) -> Int:
        """Return the number of event bindings on this child."""
        return len(self._event_bindings)

    def auto_binding_count(self) -> Int:
        """Return the number of auto-bindings on this child."""
        return len(self._auto_bindings)

    # ── Destroy ──────────────────────────────────────────────────────

    def destroy(
        self,
        runtime: UnsafePointer[Runtime, MutUntrackedOrigin],
    ):
        """Destroy the child scope and clean up its handlers.

        Removes all event handlers registered under this child's scope
        and destroys the scope itself.  The parent is responsible for
        calling this during its own destroy.

        Args:
            runtime: The Runtime pointer (shared with parent).
        """
        runtime[0].handlers.remove_for_scope(self.scope_id)
        runtime[0].destroy_scope(self.scope_id)
