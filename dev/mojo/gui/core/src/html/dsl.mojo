# Ergonomic Builder DSL — High-level API for constructing Templates and VNodes.
#
# This module provides a declarative, tree-shaped API that replaces the
# low-level step-by-step TemplateBuilder calls with composable helper
# functions.  It is the Tier 1 "Runtime Builder" from the plan's
# Ergonomics-First API Design section (M10.5).
#
# Instead of:
#
#     var b = TemplateBuilder("counter")
#     var div_idx = b.push_element(TAG_DIV, -1)
#     var span_idx = b.push_element(TAG_SPAN, Int(div_idx))
#     var _dyn = b.push_dynamic_text(0, Int(span_idx))
#     var btn = b.push_element(TAG_BUTTON, Int(div_idx))
#     b.push_text("+", Int(btn))
#     b.push_dynamic_attr(Int(btn), 0)
#     var template = b.build()
#
# Developers write:
#
#     var view = el_div([
#         el_span([dyn_text(0)]),
#         el_button([text("+"), dyn_attr(0)]),
#         el_button([text("-"), dyn_attr(1)]),
#     ])
#     var template = to_template(view, "counter")
#
# And instead of:
#
#     var idx = store[0].push(VNode.template_ref(tmpl_id))
#     store[0].push_dynamic_node(idx, DynamicNode.text_node("Count: 0"))
#     store[0].push_dynamic_attr(idx, DynamicAttr("click", AttributeValue.event(h1), UInt32(0)))
#     store[0].push_dynamic_attr(idx, DynamicAttr("click", AttributeValue.event(h2), UInt32(0)))
#
# Developers write:
#
#     var vb = VNodeBuilder(tmpl_id, store)
#     vb.add_dyn_text("Count: 0")
#     vb.add_dyn_event("click", h1)
#     vb.add_dyn_event("click", h2)
#     var vnode_idx = vb.index()
#
# Both forms produce identical Template and VNode structures.

from memory import UnsafePointer
from vdom.builder import TemplateBuilder
from vdom.template import Template
from vdom.vnode import (
    VNode,
    VNodeStore,
    DynamicNode,
    DynamicAttr,
    AttributeValue,
)
from .tags import (
    TAG_DIV,
    TAG_SPAN,
    TAG_P,
    TAG_SECTION,
    TAG_HEADER,
    TAG_FOOTER,
    TAG_NAV,
    TAG_MAIN,
    TAG_ARTICLE,
    TAG_ASIDE,
    TAG_H1,
    TAG_H2,
    TAG_H3,
    TAG_H4,
    TAG_H5,
    TAG_H6,
    TAG_UL,
    TAG_OL,
    TAG_LI,
    TAG_BUTTON,
    TAG_INPUT,
    TAG_FORM,
    TAG_TEXTAREA,
    TAG_SELECT,
    TAG_OPTION,
    TAG_LABEL,
    TAG_A,
    TAG_IMG,
    TAG_TABLE,
    TAG_THEAD,
    TAG_TBODY,
    TAG_TR,
    TAG_TD,
    TAG_TH,
    TAG_STRONG,
    TAG_EM,
    TAG_BR,
    TAG_HR,
    TAG_PRE,
    TAG_CODE,
    TAG_UNKNOWN,
)


# ══════════════════════════════════════════════════════════════════════════════
# Conditional helpers — Reduce if/else boilerplate for dynamic attributes
# ══════════════════════════════════════════════════════════════════════════════
#
# These are runtime string utilities used alongside the DSL for concise
# conditional rendering.  They complement `text()`, `dyn_text()`, etc.
#
# Instead of:
#
#     var tr_class: String
#     if selected == row.id:
#         tr_class = String("danger")
#     else:
#         tr_class = String("")
#     ib.add_dyn_text_attr(String("class"), tr_class)
#
# Write:
#
#     ib.add_dyn_text_attr(String("class"), class_if(selected == row.id, String("danger")))
#
# Or even shorter with ItemBuilder convenience methods:
#
#     ib.add_class_if(selected == row.id, String("danger"))


fn class_if(condition: Bool, name: String) -> String:
    """Return the class name if condition is True, empty string otherwise.

    A concise alternative to if/else blocks for conditional CSS classes.

    Example:
        var cls = class_if(item.completed, String("completed"))
        # Returns "completed" if True, "" if False

    Args:
        condition: Whether to include the class.
        name: The CSS class name.

    Returns:
        The class name or an empty string.
    """
    if condition:
        return name
    return String("")


fn class_when(
    condition: Bool, true_class: String, false_class: String
) -> String:
    """Return one of two class names based on a condition.

    For binary class switching (e.g. "active" vs "inactive").

    Example:
        var cls = class_when(is_open, String("open"), String("closed"))

    Args:
        condition: The boolean condition.
        true_class: Class name when True.
        false_class: Class name when False.

    Returns:
        The appropriate class name.
    """
    if condition:
        return true_class
    return false_class


fn text_when(condition: Bool, true_text: String, false_text: String) -> String:
    """Return one of two strings based on a condition.

    General-purpose conditional text helper.

    Example:
        var label = text_when(item.completed, String("✓ Done"), item.text)

    Args:
        condition: The boolean condition.
        true_text: Text when True.
        false_text: Text when False.

    Returns:
        The appropriate text.
    """
    if condition:
        return true_text
    return false_text


# ══════════════════════════════════════════════════════════════════════════════
# Node — Tagged union for the declarative element tree
# ══════════════════════════════════════════════════════════════════════════════

# ── Node kind tags ───────────────────────────────────────────────────────────

comptime NODE_TEXT: UInt8 = 0  # Static text content
comptime NODE_ELEMENT: UInt8 = 1  # HTML element with tag, children, attrs
comptime NODE_DYN_TEXT: UInt8 = 2  # Dynamic text placeholder (slot index)
comptime NODE_DYN_NODE: UInt8 = 3  # Dynamic node placeholder (slot index)
comptime NODE_STATIC_ATTR: UInt8 = 4  # Static attribute (name + value)
comptime NODE_DYN_ATTR: UInt8 = 5  # Dynamic attribute placeholder (slot index)
comptime NODE_EVENT: UInt8 = 6  # Inline event handler (action + signal + operand)
comptime NODE_BIND_VALUE: UInt8 = 7  # Value binding (SignalString → dynamic attr)

# ── Auto-numbering sentinel ──────────────────────────────────────────────────
#
# When a dyn_text() node is created without an explicit index, it uses this
# sentinel value.  ComponentContext.register_view() / setup_view() will
# auto-assign sequential indices (0, 1, 2, ...) in tree-walk order.

comptime DYN_TEXT_AUTO: UInt32 = 0xFFFFFFFF


struct Node(Copyable):
    """A declarative description of a UI element tree node.

    Node is a tagged union that can represent static text, HTML elements
    (with children and attributes), dynamic text placeholders, dynamic
    node placeholders, static attributes, dynamic attribute placeholders,
    or inline event handler bindings.

    Nodes are composed into trees using the tag helper functions and then
    converted to a Template via `to_template()`.

    For ELEMENT nodes, `items` contains a mix of children (TEXT, ELEMENT,
    DYN_TEXT, DYN_NODE) and attributes (STATIC_ATTR, DYN_ATTR, EVENT).
    The `to_template()` function separates them automatically.

    EVENT nodes carry handler metadata (action type, signal key, operand)
    and are processed by `ComponentContext.register_view()` which
    auto-assigns dynamic attribute indices and registers handlers.
    """

    var kind: UInt8  # NODE_* tag
    var tag: UInt8  # HTML tag constant (ELEMENT) or action tag (EVENT)
    var text: String  # text content (TEXT), attr name (STATIC_ATTR), event name (EVENT)
    var attr_value: String  # attr value (STATIC_ATTR only)
    var dynamic_index: UInt32  # slot index (DYN_TEXT, DYN_NODE, DYN_ATTR) or signal key (EVENT)
    var operand: Int32  # event handler operand (EVENT only, 0 otherwise)
    var items: List[Node]  # children + inline attrs (ELEMENT only)

    # ── Named constructors ───────────────────────────────────────────

    @staticmethod
    fn text_node(s: String) -> Self:
        """Create a static text node."""
        return Self(
            kind=NODE_TEXT,
            tag=TAG_UNKNOWN,
            text=s,
            attr_value=String(""),
            dynamic_index=0,
            operand=0,
            items=List[Node](),
        )

    @staticmethod
    fn element_node(html_tag: UInt8, var items: List[Node]) -> Self:
        """Create an element node with the given tag and items.

        Items can include children (text, element, dyn_text, dyn_node)
        and attributes (static_attr, dyn_attr, event) in any order.
        """
        return Self(
            kind=NODE_ELEMENT,
            tag=html_tag,
            text=String(""),
            attr_value=String(""),
            dynamic_index=0,
            operand=0,
            items=items^,
        )

    @staticmethod
    fn element_node_empty(html_tag: UInt8) -> Self:
        """Create an element node with no children or attributes."""
        return Self(
            kind=NODE_ELEMENT,
            tag=html_tag,
            text=String(""),
            attr_value=String(""),
            dynamic_index=0,
            operand=0,
            items=List[Node](),
        )

    @staticmethod
    fn dynamic_text_node(index: UInt32) -> Self:
        """Create a dynamic text placeholder (fills a DynamicText slot)."""
        return Self(
            kind=NODE_DYN_TEXT,
            tag=TAG_UNKNOWN,
            text=String(""),
            attr_value=String(""),
            dynamic_index=index,
            operand=0,
            items=List[Node](),
        )

    @staticmethod
    fn dynamic_node_slot(index: UInt32) -> Self:
        """Create a dynamic node placeholder (fills a Dynamic slot)."""
        return Self(
            kind=NODE_DYN_NODE,
            tag=TAG_UNKNOWN,
            text=String(""),
            attr_value=String(""),
            dynamic_index=index,
            operand=0,
            items=List[Node](),
        )

    @staticmethod
    fn static_attr_node(name: String, value: String) -> Self:
        """Create a static attribute (name=value)."""
        return Self(
            kind=NODE_STATIC_ATTR,
            tag=TAG_UNKNOWN,
            text=name,
            attr_value=value,
            dynamic_index=0,
            operand=0,
            items=List[Node](),
        )

    @staticmethod
    fn dynamic_attr_node(index: UInt32) -> Self:
        """Create a dynamic attribute placeholder (fills a dynamic attr slot).
        """
        return Self(
            kind=NODE_DYN_ATTR,
            tag=TAG_UNKNOWN,
            text=String(""),
            attr_value=String(""),
            dynamic_index=index,
            operand=0,
            items=List[Node](),
        )

    @staticmethod
    fn event_node(
        event_name: String, action: UInt8, signal_key: UInt32, operand: Int32
    ) -> Self:
        """Create an inline event handler node.

        EVENT nodes carry the full handler specification (action type,
        signal key, operand) and are processed by
        `ComponentContext.register_view()` which auto-assigns dynamic
        attribute indices and registers the handler.

        Args:
            event_name: DOM event name (e.g. "click", "input").
            action: Handler action tag (ACTION_SIGNAL_ADD_I32, etc.).
            signal_key: The signal key to modify.
            operand: The operand value for the action.

        Returns:
            A NODE_EVENT node.
        """
        return Self(
            kind=NODE_EVENT,
            tag=action,
            text=event_name,
            attr_value=String(""),
            dynamic_index=signal_key,
            operand=operand,
            items=List[Node](),
        )

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        kind: UInt8,
        tag: UInt8,
        text: String,
        attr_value: String,
        dynamic_index: UInt32,
        operand: Int32,
        var items: List[Node],
    ):
        self.kind = kind
        self.tag = tag
        self.text = text
        self.attr_value = attr_value
        self.dynamic_index = dynamic_index
        self.operand = operand
        self.items = items^

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.tag = other.tag
        self.text = other.text
        self.attr_value = other.attr_value
        self.dynamic_index = other.dynamic_index
        self.operand = other.operand
        self.items = other.items.copy()

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.tag = other.tag
        self.text = other.text^
        self.attr_value = other.attr_value^
        self.dynamic_index = other.dynamic_index
        self.operand = other.operand
        self.items = other.items^

    # ── Queries ──────────────────────────────────────────────────────

    fn is_text(self) -> Bool:
        """Check whether this is a static text node."""
        return self.kind == NODE_TEXT

    fn is_element(self) -> Bool:
        """Check whether this is an element node."""
        return self.kind == NODE_ELEMENT

    fn is_dyn_text(self) -> Bool:
        """Check whether this is a dynamic text placeholder."""
        return self.kind == NODE_DYN_TEXT

    fn is_dyn_node(self) -> Bool:
        """Check whether this is a dynamic node placeholder."""
        return self.kind == NODE_DYN_NODE

    fn is_static_attr(self) -> Bool:
        """Check whether this is a static attribute."""
        return self.kind == NODE_STATIC_ATTR

    fn is_dyn_attr(self) -> Bool:
        """Check whether this is a dynamic attribute placeholder."""
        return self.kind == NODE_DYN_ATTR

    fn is_event(self) -> Bool:
        """Check whether this is an inline event handler node."""
        return self.kind == NODE_EVENT

    fn is_bind_value(self) -> Bool:
        """Check whether this is a value binding node."""
        return self.kind == NODE_BIND_VALUE

    fn is_attr(self) -> Bool:
        """Check whether this is any kind of attribute (static, dynamic, event, or binding).
        """
        return (
            self.kind == NODE_STATIC_ATTR
            or self.kind == NODE_DYN_ATTR
            or self.kind == NODE_EVENT
            or self.kind == NODE_BIND_VALUE
        )

    fn is_child(self) -> Bool:
        """Check whether this is a child node (not an attribute)."""
        return not self.is_attr()

    fn item_count(self) -> Int:
        """Return the number of items (children + attrs) in an element."""
        return len(self.items)

    fn child_count(self) -> Int:
        """Return the number of child nodes (excluding attrs) in an element."""
        var count = 0
        for i in range(len(self.items)):
            if self.items[i].is_child():
                count += 1
        return count

    fn attr_count(self) -> Int:
        """Return the number of attributes (excluding children) in an element.
        """
        var count = 0
        for i in range(len(self.items)):
            if self.items[i].is_attr():
                count += 1
        return count

    fn static_attr_count(self) -> Int:
        """Return the number of static attributes in an element."""
        var count = 0
        for i in range(len(self.items)):
            if self.items[i].kind == NODE_STATIC_ATTR:
                count += 1
        return count

    fn event_count(self) -> Int:
        """Return the number of inline event handler nodes in an element."""
        var count = 0
        for i in range(len(self.items)):
            if self.items[i].kind == NODE_EVENT:
                count += 1
        return count

    fn bind_value_count(self) -> Int:
        """Return the number of value binding nodes in an element."""
        var count = 0
        for i in range(len(self.items)):
            if self.items[i].kind == NODE_BIND_VALUE:
                count += 1
        return count

    fn dynamic_attr_count(self) -> Int:
        """Return the number of dynamic attribute placeholders in an element.

        Includes explicit dyn_attr, inline event, and value binding nodes.
        """
        var count = 0
        for i in range(len(self.items)):
            if (
                self.items[i].kind == NODE_DYN_ATTR
                or self.items[i].kind == NODE_EVENT
                or self.items[i].kind == NODE_BIND_VALUE
            ):
                count += 1
        return count

    # ── Mutation ─────────────────────────────────────────────────────

    fn add_item(mut self, var item: Node):
        """Append an item (child or attribute) to this element node."""
        self.items.append(item^)


# ══════════════════════════════════════════════════════════════════════════════
# Free-function helpers — The ergonomic API surface
# ══════════════════════════════════════════════════════════════════════════════

# ── Leaf constructors ────────────────────────────────────────────────────────


fn text(s: String) -> Node:
    """Create a static text node.

    Usage: `text("Hello, world!")`
    """
    return Node.text_node(s)


fn dyn_text(index: Int) -> Node:
    """Create a dynamic text placeholder with an explicit slot index.

    The `index` identifies which slot in the VNode's dynamic_nodes list
    this placeholder fills.

    Usage: `dyn_text(0)` → first dynamic text slot
    """
    return Node.dynamic_text_node(UInt32(index))


fn dyn_text() -> Node:
    """Create an auto-numbered dynamic text placeholder.

    The slot index will be auto-assigned by `ComponentContext.setup_view()`
    or `ComponentContext.register_view()` in tree-walk order (0, 1, 2, ...).

    This eliminates manual index tracking and brings the DSL closer to
    Dioxus's `{count}` interpolation syntax.

    Usage:
        el_h1([dyn_text()])  # auto-assigned index 0
        el_p([dyn_text()])   # auto-assigned index 1
    """
    return Node.dynamic_text_node(DYN_TEXT_AUTO)


fn dyn_node(index: Int) -> Node:
    """Create a dynamic node placeholder.

    The `index` identifies which slot in the VNode's dynamic_nodes list
    this placeholder fills.

    Usage: `dyn_node(0)` → first dynamic node slot
    """
    return Node.dynamic_node_slot(UInt32(index))


fn attr(name: String, value: String) -> Node:
    """Create a static attribute.

    Usage: `attr("class", "container")`
    """
    return Node.static_attr_node(name, value)


fn dyn_attr(index: Int) -> Node:
    """Create a dynamic attribute placeholder.

    The `index` identifies which slot in the VNode's dynamic_attrs list
    this placeholder fills.

    Usage: `dyn_attr(0)` → first dynamic attribute slot
    """
    return Node.dynamic_attr_node(UInt32(index))


# ── Boolean / conditional attribute helpers ──────────────────────────────────
#
# HTML boolean attributes (disabled, checked, hidden, selected, open, readonly,
# required, etc.) are "present = true, absent = false".  Setting `disabled=""`
# means disabled; removing the attribute means not disabled.
#
# At the VNodeBuilder / RenderContext level, use `add_dyn_bool_attr(name, cond)`
# which stores AVAL_TEXT("") when true and AVAL_NONE when false.  The diff
# engine emits RemoveAttribute for AVAL_NONE transitions.
#
# The helpers below are runtime string utilities for use with
# `add_dyn_text_attr()` — analogous to `class_if()` and `class_when()`.


fn attr_if(condition: Bool, value: String) -> String:
    """Return `value` if condition is True, empty string otherwise.

    A general-purpose conditional attribute value helper, similar to
    `class_if` but for any attribute.

    Example:
        vb.add_dyn_text_attr("title", attr_if(has_tooltip, "Click me"))

    Args:
        condition: Whether to include the value.
        value: The attribute value when condition is True.

    Returns:
        The value or an empty string.
    """
    if condition:
        return value
    return String("")


fn attr_when(
    condition: Bool, true_value: String, false_value: String
) -> String:
    """Return one of two attribute values based on a condition.

    General-purpose binary attribute switching.

    Example:
        vb.add_dyn_text_attr("aria-expanded", attr_when(is_open, "true", "false"))

    Args:
        condition: The boolean condition.
        true_value: Value when True.
        false_value: Value when False.

    Returns:
        The appropriate value.
    """
    if condition:
        return true_value
    return false_value


# ── Inline event handler constructors ────────────────────────────────────────
#
# These create NODE_EVENT nodes that carry handler metadata (action type,
# signal key, operand).  They are processed by ComponentContext.register_view()
# which auto-assigns dynamic attribute indices and registers handlers.
#
# Import action tags from events.registry for the action constants.
from events.registry import (
    ACTION_SIGNAL_ADD_I32,
    ACTION_SIGNAL_SUB_I32,
    ACTION_SIGNAL_SET_I32,
    ACTION_SIGNAL_TOGGLE,
    ACTION_SIGNAL_SET_INPUT,
    ACTION_SIGNAL_SET_STRING,
    ACTION_KEY_ENTER_CUSTOM,
    ACTION_CUSTOM,
)
from signals.handle import SignalI32, SignalBool, SignalString


fn onclick_add(signal: SignalI32, delta: Int32) -> Node:
    """Create an inline click handler that adds `delta` to a signal.

    Equivalent to Dioxus: `onclick: move |_| signal += delta`

    Usage:
        el_button([text("Up high!"), onclick_add(count, 1)])
    """
    return Node.event_node(
        String("click"), ACTION_SIGNAL_ADD_I32, signal.key, delta
    )


fn onclick_sub(signal: SignalI32, delta: Int32) -> Node:
    """Create an inline click handler that subtracts `delta` from a signal.

    Equivalent to Dioxus: `onclick: move |_| signal -= delta`

    Usage:
        el_button([text("Down low!"), onclick_sub(count, 1)])
    """
    return Node.event_node(
        String("click"), ACTION_SIGNAL_SUB_I32, signal.key, delta
    )


fn onclick_set(signal: SignalI32, value: Int32) -> Node:
    """Create an inline click handler that sets a signal to a fixed value.

    Equivalent to Dioxus: `onclick: move |_| signal.set(value)`

    Usage:
        el_button([text("Reset"), onclick_set(count, 0)])
    """
    return Node.event_node(
        String("click"), ACTION_SIGNAL_SET_I32, signal.key, value
    )


fn onclick_toggle(signal: SignalI32) -> Node:
    """Create an inline click handler that toggles a boolean signal (0 ↔ 1).

    Equivalent to Dioxus: `onclick: move |_| signal.toggle()`

    Usage:
        el_button([text("Toggle"), onclick_toggle(flag)])
    """
    return Node.event_node(String("click"), ACTION_SIGNAL_TOGGLE, signal.key, 0)


fn onclick_toggle(signal: SignalBool) -> Node:
    """Create an inline click handler that toggles a boolean signal (0 ↔ 1).

    Overload accepting SignalBool directly (stored as Int32 internally).

    Equivalent to Dioxus: `onclick: move |_| signal.toggle()`

    Usage:
        el_button([text("Toggle"), onclick_toggle(show_detail)])
    """
    return Node.event_node(String("click"), ACTION_SIGNAL_TOGGLE, signal.key, 0)


fn onclick_custom() -> Node:
    """Create an inline click handler with custom action (app-defined logic).

    The handler is registered with ACTION_CUSTOM.  When dispatched, the
    runtime marks the scope dirty and returns False — the app's event
    handler then performs custom routing based on the handler ID.

    Use `ctx.view_event_handler_id(index)` after `register_view()` /
    `setup_view()` to retrieve the auto-registered handler ID.

    Equivalent to manual registration:
        var hid = ctx.register_handler(HandlerEntry.custom(scope_id, "click"))

    Usage:
        el_button(text("Add"), onclick_custom())

    Returns:
        A NODE_EVENT node for "click" with ACTION_CUSTOM.
    """
    return Node.event_node(String("click"), ACTION_CUSTOM, 0, 0)


fn onkeydown_enter_custom() -> Node:
    """Create an inline keydown handler that fires only on Enter key.

    Phase 22: Enables WASM-driven Enter key handling.  The handler is
    registered with ACTION_KEY_ENTER_CUSTOM.  When dispatched via
    `dispatch_event_with_string()`, the runtime checks the string
    payload (the key name) — only "Enter" triggers the action, all
    other keys are silently ignored.

    When "Enter" is detected, the runtime marks the scope dirty
    (same as ACTION_CUSTOM).  The app's `handle_event()` then performs
    custom routing based on the handler ID.

    Use `ctx.view_event_handler_id(index)` after `register_view()` /
    `setup_view()` to retrieve the auto-registered handler ID.

    Usage:
        el_input(
            oninput_set_string(text),
            onkeydown_enter_custom(),  # fires Add on Enter
        )

    Returns:
        A NODE_EVENT node for "keydown" with ACTION_KEY_ENTER_CUSTOM.
    """
    return Node.event_node(String("keydown"), ACTION_KEY_ENTER_CUSTOM, 0, 0)


fn on_event(
    event_name: String, signal: SignalI32, action: UInt8, operand: Int32
) -> Node:
    """Create an inline event handler for any event type and action.

    This is the generic form — use the convenience helpers (onclick_add,
    onclick_sub, etc.) for common patterns.

    Args:
        event_name: DOM event name (e.g. "click", "input", "change").
        signal: The signal to modify.
        action: Handler action tag (ACTION_SIGNAL_ADD_I32, etc.).
        operand: The operand value for the action.

    Usage:
        el_input([on_event("input", text_sig, ACTION_SIGNAL_SET_INPUT, 0)])
    """
    return Node.event_node(event_name, action, signal.key, operand)


# ── Inline string event handler constructors (Phase 20 — M20.3) ─────────────
#
# These create NODE_EVENT nodes for SignalString binding on input/change events.
# Processed by ComponentContext.register_view() which registers handlers with
# ACTION_SIGNAL_SET_STRING, storing string_key in signal_key and version_key
# in operand — exactly matching HandlerEntry.signal_set_string().


fn oninput_set_string(signal: SignalString) -> Node:
    """Create an inline input handler that sets a SignalString from the input value.

    Equivalent to Dioxus: `oninput: move |e| signal.set(e.value())`

    The handler stores both the string_key and version_key so that
    dispatch_event_with_string() can call write_signal_string().

    Usage:
        el_input(oninput_set_string(name))
        el_input(bind_value(name), oninput_set_string(name))  # two-way binding

    Args:
        signal: The SignalString to update from input events.

    Returns:
        A NODE_EVENT node for "input" with ACTION_SIGNAL_SET_STRING.
    """
    return Node.event_node(
        String("input"),
        ACTION_SIGNAL_SET_STRING,
        signal.string_key,
        Int32(signal.version_key),
    )


fn onchange_set_string(signal: SignalString) -> Node:
    """Create an inline change handler that sets a SignalString from the input value.

    Like `oninput_set_string` but fires on the "change" event (when the
    input loses focus or the user presses Enter), not on every keystroke.

    Equivalent to Dioxus: `onchange: move |e| signal.set(e.value())`

    Usage:
        el_input(onchange_set_string(name))
        el_select(onchange_set_string(selected))

    Args:
        signal: The SignalString to update from change events.

    Returns:
        A NODE_EVENT node for "change" with ACTION_SIGNAL_SET_STRING.
    """
    return Node.event_node(
        String("change"),
        ACTION_SIGNAL_SET_STRING,
        signal.string_key,
        Int32(signal.version_key),
    )


# ── Value binding constructors (Phase 20 — M20.4) ───────────────────────────
#
# These create NODE_BIND_VALUE nodes that carry a SignalString reference.
# Processed by ComponentContext.register_view() which auto-populates the
# dynamic attribute at render time by reading the signal's current value.
#
# NODE_BIND_VALUE fields:
#   text         → attribute name (e.g. "value", "checked")
#   dynamic_index → string_key (SignalString's StringStore key)
#   operand      → Int32(version_key) (companion version signal key)


fn bind_value(signal: SignalString) -> Node:
    """Create a value binding that syncs an input's value to a SignalString.

    Produces a NODE_BIND_VALUE node with attr_name="value".  When used
    with `register_view()` / `setup_view()`, the binding is auto-populated
    at render time — `RenderBuilder.build()` reads the signal and emits
    a dynamic "value" attribute.

    For two-way binding, combine with `oninput_set_string()`:

        el_input(
            attr("type", "text"),
            bind_value(input_text),          # M20.4: value → signal
            oninput_set_string(input_text),   # M20.3: signal ← input
        )

    Equivalent Dioxus pattern:
        input { value: "{text}", oninput: move |e| text.set(e.value()) }

    Args:
        signal: The SignalString whose value drives the attribute.

    Returns:
        A NODE_BIND_VALUE node for the "value" attribute.
    """
    return Node(
        kind=NODE_BIND_VALUE,
        tag=0,
        text=String("value"),
        attr_value=String(""),
        dynamic_index=signal.string_key,
        operand=Int32(signal.version_key),
        items=List[Node](),
    )


fn bind_attr(attr_name: String, signal: SignalString) -> Node:
    """Create a value binding for an arbitrary attribute name.

    Like `bind_value()` but lets you specify the attribute name.
    Useful for binding to attributes other than "value" (e.g. "placeholder").

    Usage:
        el_input(bind_attr("placeholder", hint_signal))

    Args:
        attr_name: The HTML attribute name to bind.
        signal: The SignalString whose value drives the attribute.

    Returns:
        A NODE_BIND_VALUE node for the specified attribute.
    """
    return Node(
        kind=NODE_BIND_VALUE,
        tag=0,
        text=attr_name,
        attr_value=String(""),
        dynamic_index=signal.string_key,
        operand=Int32(signal.version_key),
        items=List[Node](),
    )


# ── Generic element constructor ──────────────────────────────────────────────


fn el(html_tag: UInt8, var items: List[Node]) -> Node:
    """Create an element node with the given HTML tag and items.

    Items can be a mix of children and attributes in any order.
    The `to_template()` function sorts them appropriately.

    Usage: `el(TAG_DIV, [text("hello"), attr("class", "x")])`
    """
    return Node.element_node(html_tag, items^)


fn el_empty(html_tag: UInt8) -> Node:
    """Create an empty element node (no children, no attributes).

    Usage: `el_empty(TAG_BR)`
    """
    return Node.element_node_empty(html_tag)


# ── Tag helpers — Layout / Sectioning ────────────────────────────────────────


fn el_div(var items: List[Node]) -> Node:
    """Create a `<div>` element."""
    return Node.element_node(TAG_DIV, items^)


fn el_div() -> Node:
    """Create an empty `<div>` element."""
    return Node.element_node_empty(TAG_DIV)


fn el_span(var items: List[Node]) -> Node:
    """Create a `<span>` element."""
    return Node.element_node(TAG_SPAN, items^)


fn el_span() -> Node:
    """Create an empty `<span>` element."""
    return Node.element_node_empty(TAG_SPAN)


fn el_p(var items: List[Node]) -> Node:
    """Create a `<p>` element."""
    return Node.element_node(TAG_P, items^)


fn el_p() -> Node:
    """Create an empty `<p>` element."""
    return Node.element_node_empty(TAG_P)


fn el_section(var items: List[Node]) -> Node:
    """Create a `<section>` element."""
    return Node.element_node(TAG_SECTION, items^)


fn el_section() -> Node:
    """Create an empty `<section>` element."""
    return Node.element_node_empty(TAG_SECTION)


fn el_header(var items: List[Node]) -> Node:
    """Create a `<header>` element."""
    return Node.element_node(TAG_HEADER, items^)


fn el_header() -> Node:
    """Create an empty `<header>` element."""
    return Node.element_node_empty(TAG_HEADER)


fn el_footer(var items: List[Node]) -> Node:
    """Create a `<footer>` element."""
    return Node.element_node(TAG_FOOTER, items^)


fn el_footer() -> Node:
    """Create an empty `<footer>` element."""
    return Node.element_node_empty(TAG_FOOTER)


fn el_nav(var items: List[Node]) -> Node:
    """Create a `<nav>` element."""
    return Node.element_node(TAG_NAV, items^)


fn el_nav() -> Node:
    """Create an empty `<nav>` element."""
    return Node.element_node_empty(TAG_NAV)


fn el_main(var items: List[Node]) -> Node:
    """Create a `<main>` element.

    Named `el_main` (not `main_`) to follow the `el_` prefix convention.
    """
    return Node.element_node(TAG_MAIN, items^)


fn el_main() -> Node:
    """Create an empty `<main>` element."""
    return Node.element_node_empty(TAG_MAIN)


fn el_article(var items: List[Node]) -> Node:
    """Create an `<article>` element."""
    return Node.element_node(TAG_ARTICLE, items^)


fn el_article() -> Node:
    """Create an empty `<article>` element."""
    return Node.element_node_empty(TAG_ARTICLE)


fn el_aside(var items: List[Node]) -> Node:
    """Create an `<aside>` element."""
    return Node.element_node(TAG_ASIDE, items^)


fn el_aside() -> Node:
    """Create an empty `<aside>` element."""
    return Node.element_node_empty(TAG_ASIDE)


# ── Tag helpers — Headings ───────────────────────────────────────────────────


fn el_h1(var items: List[Node]) -> Node:
    """Create an `<h1>` element."""
    return Node.element_node(TAG_H1, items^)


fn el_h1() -> Node:
    """Create an empty `<h1>` element."""
    return Node.element_node_empty(TAG_H1)


fn el_h2(var items: List[Node]) -> Node:
    """Create an `<h2>` element."""
    return Node.element_node(TAG_H2, items^)


fn el_h2() -> Node:
    """Create an empty `<h2>` element."""
    return Node.element_node_empty(TAG_H2)


fn el_h3(var items: List[Node]) -> Node:
    """Create an `<h3>` element."""
    return Node.element_node(TAG_H3, items^)


fn el_h3() -> Node:
    """Create an empty `<h3>` element."""
    return Node.element_node_empty(TAG_H3)


fn el_h4(var items: List[Node]) -> Node:
    """Create an `<h4>` element."""
    return Node.element_node(TAG_H4, items^)


fn el_h4() -> Node:
    """Create an empty `<h4>` element."""
    return Node.element_node_empty(TAG_H4)


fn el_h5(var items: List[Node]) -> Node:
    """Create an `<h5>` element."""
    return Node.element_node(TAG_H5, items^)


fn el_h5() -> Node:
    """Create an empty `<h5>` element."""
    return Node.element_node_empty(TAG_H5)


fn el_h6(var items: List[Node]) -> Node:
    """Create an `<h6>` element."""
    return Node.element_node(TAG_H6, items^)


fn el_h6() -> Node:
    """Create an empty `<h6>` element."""
    return Node.element_node_empty(TAG_H6)


# ── Tag helpers — Lists ──────────────────────────────────────────────────────


fn el_ul(var items: List[Node]) -> Node:
    """Create a `<ul>` element."""
    return Node.element_node(TAG_UL, items^)


fn el_ul() -> Node:
    """Create an empty `<ul>` element."""
    return Node.element_node_empty(TAG_UL)


fn el_ol(var items: List[Node]) -> Node:
    """Create an `<ol>` element."""
    return Node.element_node(TAG_OL, items^)


fn el_ol() -> Node:
    """Create an empty `<ol>` element."""
    return Node.element_node_empty(TAG_OL)


fn el_li(var items: List[Node]) -> Node:
    """Create a `<li>` element."""
    return Node.element_node(TAG_LI, items^)


fn el_li() -> Node:
    """Create an empty `<li>` element."""
    return Node.element_node_empty(TAG_LI)


# ── Tag helpers — Interactive ────────────────────────────────────────────────


fn el_button(var items: List[Node]) -> Node:
    """Create a `<button>` element."""
    return Node.element_node(TAG_BUTTON, items^)


fn el_button() -> Node:
    """Create an empty `<button>` element."""
    return Node.element_node_empty(TAG_BUTTON)


fn el_input(var items: List[Node]) -> Node:
    """Create an `<input>` element."""
    return Node.element_node(TAG_INPUT, items^)


fn el_input() -> Node:
    """Create an empty `<input>` element."""
    return Node.element_node_empty(TAG_INPUT)


fn el_form(var items: List[Node]) -> Node:
    """Create a `<form>` element."""
    return Node.element_node(TAG_FORM, items^)


fn el_form() -> Node:
    """Create an empty `<form>` element."""
    return Node.element_node_empty(TAG_FORM)


fn el_textarea(var items: List[Node]) -> Node:
    """Create a `<textarea>` element."""
    return Node.element_node(TAG_TEXTAREA, items^)


fn el_textarea() -> Node:
    """Create an empty `<textarea>` element."""
    return Node.element_node_empty(TAG_TEXTAREA)


fn el_select(var items: List[Node]) -> Node:
    """Create a `<select>` element."""
    return Node.element_node(TAG_SELECT, items^)


fn el_select() -> Node:
    """Create an empty `<select>` element."""
    return Node.element_node_empty(TAG_SELECT)


fn el_option(var items: List[Node]) -> Node:
    """Create an `<option>` element."""
    return Node.element_node(TAG_OPTION, items^)


fn el_option() -> Node:
    """Create an empty `<option>` element."""
    return Node.element_node_empty(TAG_OPTION)


fn el_label(var items: List[Node]) -> Node:
    """Create a `<label>` element."""
    return Node.element_node(TAG_LABEL, items^)


fn el_label() -> Node:
    """Create an empty `<label>` element."""
    return Node.element_node_empty(TAG_LABEL)


# ── Tag helpers — Links / Media ──────────────────────────────────────────────


fn el_a(var items: List[Node]) -> Node:
    """Create an `<a>` element."""
    return Node.element_node(TAG_A, items^)


fn el_a() -> Node:
    """Create an empty `<a>` element."""
    return Node.element_node_empty(TAG_A)


fn el_img(var items: List[Node]) -> Node:
    """Create an `<img>` element."""
    return Node.element_node(TAG_IMG, items^)


fn el_img() -> Node:
    """Create an empty `<img>` element."""
    return Node.element_node_empty(TAG_IMG)


# ── Tag helpers — Table ──────────────────────────────────────────────────────


fn el_table(var items: List[Node]) -> Node:
    """Create a `<table>` element."""
    return Node.element_node(TAG_TABLE, items^)


fn el_table() -> Node:
    """Create an empty `<table>` element."""
    return Node.element_node_empty(TAG_TABLE)


fn el_thead(var items: List[Node]) -> Node:
    """Create a `<thead>` element."""
    return Node.element_node(TAG_THEAD, items^)


fn el_thead() -> Node:
    """Create an empty `<thead>` element."""
    return Node.element_node_empty(TAG_THEAD)


fn el_tbody(var items: List[Node]) -> Node:
    """Create a `<tbody>` element."""
    return Node.element_node(TAG_TBODY, items^)


fn el_tbody() -> Node:
    """Create an empty `<tbody>` element."""
    return Node.element_node_empty(TAG_TBODY)


fn el_tr(var items: List[Node]) -> Node:
    """Create a `<tr>` element."""
    return Node.element_node(TAG_TR, items^)


fn el_tr() -> Node:
    """Create an empty `<tr>` element."""
    return Node.element_node_empty(TAG_TR)


fn el_td(var items: List[Node]) -> Node:
    """Create a `<td>` element."""
    return Node.element_node(TAG_TD, items^)


fn el_td() -> Node:
    """Create an empty `<td>` element."""
    return Node.element_node_empty(TAG_TD)


fn el_th(var items: List[Node]) -> Node:
    """Create a `<th>` element."""
    return Node.element_node(TAG_TH, items^)


fn el_th() -> Node:
    """Create an empty `<th>` element."""
    return Node.element_node_empty(TAG_TH)


# ── Tag helpers — Inline / Formatting ────────────────────────────────────────


fn el_strong(var items: List[Node]) -> Node:
    """Create a `<strong>` element."""
    return Node.element_node(TAG_STRONG, items^)


fn el_strong() -> Node:
    """Create an empty `<strong>` element."""
    return Node.element_node_empty(TAG_STRONG)


fn el_em(var items: List[Node]) -> Node:
    """Create an `<em>` element."""
    return Node.element_node(TAG_EM, items^)


fn el_em() -> Node:
    """Create an empty `<em>` element."""
    return Node.element_node_empty(TAG_EM)


fn el_br() -> Node:
    """Create a `<br>` element (void element, no children)."""
    return Node.element_node_empty(TAG_BR)


fn el_hr() -> Node:
    """Create an `<hr>` element (void element, no children)."""
    return Node.element_node_empty(TAG_HR)


fn el_pre(var items: List[Node]) -> Node:
    """Create a `<pre>` element."""
    return Node.element_node(TAG_PRE, items^)


fn el_pre() -> Node:
    """Create an empty `<pre>` element."""
    return Node.element_node_empty(TAG_PRE)


fn el_code(var items: List[Node]) -> Node:
    """Create a `<code>` element."""
    return Node.element_node(TAG_CODE, items^)


fn el_code() -> Node:
    """Create an empty `<code>` element."""
    return Node.element_node_empty(TAG_CODE)


# ══════════════════════════════════════════════════════════════════════════════
# Multi-arg el_* overloads — Dioxus-style ergonomic element construction
# ══════════════════════════════════════════════════════════════════════════════
#
# These overloads eliminate the need for `[...]` list literal wrappers,
# bringing the DSL much closer to Dioxus's `rsx!` macro syntax.
#
# Instead of:
#
#     el_div([
#         el_h1([dyn_text()]),
#         el_button([text("Up high!"), onclick_add(count, 1)]),
#         el_button([text("Down low!"), onclick_sub(count, 1)]),
#     ])
#
# Developers write:
#
#     el_div(
#         el_h1(dyn_text()),
#         el_button(text("Up high!"), onclick_add(count, 1)),
#         el_button(text("Down low!"), onclick_sub(count, 1)),
#     )
#
# Overloads are provided for 1–5 Node arguments per element.  For more
# than 5 children, use the `[...]` list literal form.


# ── Layout / Sectioning — multi-arg overloads ────────────────────────────────


fn el_div(var a: Node) -> Node:
    """Create a `<div>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_DIV, items^)


fn el_div(var a: Node, var b: Node) -> Node:
    """Create a `<div>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_DIV, items^)


fn el_div(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<div>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_DIV, items^)


fn el_div(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<div>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_DIV, items^)


fn el_div(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<div>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_DIV, items^)


fn el_span(var a: Node) -> Node:
    """Create a `<span>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_SPAN, items^)


fn el_span(var a: Node, var b: Node) -> Node:
    """Create a `<span>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_SPAN, items^)


fn el_span(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<span>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_SPAN, items^)


fn el_span(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<span>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_SPAN, items^)


fn el_span(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<span>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_SPAN, items^)


fn el_p(var a: Node) -> Node:
    """Create a `<p>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_P, items^)


fn el_p(var a: Node, var b: Node) -> Node:
    """Create a `<p>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_P, items^)


fn el_p(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<p>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_P, items^)


fn el_p(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<p>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_P, items^)


fn el_p(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<p>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_P, items^)


fn el_section(var a: Node) -> Node:
    """Create a `<section>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_SECTION, items^)


fn el_section(var a: Node, var b: Node) -> Node:
    """Create a `<section>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_SECTION, items^)


fn el_section(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<section>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_SECTION, items^)


fn el_section(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<section>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_SECTION, items^)


fn el_section(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<section>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_SECTION, items^)


fn el_header(var a: Node) -> Node:
    """Create a `<header>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_HEADER, items^)


fn el_header(var a: Node, var b: Node) -> Node:
    """Create a `<header>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_HEADER, items^)


fn el_header(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<header>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_HEADER, items^)


fn el_header(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<header>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_HEADER, items^)


fn el_header(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<header>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_HEADER, items^)


fn el_footer(var a: Node) -> Node:
    """Create a `<footer>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_FOOTER, items^)


fn el_footer(var a: Node, var b: Node) -> Node:
    """Create a `<footer>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_FOOTER, items^)


fn el_footer(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<footer>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_FOOTER, items^)


fn el_footer(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<footer>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_FOOTER, items^)


fn el_footer(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<footer>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_FOOTER, items^)


fn el_nav(var a: Node) -> Node:
    """Create a `<nav>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_NAV, items^)


fn el_nav(var a: Node, var b: Node) -> Node:
    """Create a `<nav>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_NAV, items^)


fn el_nav(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<nav>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_NAV, items^)


fn el_nav(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<nav>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_NAV, items^)


fn el_nav(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<nav>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_NAV, items^)


fn el_main(var a: Node) -> Node:
    """Create a `<main>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_MAIN, items^)


fn el_main(var a: Node, var b: Node) -> Node:
    """Create a `<main>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_MAIN, items^)


fn el_main(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<main>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_MAIN, items^)


fn el_main(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<main>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_MAIN, items^)


fn el_main(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<main>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_MAIN, items^)


fn el_article(var a: Node) -> Node:
    """Create an `<article>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_ARTICLE, items^)


fn el_article(var a: Node, var b: Node) -> Node:
    """Create an `<article>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_ARTICLE, items^)


fn el_article(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<article>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_ARTICLE, items^)


fn el_article(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<article>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_ARTICLE, items^)


fn el_article(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<article>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_ARTICLE, items^)


fn el_aside(var a: Node) -> Node:
    """Create an `<aside>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_ASIDE, items^)


fn el_aside(var a: Node, var b: Node) -> Node:
    """Create an `<aside>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_ASIDE, items^)


fn el_aside(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<aside>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_ASIDE, items^)


fn el_aside(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<aside>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_ASIDE, items^)


fn el_aside(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<aside>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_ASIDE, items^)


# ── Headings — multi-arg overloads ───────────────────────────────────────────


fn el_h1(var a: Node) -> Node:
    """Create an `<h1>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_H1, items^)


fn el_h1(var a: Node, var b: Node) -> Node:
    """Create an `<h1>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_H1, items^)


fn el_h1(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<h1>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_H1, items^)


fn el_h1(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<h1>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_H1, items^)


fn el_h1(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<h1>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_H1, items^)


fn el_h2(var a: Node) -> Node:
    """Create an `<h2>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_H2, items^)


fn el_h2(var a: Node, var b: Node) -> Node:
    """Create an `<h2>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_H2, items^)


fn el_h2(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<h2>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_H2, items^)


fn el_h2(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<h2>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_H2, items^)


fn el_h2(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<h2>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_H2, items^)


fn el_h3(var a: Node) -> Node:
    """Create an `<h3>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_H3, items^)


fn el_h3(var a: Node, var b: Node) -> Node:
    """Create an `<h3>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_H3, items^)


fn el_h3(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<h3>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_H3, items^)


fn el_h3(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<h3>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_H3, items^)


fn el_h3(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<h3>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_H3, items^)


fn el_h4(var a: Node) -> Node:
    """Create an `<h4>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_H4, items^)


fn el_h4(var a: Node, var b: Node) -> Node:
    """Create an `<h4>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_H4, items^)


fn el_h4(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<h4>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_H4, items^)


fn el_h4(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<h4>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_H4, items^)


fn el_h4(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<h4>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_H4, items^)


fn el_h5(var a: Node) -> Node:
    """Create an `<h5>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_H5, items^)


fn el_h5(var a: Node, var b: Node) -> Node:
    """Create an `<h5>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_H5, items^)


fn el_h5(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<h5>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_H5, items^)


fn el_h5(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<h5>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_H5, items^)


fn el_h5(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<h5>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_H5, items^)


fn el_h6(var a: Node) -> Node:
    """Create an `<h6>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_H6, items^)


fn el_h6(var a: Node, var b: Node) -> Node:
    """Create an `<h6>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_H6, items^)


fn el_h6(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<h6>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_H6, items^)


fn el_h6(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<h6>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_H6, items^)


fn el_h6(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<h6>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_H6, items^)


# ── Lists — multi-arg overloads ──────────────────────────────────────────────


fn el_ul(var a: Node) -> Node:
    """Create a `<ul>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_UL, items^)


fn el_ul(var a: Node, var b: Node) -> Node:
    """Create a `<ul>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_UL, items^)


fn el_ul(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<ul>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_UL, items^)


fn el_ul(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<ul>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_UL, items^)


fn el_ul(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<ul>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_UL, items^)


fn el_ol(var a: Node) -> Node:
    """Create an `<ol>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_OL, items^)


fn el_ol(var a: Node, var b: Node) -> Node:
    """Create an `<ol>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_OL, items^)


fn el_ol(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<ol>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_OL, items^)


fn el_ol(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<ol>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_OL, items^)


fn el_ol(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<ol>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_OL, items^)


fn el_li(var a: Node) -> Node:
    """Create a `<li>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_LI, items^)


fn el_li(var a: Node, var b: Node) -> Node:
    """Create a `<li>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_LI, items^)


fn el_li(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<li>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_LI, items^)


fn el_li(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<li>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_LI, items^)


fn el_li(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<li>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_LI, items^)


# ── Interactive — multi-arg overloads ────────────────────────────────────────


fn el_button(var a: Node) -> Node:
    """Create a `<button>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_BUTTON, items^)


fn el_button(var a: Node, var b: Node) -> Node:
    """Create a `<button>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_BUTTON, items^)


fn el_button(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<button>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_BUTTON, items^)


fn el_button(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<button>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_BUTTON, items^)


fn el_button(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<button>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_BUTTON, items^)


fn el_input(var a: Node) -> Node:
    """Create an `<input>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_INPUT, items^)


fn el_input(var a: Node, var b: Node) -> Node:
    """Create an `<input>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_INPUT, items^)


fn el_input(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<input>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_INPUT, items^)


fn el_input(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<input>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_INPUT, items^)


fn el_input(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<input>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_INPUT, items^)


fn el_form(var a: Node) -> Node:
    """Create a `<form>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_FORM, items^)


fn el_form(var a: Node, var b: Node) -> Node:
    """Create a `<form>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_FORM, items^)


fn el_form(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<form>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_FORM, items^)


fn el_form(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<form>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_FORM, items^)


fn el_form(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<form>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_FORM, items^)


fn el_textarea(var a: Node) -> Node:
    """Create a `<textarea>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_TEXTAREA, items^)


fn el_textarea(var a: Node, var b: Node) -> Node:
    """Create a `<textarea>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_TEXTAREA, items^)


fn el_textarea(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<textarea>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_TEXTAREA, items^)


fn el_textarea(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<textarea>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_TEXTAREA, items^)


fn el_textarea(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<textarea>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_TEXTAREA, items^)


fn el_select(var a: Node) -> Node:
    """Create a `<select>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_SELECT, items^)


fn el_select(var a: Node, var b: Node) -> Node:
    """Create a `<select>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_SELECT, items^)


fn el_select(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<select>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_SELECT, items^)


fn el_select(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<select>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_SELECT, items^)


fn el_select(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<select>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_SELECT, items^)


fn el_option(var a: Node) -> Node:
    """Create an `<option>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_OPTION, items^)


fn el_option(var a: Node, var b: Node) -> Node:
    """Create an `<option>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_OPTION, items^)


fn el_option(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<option>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_OPTION, items^)


fn el_option(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<option>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_OPTION, items^)


fn el_option(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<option>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_OPTION, items^)


fn el_label(var a: Node) -> Node:
    """Create a `<label>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_LABEL, items^)


fn el_label(var a: Node, var b: Node) -> Node:
    """Create a `<label>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_LABEL, items^)


fn el_label(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<label>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_LABEL, items^)


fn el_label(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<label>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_LABEL, items^)


fn el_label(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<label>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_LABEL, items^)


# ── Links / Media — multi-arg overloads ──────────────────────────────────────


fn el_a(var a: Node) -> Node:
    """Create an `<a>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_A, items^)


fn el_a(var a: Node, var b: Node) -> Node:
    """Create an `<a>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_A, items^)


fn el_a(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<a>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_A, items^)


fn el_a(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<a>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_A, items^)


fn el_a(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<a>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_A, items^)


fn el_img(var a: Node) -> Node:
    """Create an `<img>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_IMG, items^)


fn el_img(var a: Node, var b: Node) -> Node:
    """Create an `<img>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_IMG, items^)


fn el_img(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<img>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_IMG, items^)


fn el_img(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<img>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_IMG, items^)


fn el_img(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<img>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_IMG, items^)


# ── Table — multi-arg overloads ──────────────────────────────────────────────


fn el_table(var a: Node) -> Node:
    """Create a `<table>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_TABLE, items^)


fn el_table(var a: Node, var b: Node) -> Node:
    """Create a `<table>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_TABLE, items^)


fn el_table(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<table>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_TABLE, items^)


fn el_table(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<table>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_TABLE, items^)


fn el_table(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<table>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_TABLE, items^)


fn el_thead(var a: Node) -> Node:
    """Create a `<thead>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_THEAD, items^)


fn el_thead(var a: Node, var b: Node) -> Node:
    """Create a `<thead>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_THEAD, items^)


fn el_thead(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<thead>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_THEAD, items^)


fn el_thead(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<thead>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_THEAD, items^)


fn el_thead(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<thead>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_THEAD, items^)


fn el_tbody(var a: Node) -> Node:
    """Create a `<tbody>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_TBODY, items^)


fn el_tbody(var a: Node, var b: Node) -> Node:
    """Create a `<tbody>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_TBODY, items^)


fn el_tbody(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<tbody>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_TBODY, items^)


fn el_tbody(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<tbody>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_TBODY, items^)


fn el_tbody(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<tbody>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_TBODY, items^)


fn el_tr(var a: Node) -> Node:
    """Create a `<tr>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_TR, items^)


fn el_tr(var a: Node, var b: Node) -> Node:
    """Create a `<tr>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_TR, items^)


fn el_tr(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<tr>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_TR, items^)


fn el_tr(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<tr>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_TR, items^)


fn el_tr(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<tr>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_TR, items^)


fn el_td(var a: Node) -> Node:
    """Create a `<td>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_TD, items^)


fn el_td(var a: Node, var b: Node) -> Node:
    """Create a `<td>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_TD, items^)


fn el_td(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<td>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_TD, items^)


fn el_td(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<td>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_TD, items^)


fn el_td(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<td>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_TD, items^)


fn el_th(var a: Node) -> Node:
    """Create a `<th>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_TH, items^)


fn el_th(var a: Node, var b: Node) -> Node:
    """Create a `<th>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_TH, items^)


fn el_th(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<th>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_TH, items^)


fn el_th(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<th>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_TH, items^)


fn el_th(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<th>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_TH, items^)


# ── Inline / Formatting — multi-arg overloads ────────────────────────────────


fn el_strong(var a: Node) -> Node:
    """Create a `<strong>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_STRONG, items^)


fn el_strong(var a: Node, var b: Node) -> Node:
    """Create a `<strong>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_STRONG, items^)


fn el_strong(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<strong>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_STRONG, items^)


fn el_strong(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<strong>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_STRONG, items^)


fn el_strong(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<strong>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_STRONG, items^)


fn el_em(var a: Node) -> Node:
    """Create an `<em>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_EM, items^)


fn el_em(var a: Node, var b: Node) -> Node:
    """Create an `<em>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_EM, items^)


fn el_em(var a: Node, var b: Node, var c: Node) -> Node:
    """Create an `<em>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_EM, items^)


fn el_em(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create an `<em>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_EM, items^)


fn el_em(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create an `<em>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_EM, items^)


fn el_pre(var a: Node) -> Node:
    """Create a `<pre>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_PRE, items^)


fn el_pre(var a: Node, var b: Node) -> Node:
    """Create a `<pre>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_PRE, items^)


fn el_pre(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<pre>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_PRE, items^)


fn el_pre(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<pre>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_PRE, items^)


fn el_pre(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<pre>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_PRE, items^)


fn el_code(var a: Node) -> Node:
    """Create a `<code>` with 1 child/attr."""
    var items = List[Node]()
    items.append(a^)
    return Node.element_node(TAG_CODE, items^)


fn el_code(var a: Node, var b: Node) -> Node:
    """Create a `<code>` with 2 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    return Node.element_node(TAG_CODE, items^)


fn el_code(var a: Node, var b: Node, var c: Node) -> Node:
    """Create a `<code>` with 3 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    return Node.element_node(TAG_CODE, items^)


fn el_code(var a: Node, var b: Node, var c: Node, var d: Node) -> Node:
    """Create a `<code>` with 4 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    return Node.element_node(TAG_CODE, items^)


fn el_code(
    var a: Node, var b: Node, var c: Node, var d: Node, var e: Node
) -> Node:
    """Create a `<code>` with 5 children/attrs."""
    var items = List[Node]()
    items.append(a^)
    items.append(b^)
    items.append(c^)
    items.append(d^)
    items.append(e^)
    return Node.element_node(TAG_CODE, items^)


# ══════════════════════════════════════════════════════════════════════════════
# to_template — Convert a Node tree to a Template
# ══════════════════════════════════════════════════════════════════════════════


fn to_template(node: Node, name: String) -> Template:
    """Convert a Node tree into a Template.

    The root node should typically be an ELEMENT node (e.g. from `el_div`).
    Text-only roots and dynamic roots are also supported.

    Attributes within ELEMENT nodes are separated from children and added
    to the template's attribute system.  Static attributes are added first,
    then dynamic attributes, then child nodes — matching the convention
    expected by the diff engine.

    Args:
        node: The root Node of the element tree.
        name: The template name (for registry deduplication).

    Returns:
        A fully-constructed Template ready for registration.
    """
    var builder = TemplateBuilder(name)
    _build_node(builder, node, -1)
    return builder.build()


fn to_template_multi(roots: List[Node], name: String) -> Template:
    """Convert multiple root Nodes into a single Template.

    Used for templates that have more than one root node (e.g. a fragment
    of adjacent elements without a wrapper).

    Args:
        roots: The root Node list.
        name: The template name.

    Returns:
        A fully-constructed Template ready for registration.
    """
    var builder = TemplateBuilder(name)
    for i in range(len(roots)):
        _build_node(builder, roots[i], -1)
    return builder.build()


fn _build_node(mut builder: TemplateBuilder, node: Node, parent: Int):
    """Recursively walk a Node tree and emit TemplateBuilder calls.

    For ELEMENT nodes:
      1. Push the element itself.
      2. Add static attributes (pass 1).
      3. Add dynamic attributes (pass 2).
      4. Recurse into child nodes (pass 3).

    The three-pass approach keeps attributes grouped before children,
    matching the layout expected by the diff/create engines.
    """
    if node.kind == NODE_TEXT:
        _ = builder.push_text(node.text, parent)

    elif node.kind == NODE_ELEMENT:
        var idx = builder.push_element(node.tag, parent)

        # Pass 1: static attributes
        for i in range(len(node.items)):
            if node.items[i].kind == NODE_STATIC_ATTR:
                builder.push_static_attr(
                    idx, node.items[i].text, node.items[i].attr_value
                )

        # Pass 2: dynamic attributes (explicit dyn_attr + event + bind_value nodes)
        for i in range(len(node.items)):
            if (
                node.items[i].kind == NODE_DYN_ATTR
                or node.items[i].kind == NODE_EVENT
                or node.items[i].kind == NODE_BIND_VALUE
            ):
                builder.push_dynamic_attr(idx, node.items[i].dynamic_index)

        # Pass 3: child nodes (recurse)
        for i in range(len(node.items)):
            if node.items[i].is_child():
                _build_node(builder, node.items[i], idx)

    elif node.kind == NODE_DYN_TEXT:
        _ = builder.push_dynamic_text(node.dynamic_index, parent)

    elif node.kind == NODE_DYN_NODE:
        _ = builder.push_dynamic(node.dynamic_index, parent)

    elif node.kind == NODE_EVENT:
        # EVENT nodes in _build_node are treated as dynamic attrs.
        # Their dynamic_index is used as the attr slot index (set by
        # register_view's reindexing pass, or by the caller).
        if parent >= 0:
            builder.push_dynamic_attr(parent, node.dynamic_index)

    elif node.kind == NODE_BIND_VALUE:
        # BIND_VALUE nodes are treated as dynamic attrs (like EVENT).
        # Their dynamic_index is the attr slot index (set by
        # register_view's reindexing pass).
        if parent >= 0:
            builder.push_dynamic_attr(parent, node.dynamic_index)

    # NODE_STATIC_ATTR and NODE_DYN_ATTR at root level are silently
    # ignored (they only make sense inside an ELEMENT node's items).


# ══════════════════════════════════════════════════════════════════════════════
# VNodeBuilder — Ergonomic VNode construction
# ══════════════════════════════════════════════════════════════════════════════


struct VNodeBuilder(Movable):
    """Ergonomic builder for constructing VNode instances.

    Wraps a VNodeStore pointer and provides short methods for adding
    dynamic text, events, and attributes to a TemplateRef VNode.

    Usage:
        var vb = VNodeBuilder(template_id, store_ptr)
        vb.add_dyn_text("Count: 42")
        vb.add_dyn_event("click", handler_id)
        var idx = vb.index()
    """

    var _store: UnsafePointer[VNodeStore, MutExternalOrigin]
    var _vnode_idx: UInt32

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        template_id: UInt32,
        store: UnsafePointer[VNodeStore, MutExternalOrigin],
    ):
        """Create a new TemplateRef VNode in the store.

        Args:
            template_id: The registered template's ID.
            store: Pointer to the VNodeStore to push the VNode into.
        """
        self._store = store
        self._vnode_idx = store[0].push(VNode.template_ref(template_id))

    fn __init__(
        out self,
        template_id: UInt32,
        key: String,
        store: UnsafePointer[VNodeStore, MutExternalOrigin],
    ):
        """Create a new keyed TemplateRef VNode in the store.

        Args:
            template_id: The registered template's ID.
            key: The key string for keyed diffing.
            store: Pointer to the VNodeStore.
        """
        self._store = store
        self._vnode_idx = store[0].push(
            VNode.template_ref_keyed(template_id, key)
        )

    fn __moveinit__(out self, deinit other: Self):
        self._store = other._store
        self._vnode_idx = other._vnode_idx

    # ── Dynamic text nodes ───────────────────────────────────────────

    fn add_dyn_text(mut self, value: String):
        """Add a dynamic text node (fills the next DynamicText slot).

        Call in order corresponding to DYN_TEXT placeholders in the template
        (dyn_text(0), dyn_text(1), ...).
        """
        self._store[0].push_dynamic_node(
            self._vnode_idx, DynamicNode.text_node(value)
        )

    fn add_dyn_placeholder(mut self):
        """Add a dynamic placeholder node (fills the next Dynamic slot).

        Used for conditional content that is currently absent.
        """
        self._store[0].push_dynamic_node(
            self._vnode_idx, DynamicNode.placeholder()
        )

    # ── Dynamic attributes ───────────────────────────────────────────

    fn add_dyn_event(mut self, event_name: String, handler_id: UInt32):
        """Add a dynamic event handler attribute.

        Args:
            event_name: The event name (e.g. "click", "input").
            handler_id: The handler ID from the HandlerRegistry.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(
                event_name, AttributeValue.event(handler_id), UInt32(0)
            ),
        )

    fn add_dyn_event_on(
        mut self, event_name: String, handler_id: UInt32, element_id: UInt32
    ):
        """Add a dynamic event handler targeting a specific template element.

        Args:
            event_name: The event name.
            handler_id: The handler ID.
            element_id: The template element index this attr targets.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(
                event_name, AttributeValue.event(handler_id), element_id
            ),
        )

    fn add_dyn_text_attr(mut self, name: String, value: String):
        """Add a dynamic text attribute (e.g. class, id, href).

        Args:
            name: The attribute name.
            value: The attribute text value.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(name, AttributeValue.text(value), UInt32(0)),
        )

    fn add_dyn_text_attr_on(
        mut self, name: String, value: String, element_id: UInt32
    ):
        """Add a dynamic text attribute targeting a specific template element.

        Args:
            name: The attribute name.
            value: The attribute text value.
            element_id: The template element index this attr targets.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(name, AttributeValue.text(value), element_id),
        )

    fn add_dyn_int_attr(mut self, name: String, value: Int64):
        """Add a dynamic integer attribute.

        Args:
            name: The attribute name.
            value: The integer value.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(name, AttributeValue.integer(value), UInt32(0)),
        )

    fn add_dyn_float_attr(mut self, name: String, value: Float64):
        """Add a dynamic float attribute.

        Args:
            name: The attribute name.
            value: The float value.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(name, AttributeValue.floating(value), UInt32(0)),
        )

    fn add_dyn_bool_attr(mut self, name: String, value: Bool):
        """Add a dynamic boolean attribute (e.g. disabled, checked, hidden).

        When `value` is True, emits SetAttribute(name, "") — the presence
        of the attribute means "on" for HTML boolean attributes.

        When `value` is False, stores AVAL_NONE so the diff engine emits
        RemoveAttribute instead of SetAttribute(name, "") — correctly
        removing the attribute from the DOM rather than leaving an empty
        attribute present.

        Args:
            name: The attribute name.
            value: The boolean value.
        """
        if value:
            # Present: set attribute to empty string (HTML boolean convention)
            self._store[0].push_dynamic_attr(
                self._vnode_idx,
                DynamicAttr(name, AttributeValue.text(String("")), UInt32(0)),
            )
        else:
            # Absent: use AVAL_NONE → diff engine emits RemoveAttribute
            self._store[0].push_dynamic_attr(
                self._vnode_idx,
                DynamicAttr(name, AttributeValue.none(), UInt32(0)),
            )

    fn add_dyn_none_attr(mut self, name: String):
        """Add a dynamic none/removal attribute.

        Used to remove a previously-set attribute during diffing.

        Args:
            name: The attribute name to remove.
        """
        self._store[0].push_dynamic_attr(
            self._vnode_idx,
            DynamicAttr(name, AttributeValue.none(), UInt32(0)),
        )

    # ── Queries ──────────────────────────────────────────────────────

    fn index(self) -> UInt32:
        """Return the VNode's index in the VNodeStore."""
        return self._vnode_idx

    fn store(self) -> UnsafePointer[VNodeStore, MutExternalOrigin]:
        """Return the VNodeStore pointer."""
        return self._store


# ══════════════════════════════════════════════════════════════════════════════
# Utility helpers
# ══════════════════════════════════════════════════════════════════════════════


fn count_nodes(node: Node) -> Int:
    """Recursively count the total number of nodes in a tree.

    Includes the node itself and all descendants.  Attribute nodes
    inside elements are NOT counted (they are metadata, not tree nodes).
    """
    if node.kind == NODE_ELEMENT:
        var total = 1  # this element
        for i in range(len(node.items)):
            if node.items[i].is_child():
                total += count_nodes(node.items[i])
        return total
    elif node.is_attr():
        return 0  # attrs are not tree nodes
    else:
        return 1  # TEXT, DYN_TEXT, DYN_NODE


fn count_all_items(node: Node) -> Int:
    """Recursively count all items (children + attrs) in the tree.

    Includes every Node in the tree including attribute nodes.
    """
    if node.kind == NODE_ELEMENT:
        var total = 1  # this element
        for i in range(len(node.items)):
            total += count_all_items(node.items[i])
        return total
    else:
        return 1


fn count_dynamic_text_slots(node: Node) -> Int:
    """Count the total number of DYN_TEXT nodes in the tree."""
    if node.kind == NODE_DYN_TEXT:
        return 1
    elif node.kind == NODE_ELEMENT:
        var total = 0
        for i in range(len(node.items)):
            if node.items[i].is_child():
                total += count_dynamic_text_slots(node.items[i])
        return total
    else:
        return 0


fn count_dynamic_node_slots(node: Node) -> Int:
    """Count the total number of DYN_NODE nodes in the tree."""
    if node.kind == NODE_DYN_NODE:
        return 1
    elif node.kind == NODE_ELEMENT:
        var total = 0
        for i in range(len(node.items)):
            if node.items[i].is_child():
                total += count_dynamic_node_slots(node.items[i])
        return total
    else:
        return 0


fn count_dynamic_attr_slots(node: Node) -> Int:
    """Count the total number of DYN_ATTR, EVENT, and BIND_VALUE nodes in the tree.
    """
    if node.kind == NODE_ELEMENT:
        var total = 0
        for i in range(len(node.items)):
            if (
                node.items[i].kind == NODE_DYN_ATTR
                or node.items[i].kind == NODE_EVENT
                or node.items[i].kind == NODE_BIND_VALUE
            ):
                total += 1
            elif node.items[i].is_child():
                total += count_dynamic_attr_slots(node.items[i])
        return total
    else:
        return 0


fn count_static_attr_nodes(node: Node) -> Int:
    """Count the total number of STATIC_ATTR nodes in the tree."""
    if node.kind == NODE_ELEMENT:
        var total = 0
        for i in range(len(node.items)):
            if node.items[i].kind == NODE_STATIC_ATTR:
                total += 1
            elif node.items[i].is_child():
                total += count_static_attr_nodes(node.items[i])
        return total
    else:
        return 0
