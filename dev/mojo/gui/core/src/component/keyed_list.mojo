# KeyedList — Abstraction for dynamic keyed list state management.
#
# KeyedList bundles the pieces of state that every keyed-list component
# needs:
#
#   1. FragmentSlot — tracks empty↔populated DOM transitions
#   2. scope_ids   — child scope IDs for per-item handler cleanup
#   3. template_id — the item template for building VNodes
#   4. handler_map — maps handler IDs → (action_tag, item_data) for dispatch
#
# Phase 17 adds ItemBuilder and HandlerAction for Dioxus-style ergonomics:
#
#   - `begin_item(key, ctx)` → `ItemBuilder` (bundles scope creation +
#     VNodeBuilder + handler map pointer)
#   - `ItemBuilder.add_custom_event(event, tag, data)` → registers a
#     custom handler, maps it to an app-defined action tag + data, and
#     adds the dynamic event attribute — all in one call
#   - `get_action(handler_id)` → `HandlerAction` for WASM-side dispatch
#
# Usage (Phase 17 — e.g. todo app):
#
#     struct TodoApp:
#         var ctx: ComponentContext
#         var items: KeyedList
#
#         fn build_item(mut self, item: TodoItem) -> UInt32:
#             var ib = self.items.begin_item(String(item.id), self.ctx)
#             ib.add_dyn_text(item.display_text())
#             ib.add_custom_event(String("click"), ACTION_TOGGLE, item.id)
#             ib.add_custom_event(String("click"), ACTION_REMOVE, item.id)
#             ib.add_dyn_text_attr(String("class"), item.class_name())
#             return ib.index()
#
#         fn build_items(mut self) -> UInt32:
#             var frag = self.items.begin_rebuild(self.ctx)
#             for i in range(len(self.data)):
#                 var idx = self.build_item(self.data[i])
#                 self.items.push_child(self.ctx, frag, idx)
#             return frag
#
#         fn handle_event(mut self, handler_id: UInt32) -> Bool:
#             var action = self.items.get_action(handler_id)
#             if not action.found:
#                 return False
#             if action.tag == ACTION_TOGGLE:
#                 self.toggle_item(action.data)
#                 return True
#             ...
#
# Compare with Phase 16 pattern (3–5 more lines per handler):
#
#     var child_scope = self.items.create_scope(self.ctx)
#     var vb = self.items.item_builder(String(item.id), self.ctx)
#     var toggle = self.ctx.register_handler(HandlerEntry.custom(child_scope, String("click")))
#     vb.add_dyn_event(String("click"), toggle)
#     self.handler_map.append(HandlerItemMapping(toggle, ACTION_TOGGLE, item.id))
#
# The Phase 16 methods (create_scope, item_builder, push_child) remain
# available for apps that prefer the manual pattern.

from memory import UnsafePointer
from bridge import MutationWriter
from events import HandlerEntry
from .lifecycle import FragmentSlot
from .context import ComponentContext
from signals import Runtime
from signals.handle import SignalString
from html import VNodeBuilder


# ══════════════════════════════════════════════════════════════════════════════
# _HandlerMapping — Internal handler → action mapping (private)
# ══════════════════════════════════════════════════════════════════════════════


@fieldwise_init
struct _HandlerMapping(Copyable, Equatable, Writable):
    """Maps a handler ID to an app-defined action tag and data value.

    This is the internal storage type used by KeyedList's handler_map.
    Apps never see this directly — they use `HandlerAction` from
    `get_action()` instead.

    `Equatable` and `Writable` are auto-derived from fields.
    """

    var handler_id: UInt32
    var tag: UInt8
    var data: Int32

    fn __copyinit__(out self, other: Self):
        self.handler_id = other.handler_id
        self.tag = other.tag
        self.data = other.data

    fn __moveinit__(out self, deinit other: Self):
        self.handler_id = other.handler_id
        self.tag = other.tag
        self.data = other.data


# ══════════════════════════════════════════════════════════════════════════════
# HandlerAction — Result of looking up a handler ID in the handler map
# ══════════════════════════════════════════════════════════════════════════════


struct HandlerAction(Copyable, Equatable, Writable):
    """Result of looking up a handler ID via `KeyedList.get_action()`.

    Fields:
        tag:   The app-defined action tag (e.g. ACTION_TOGGLE, ACTION_REMOVE).
        data:  The associated data value (e.g. item ID).
        found: Whether the handler ID was found in the map.

    `Equatable` and `Writable` are auto-derived from fields.

    Usage:
        var action = self.items.get_action(handler_id)
        if action.found and action.tag == ACTION_TOGGLE:
            self.toggle_item(action.data)
    """

    var tag: UInt8
    var data: Int32
    var found: Bool

    fn __init__(out self):
        """Create a not-found sentinel."""
        self.tag = 0
        self.data = 0
        self.found = False

    fn __init__(out self, tag: UInt8, data: Int32):
        """Create a found result with the given tag and data."""
        self.tag = tag
        self.data = data
        self.found = True

    fn __copyinit__(out self, other: Self):
        self.tag = other.tag
        self.data = other.data
        self.found = other.found

    fn __moveinit__(out self, deinit other: Self):
        self.tag = other.tag
        self.data = other.data
        self.found = other.found


# ══════════════════════════════════════════════════════════════════════════════
# ItemBuilder — Ergonomic per-item VNode + handler builder
# ══════════════════════════════════════════════════════════════════════════════


struct ItemBuilder(Movable):
    """Ergonomic builder for a single keyed list item.

    Created by `KeyedList.begin_item()`, which handles child scope
    creation and VNodeBuilder setup automatically.  ItemBuilder wraps
    a VNodeBuilder and adds convenience methods for registering custom
    event handlers with action mapping.

    Usage:

        var ib = self.items.begin_item(String(item.id), self.ctx)
        ib.add_dyn_text(item.label)
        ib.add_custom_event(String("click"), ACTION_TOGGLE, item.id)
        ib.add_custom_event(String("click"), ACTION_REMOVE, item.id)
        ib.add_dyn_text_attr(String("class"), class_name)
        return ib.index()

    Compare with the manual pattern (Phase 16):

        var child_scope = self.items.create_scope(self.ctx)
        var vb = self.items.item_builder(String(item.id), self.ctx)
        vb.add_dyn_text(item.label)
        var toggle = self.ctx.register_handler(
            HandlerEntry.custom(child_scope, String("click")))
        vb.add_dyn_event(String("click"), toggle)
        self.handler_map.append(
            HandlerItemMapping(toggle, ACTION_TOGGLE, item.id))
        ...

    ItemBuilder holds:
      - A VNodeBuilder (moved in, owns the VNode being constructed)
      - The child scope ID (for registering handlers)
      - A non-owning pointer to the Runtime (for handler registration)
      - A non-owning pointer to the KeyedList's handler_map
    """

    var vb: VNodeBuilder
    var scope_id: UInt32
    var _runtime: UnsafePointer[Runtime, MutExternalOrigin]
    var _handler_map_ptr: UnsafePointer[
        List[_HandlerMapping], MutExternalOrigin
    ]

    fn __init__(
        out self,
        var vb: VNodeBuilder,
        scope_id: UInt32,
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
        handler_map_ptr: UnsafePointer[
            List[_HandlerMapping], MutExternalOrigin
        ],
    ):
        """Create an ItemBuilder (called internally by KeyedList.begin_item).

        Args:
            vb: The VNodeBuilder for this item (moved in).
            scope_id: The child scope ID for handler registration.
            runtime: Non-owning pointer to the Runtime.
            handler_map_ptr: Non-owning pointer to the KeyedList's handler_map.
        """
        self.vb = vb^
        self.scope_id = scope_id
        self._runtime = runtime
        self._handler_map_ptr = handler_map_ptr

    fn __moveinit__(out self, deinit other: Self):
        self.vb = other.vb^
        self.scope_id = other.scope_id
        self._runtime = other._runtime
        self._handler_map_ptr = other._handler_map_ptr

    # ── Dynamic text ─────────────────────────────────────────────────

    fn add_dyn_text(mut self, value: String):
        """Add a dynamic text node (fills the next DynamicText slot).

        Call in order corresponding to `dyn_text(0)`, `dyn_text(1)`, ...
        placeholders in the item template.

        Args:
            value: The text content for this slot.
        """
        self.vb.add_dyn_text(value)

    fn add_dyn_text_signal(mut self, signal: SignalString):
        """Add a dynamic text node from a SignalString value.

        Reads the signal's current value (via peek/get — no subscription)
        and adds it as the next dynamic text slot.  This is a convenience
        for the common pattern of reading a SignalString and passing it
        to `add_dyn_text()`.

        Args:
            signal: The SignalString to read.
        """
        self.vb.add_dyn_text(signal.get())

    # ── Dynamic attributes ───────────────────────────────────────────

    fn add_dyn_text_attr(mut self, name: String, value: String):
        """Add a dynamic text attribute (e.g. class, id, href).

        Args:
            name: The attribute name.
            value: The attribute value.
        """
        self.vb.add_dyn_text_attr(name, value)

    fn add_dyn_bool_attr(mut self, name: String, value: Bool):
        """Add a dynamic boolean attribute (e.g. disabled, checked).

        Args:
            name: The attribute name.
            value: The attribute value.
        """
        self.vb.add_dyn_bool_attr(name, value)

    # ── Events (manual) ──────────────────────────────────────────────

    fn add_dyn_event(mut self, event_name: String, handler_id: UInt32):
        """Add a dynamic event attribute with an existing handler ID.

        Use this when the handler is already registered (e.g. via
        `ctx.register_handler()`).  For the ergonomic one-call pattern,
        use `add_custom_event()` instead.

        Args:
            event_name: The DOM event name (e.g. "click").
            handler_id: The pre-registered handler ID.
        """
        self.vb.add_dyn_event(event_name, handler_id)

    # ── Events (ergonomic — register + map + add in one call) ────────

    fn add_custom_event(
        mut self, event_name: String, action_tag: UInt8, data: Int32
    ):
        """Register a custom event handler with action mapping.

        This is the ergonomic one-call alternative to the manual pattern
        of `register_handler()` + `add_dyn_event()` + `handler_map.append()`.

        Performs three operations atomically:
        1. Registers a custom handler in the Runtime's handler registry
        2. Stores the handler_id → (action_tag, data) mapping
        3. Adds a dynamic event attribute to the VNode

        The action_tag and data are app-defined values retrievable later
        via `KeyedList.get_action(handler_id)`.

        Args:
            event_name: The DOM event name (e.g. "click", "input").
            action_tag: App-defined action tag (e.g. ACTION_TOGGLE = 1).
            data: App-defined data (e.g. item ID for toggle/remove).
        """
        # 1. Register the custom handler in the Runtime
        var handler_id = self._runtime[0].register_handler(
            HandlerEntry.custom(self.scope_id, event_name)
        )

        # 2. Store the handler → action mapping
        self._handler_map_ptr[0].append(
            _HandlerMapping(handler_id, action_tag, data)
        )

        # 3. Add the dynamic event attribute to the VNode
        self.vb.add_dyn_event(event_name, handler_id)

    # ── Conditional class helpers ────────────────────────────────────

    fn add_class_if(mut self, condition: Bool, class_name: String):
        """Add a conditional CSS class attribute.

        Shortcut for `add_dyn_text_attr("class", class_if(condition, name))`.
        Adds the class name if condition is True, or an empty string
        if False.

        Replaces the common 4–5 line pattern:

            var cls: String
            if item.completed:
                cls = String("completed")
            else:
                cls = String("")
            ib.add_dyn_text_attr(String("class"), cls)

        With a single call:

            ib.add_class_if(item.completed, String("completed"))

        Args:
            condition: Whether to apply the class.
            class_name: The CSS class name to apply when True.
        """
        if condition:
            self.vb.add_dyn_text_attr(String("class"), class_name)
        else:
            self.vb.add_dyn_text_attr(String("class"), String(""))

    fn add_class_when(
        mut self,
        condition: Bool,
        true_class: String,
        false_class: String,
    ):
        """Add one of two CSS class names based on a condition.

        Shortcut for `add_dyn_text_attr("class", class_when(cond, a, b))`.
        For binary class switching (e.g. "active" vs "inactive").

        Example:
            ib.add_class_when(is_selected, String("danger"), String(""))

        Args:
            condition: The boolean condition.
            true_class: Class name when True.
            false_class: Class name when False.
        """
        if condition:
            self.vb.add_dyn_text_attr(String("class"), true_class)
        else:
            self.vb.add_dyn_text_attr(String("class"), false_class)

    # ── Placeholder (for dyn_node slots) ─────────────────────────────

    fn add_dyn_placeholder(mut self):
        """Add a dynamic placeholder node."""
        self.vb.add_dyn_placeholder()

    # ── Finalize ─────────────────────────────────────────────────────

    fn index(self) -> UInt32:
        """Return the VNode index of the item being built.

        Call this after all dynamic text, attributes, and events have
        been added.  The returned index is used with
        `KeyedList.push_child()` to add the item to the fragment.

        Returns:
            The VNode's index in the VNodeStore.
        """
        return self.vb.index()


# ══════════════════════════════════════════════════════════════════════════════
# KeyedList — Dynamic keyed list state management
# ══════════════════════════════════════════════════════════════════════════════


struct KeyedList(Movable):
    """Manages the state for a dynamic keyed list within a component.

    Bundles the FragmentSlot (DOM lifecycle), child scope IDs (handler
    cleanup), item template ID (VNode construction), and handler map
    (action dispatch) that every keyed-list component needs.

    Phase 17 adds `begin_item()` and `get_action()` for ergonomic
    per-item building and event dispatch.  The Phase 16 methods
    (`create_scope`, `item_builder`, `push_child`) remain available
    for apps that prefer the manual pattern.

    Typical lifecycle:

        # In __init__:
        self.items = KeyedList(ctx.register_extra_template(item_view, name))

        # In build_items:
        var frag = self.items.begin_rebuild(ctx)
        for i in range(len(self.data)):
            var ib = self.items.begin_item(String(id), ctx)
            ib.add_dyn_text(...)
            ib.add_custom_event("click", ACTION_TOGGLE, id)
            self.items.push_child(ctx, frag, ib.index())
        return frag

        # In handle_event:
        var action = self.items.get_action(handler_id)
        if action.found: ...

        # In flush:
        self.items.flush(ctx, writer, new_frag)
    """

    var template_id: UInt32
    var slot: FragmentSlot
    var scope_ids: List[UInt32]
    var handler_map: List[_HandlerMapping]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create an uninitialized KeyedList (no template).

        Call `init_slot()` after the initial mount to set the anchor.
        """
        self.template_id = 0
        self.slot = FragmentSlot()
        self.scope_ids = List[UInt32]()
        self.handler_map = List[_HandlerMapping]()

    fn __init__(out self, template_id: UInt32):
        """Create a KeyedList for the given item template.

        Args:
            template_id: The registered template ID for list items.
        """
        self.template_id = template_id
        self.slot = FragmentSlot()
        self.scope_ids = List[UInt32]()
        self.handler_map = List[_HandlerMapping]()

    fn __moveinit__(out self, deinit other: Self):
        self.template_id = other.template_id
        self.slot = other.slot^
        self.scope_ids = other.scope_ids^
        self.handler_map = other.handler_map^

    # ── Slot initialization ──────────────────────────────────────────

    fn init_slot(mut self, anchor_id: UInt32, frag_idx: UInt32):
        """Initialize the fragment slot after the initial mount.

        Call this after CreateEngine has assigned an ElementId to the
        list's anchor placeholder node.

        Args:
            anchor_id: ElementId of the placeholder/anchor in the DOM.
            frag_idx: VNode index of the initial (typically empty) fragment.
        """
        self.slot = FragmentSlot(anchor_id, Int(frag_idx))

    # ── Rebuild lifecycle ────────────────────────────────────────────

    fn begin_rebuild(mut self, mut ctx: ComponentContext) -> UInt32:
        """Start a keyed list rebuild: destroy old scopes, return empty fragment.

        Destroys all tracked child scopes (cleaning up their handlers),
        clears the scope list AND handler map, and creates a new empty
        Fragment VNode.

        Call this at the start of your build_items method, then iterate
        over your data calling `begin_item()` (or `create_scope()` +
        `item_builder()`) for each item, and `push_child()` to add
        each built VNode.

        Args:
            ctx: The owning component's context (mutated to destroy scopes).

        Returns:
            VNode index of the new empty Fragment.
        """
        ctx.destroy_child_scopes(self.scope_ids)
        self.scope_ids.clear()
        self.handler_map.clear()
        return ctx.build_empty_fragment()

    # ── Phase 17 — Ergonomic item building ───────────────────────────

    fn begin_item(
        mut self, key: String, mut ctx: ComponentContext
    ) -> ItemBuilder:
        """Begin building a keyed list item with automatic scope + builder.

        Creates a child scope (tracked for cleanup), creates a keyed
        VNodeBuilder, and returns an `ItemBuilder` that bundles both
        with convenience methods for dynamic text, attributes, and
        custom event registration.

        This is the Phase 17 ergonomic alternative to the manual
        `create_scope()` + `item_builder()` pattern.

        Usage:
            var ib = self.items.begin_item(String(item.id), self.ctx)
            ib.add_dyn_text(item.label)
            ib.add_custom_event(String("click"), ACTION_TOGGLE, item.id)
            return ib.index()

        Args:
            key: The unique key for this item (for keyed diffing).
            ctx: The owning component's context.

        Returns:
            An ItemBuilder ready for add_dyn_text/add_custom_event calls.
        """
        var scope_id = ctx.create_child_scope()
        self.scope_ids.append(scope_id)
        var vb = VNodeBuilder(self.template_id, key, ctx.store_ptr())
        var handler_map_ptr = UnsafePointer(to=self.handler_map)
        return ItemBuilder(
            vb^,
            scope_id,
            ctx.runtime_ptr(),
            UnsafePointer[List[_HandlerMapping], MutExternalOrigin](
                unsafe_from_address=Int(handler_map_ptr)
            ),
        )

    # ── Phase 17 — Handler action dispatch ───────────────────────────

    fn get_action(self, handler_id: UInt32) -> HandlerAction:
        """Look up a handler ID in the handler map.

        Returns a `HandlerAction` with the app-defined action tag and
        data if found, or a not-found sentinel otherwise.

        Use this for WASM-side event dispatch in components that handle
        custom events (e.g. toggle/remove in a todo app).

        Args:
            handler_id: The handler ID from the event dispatch.

        Returns:
            A HandlerAction with `found=True` and the stored tag/data,
            or `found=False` if the handler ID is not in the map.
        """
        for i in range(len(self.handler_map)):
            if self.handler_map[i].handler_id == handler_id:
                return HandlerAction(
                    self.handler_map[i].tag,
                    self.handler_map[i].data,
                )
        return HandlerAction()

    # ── Phase 16 — Manual item building (still available) ────────────

    fn create_scope(mut self, mut ctx: ComponentContext) -> UInt32:
        """Create a child scope for a list item and track it.

        The scope is automatically cleaned up on the next `begin_rebuild()`
        or when the component is destroyed.

        This is the Phase 16 manual alternative to `begin_item()`.
        Use `begin_item()` for the ergonomic one-call pattern.

        Args:
            ctx: The owning component's context.

        Returns:
            The new child scope ID for handler registration.
        """
        var scope_id = ctx.create_child_scope()
        self.scope_ids.append(scope_id)
        return scope_id

    fn item_builder(self, key: String, ctx: ComponentContext) -> VNodeBuilder:
        """Create a keyed VNodeBuilder for a list item.

        Uses this KeyedList's template_id and the component's store.

        This is the Phase 16 manual alternative to `begin_item()`.
        Use `begin_item()` for the ergonomic one-call pattern.

        Args:
            key: The unique key for this item (for keyed diffing).
            ctx: The owning component's context.

        Returns:
            A VNodeBuilder ready for add_dyn_text/add_dyn_event calls.
        """
        return VNodeBuilder(self.template_id, key, ctx.store_ptr())

    fn push_child(
        self, ctx: ComponentContext, frag_idx: UInt32, child_idx: UInt32
    ):
        """Append a built item VNode to the fragment.

        Args:
            ctx: The owning component's context.
            frag_idx: The Fragment VNode index (from begin_rebuild).
            child_idx: The item VNode index (from item_builder().index()
                       or begin_item().index()).
        """
        ctx.push_fragment_child(frag_idx, child_idx)

    # ── Flush lifecycle ──────────────────────────────────────────────

    fn flush(
        mut self,
        mut ctx: ComponentContext,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        new_frag_idx: UInt32,
    ):
        """Flush the keyed list: diff old vs new fragment, emit mutations.

        Delegates to ComponentContext.flush_fragment() which handles all
        three transitions (empty→populated, populated→populated,
        populated→empty).

        Does NOT call finalize() — the caller must finalize the mutation
        buffer after this returns.

        Args:
            ctx: The owning component's context.
            writer_ptr: Pointer to the MutationWriter for output.
            new_frag_idx: Index of the new Fragment VNode from rebuild.
        """
        self.slot = ctx.flush_fragment(writer_ptr, self.slot, new_frag_idx)

    # ── Queries ──────────────────────────────────────────────────────

    fn scope_count(self) -> Int:
        """Return the number of tracked child scopes."""
        return len(self.scope_ids)

    fn handler_count(self) -> Int:
        """Return the number of handler→action mappings.

        This reflects the number of custom events registered via
        `ItemBuilder.add_custom_event()` since the last `begin_rebuild()`.
        """
        return len(self.handler_map)

    fn is_mounted(self) -> Bool:
        """Check whether the list has items in the DOM."""
        return self.slot.mounted
