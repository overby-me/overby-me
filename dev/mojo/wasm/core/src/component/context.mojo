# ComponentContext — Streamlined component authoring API.
#
# ComponentContext wraps an AppShell and provides a high-level API for
# component authors, dramatically reducing boilerplate compared to raw
# AppShell + Runtime manipulation.
#
# Instead of:
#
#     var shell = app_shell_create()
#     var scope_id = shell.create_root_scope()
#     _ = shell.begin_render(scope_id)
#     var count_key = shell.use_signal_i32(0)
#     _ = shell.read_signal_i32(count_key)    # subscribe scope
#     var memo_id = shell.use_memo_i32(0)
#     _ = shell.memo_read_i32(memo_id)        # subscribe scope
#     shell.end_render(-1)
#     var tmpl_id = UInt32(shell.runtime[0].templates.register(template^))
#     var incr = UInt32(shell.runtime[0].register_handler(
#         HandlerEntry.signal_add(scope_id, count_key, 1, String("click"))
#     ))
#
# Developers write:
#
#     var ctx = ComponentContext.create()
#     var count = ctx.use_signal(0)           # → SignalI32
#     var doubled = ctx.use_memo(0)           # → MemoI32
#     ctx.end_setup()
#     ctx.register_template(view, "counter")
#     var incr = ctx.on_click_add(count, 1)   # → UInt32 handler ID
#
# Multi-template apps (e.g. todo, bench) use register_extra_template():
#
#     var ctx = ComponentContext.create()
#     var version = ctx.use_signal(0)
#     ctx.end_setup()
#     ctx.register_template(app_view, "todo-app")
#     var item_tmpl = ctx.register_extra_template(item_view, "todo-item")
#
# ComponentContext manages:
#   - AppShell creation and lifecycle
#   - Root scope creation and render bracket
#   - Signal/memo/effect creation with automatic scope subscription
#   - Template registration (single primary + extra templates)
#   - Handler registration with short convenience methods
#   - VNode building via the store
#   - Event dispatch, flush, mount, diff lifecycle
#   - Child scope creation and destruction (for keyed list items)
#   - Fragment lifecycle (for dynamic keyed lists)
#
# Ownership: ComponentContext OWNS the AppShell and is responsible for
# destroying it.  The reactive handles (SignalI32, MemoI32, EffectHandle)
# hold non-owning pointers back to the Runtime inside the shell.

from memory import UnsafePointer
from signals import Runtime
from signals.handle import (
    SignalI32,
    SignalBool,
    SignalString,
    MemoI32,
    MemoBool,
    MemoString,
    EffectHandle,
)
from signals.runtime import StringStore
from events import HandlerEntry
from bridge import MutationWriter
from .app_shell import AppShell, app_shell_create
from .lifecycle import (
    FragmentSlot,
    ConditionalSlot,
    flush_conditional,
    flush_conditional_empty,
)
from .child import (
    ChildComponent,
    ChildRenderBuilder,
    ChildEventBinding,
    ChildAutoBinding,
    _ChildEventInfo,
)
from .child_context import ChildComponentContext
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


# ══════════════════════════════════════════════════════════════════════════════
# EventBinding — Stored handler info for auto-populating VNode event attrs
# ══════════════════════════════════════════════════════════════════════════════


struct EventBinding(Copyable, Equatable, Writable):
    """A registered event handler binding for use by RenderBuilder.

    Stores the event name and handler ID so that `RenderBuilder.build()`
    can automatically add dynamic event attributes to the VNode without
    the component author needing to track handler IDs manually.

    `Equatable` and `Writable` are auto-derived from struct fields.
    """

    var event_name: String
    var handler_id: UInt32

    fn __init__(out self, event_name: String, handler_id: UInt32):
        self.event_name = event_name
        self.handler_id = handler_id

    fn __copyinit__(out self, other: Self):
        self.event_name = other.event_name
        self.handler_id = other.handler_id

    fn __moveinit__(out self, deinit other: Self):
        self.event_name = other.event_name^
        self.handler_id = other.handler_id


# ══════════════════════════════════════════════════════════════════════════════
# AutoBinding — Tagged union for auto-populated dynamic attributes (M20.4)
# ══════════════════════════════════════════════════════════════════════════════

comptime AUTO_BIND_EVENT: UInt8 = 0
comptime AUTO_BIND_VALUE: UInt8 = 1


struct AutoBinding(Copyable):
    """A single auto-populated dynamic attribute for RenderBuilder.

    Tagged union with two variants:

    - **EVENT** (`AUTO_BIND_EVENT`): An event handler attribute.
      Uses `event_name` and `handler_id`.

    - **VALUE** (`AUTO_BIND_VALUE`): A value binding attribute.
      Uses `attr_name`, `string_key`, and `version_key`.
      At render time, `build()` reads the SignalString and emits
      a dynamic text attribute.

    Auto-bindings are stored in tree-walk order (matching dyn_attr
    slot indices) so `build()` can emit them sequentially.
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
    fn event(event_name: String, handler_id: UInt32) -> Self:
        """Create an event auto-binding."""
        return Self(
            kind=AUTO_BIND_EVENT,
            event_name=event_name,
            handler_id=handler_id,
            attr_name=String(""),
            string_key=0,
            version_key=0,
        )

    @staticmethod
    fn value(
        attr_name: String, string_key: UInt32, version_key: UInt32
    ) -> Self:
        """Create a value binding auto-binding."""
        return Self(
            kind=AUTO_BIND_VALUE,
            event_name=String(""),
            handler_id=0,
            attr_name=attr_name,
            string_key=string_key,
            version_key=version_key,
        )

    fn __init__(
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

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.event_name = other.event_name
        self.handler_id = other.handler_id
        self.attr_name = other.attr_name
        self.string_key = other.string_key
        self.version_key = other.version_key

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.event_name = other.event_name^
        self.handler_id = other.handler_id
        self.attr_name = other.attr_name^
        self.string_key = other.string_key
        self.version_key = other.version_key

    fn is_event(self) -> Bool:
        """Check whether this is an event binding."""
        return self.kind == AUTO_BIND_EVENT

    fn is_value(self) -> Bool:
        """Check whether this is a value binding."""
        return self.kind == AUTO_BIND_VALUE


# ══════════════════════════════════════════════════════════════════════════════
# RenderBuilder — VNodeBuilder wrapper that auto-adds registered events
# ══════════════════════════════════════════════════════════════════════════════


struct RenderBuilder(Movable):
    """Ergonomic VNode builder that auto-populates event handler attributes
    and value binding attributes.

    Created by `ComponentContext.render_builder()`.  The component author
    only needs to call `add_dyn_text()` for each dynamic text slot — the
    event handlers and value bindings registered via `register_view()` are
    added automatically when `build()` is called.

    Phase 20 (M20.4): Value bindings from `bind_value()` / `bind_attr()`
    are auto-populated by reading the SignalString's current value at
    render time and emitting a dynamic text attribute.

    Usage (in a component's render method):

        fn render(self) -> UInt32:
            var vb = self.ctx.render_builder()
            vb.add_dyn_text("Count: " + str(self.count.peek()))
            return vb.build()

    Compare with manual VNodeBuilder:

        fn build_vnode(self) -> UInt32:
            var vb = self.ctx.vnode_builder()
            vb.add_dyn_text("Count: " + str(self.count.peek()))
            vb.add_dyn_event("click", self.incr_handler)
            vb.add_dyn_event("click", self.decr_handler)
            return vb.index()
    """

    var _vb: VNodeBuilder
    var _events: List[EventBinding]
    var _auto_bindings: List[AutoBinding]
    var _runtime: UnsafePointer[Runtime, MutExternalOrigin]

    fn __init__(
        out self,
        var vb: VNodeBuilder,
        var events: List[EventBinding],
    ):
        """Legacy constructor for backward compatibility (no auto-bindings)."""
        self._vb = vb^
        self._events = events^
        self._auto_bindings = List[AutoBinding]()
        self._runtime = UnsafePointer[Runtime, MutExternalOrigin]()

    fn __init__(
        out self,
        var vb: VNodeBuilder,
        var auto_bindings: List[AutoBinding],
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
    ):
        """Construct with auto-bindings (events + value bindings)."""
        self._vb = vb^
        self._events = List[EventBinding]()
        self._auto_bindings = auto_bindings^
        self._runtime = runtime

    fn __moveinit__(out self, deinit other: Self):
        self._vb = other._vb^
        self._events = other._events^
        self._auto_bindings = other._auto_bindings^
        self._runtime = other._runtime

    # ── Dynamic text ─────────────────────────────────────────────────

    fn add_dyn_text(mut self, value: String):
        """Add a dynamic text node (fills the next DynamicText slot).

        Call in order corresponding to `dyn_text(0)`, `dyn_text(1)`, ...
        placeholders in the template.
        """
        self._vb.add_dyn_text(value)

    fn add_dyn_placeholder(mut self):
        """Add a dynamic placeholder node."""
        self._vb.add_dyn_placeholder()

    # ── Dynamic text from signal ─────────────────────────────────────

    fn add_dyn_text_signal(mut self, signal: SignalString):
        """Add a dynamic text node from a SignalString value.

        Reads the signal's current value (via peek/get — no subscription)
        and adds it as the next dynamic text slot.  This is a convenience
        for the common pattern of reading a SignalString and passing it
        to `add_dyn_text()`.

        Args:
            signal: The SignalString to read.
        """
        self._vb.add_dyn_text(signal.get())

    # ── Dynamic attributes (manual) ─────────────────────────────────

    fn add_dyn_text_attr(mut self, name: String, value: String):
        """Add a dynamic text attribute (e.g. class, id, href)."""
        self._vb.add_dyn_text_attr(name, value)

    fn add_dyn_bool_attr(mut self, name: String, value: Bool):
        """Add a dynamic boolean attribute (e.g. disabled, checked)."""
        self._vb.add_dyn_bool_attr(name, value)

    # ── Conditional class helpers ────────────────────────────────────

    fn add_class_if(mut self, condition: Bool, class_name: String):
        """Add a conditional CSS class attribute.

        Shortcut for `add_dyn_text_attr("class", ...)`.
        Adds the class name if condition is True, or an empty string
        if False.

        Args:
            condition: Whether to apply the class.
            class_name: The CSS class name to apply when True.
        """
        if condition:
            self._vb.add_dyn_text_attr(String("class"), class_name)
        else:
            self._vb.add_dyn_text_attr(String("class"), String(""))

    fn add_class_when(
        mut self,
        condition: Bool,
        true_class: String,
        false_class: String,
    ):
        """Add one of two CSS class names based on a condition.

        Shortcut for `add_dyn_text_attr("class", ...)`.
        For binary class switching (e.g. "active" vs "inactive").

        Args:
            condition: The boolean condition.
            true_class: Class name when True.
            false_class: Class name when False.
        """
        if condition:
            self._vb.add_dyn_text_attr(String("class"), true_class)
        else:
            self._vb.add_dyn_text_attr(String("class"), false_class)

    # ── Build ────────────────────────────────────────────────────────

    fn build(mut self) -> UInt32:
        """Finalize the VNode by auto-adding all registered bindings.

        Adds dynamic attributes for each AutoBinding (events + value
        bindings) registered via `register_view()` in tree-walk order,
        then returns the VNode index.

        For value bindings, reads the SignalString's current value
        from the Runtime and emits a dynamic text attribute.

        Falls back to legacy EventBinding list if no auto-bindings
        are present (backward compatibility).

        Returns:
            The VNode's index in the VNodeStore.
        """
        if len(self._auto_bindings) > 0:
            # Phase 20 (M20.4): unified auto-binding path
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
            # Legacy path: event-only auto-population
            for i in range(len(self._events)):
                self._vb.add_dyn_event(
                    self._events[i].event_name,
                    self._events[i].handler_id,
                )
        return self._vb.index()


struct ComponentContext(Movable):
    """High-level component authoring context.

    Wraps an AppShell and provides ergonomic methods for creating
    signals, memos, effects, handlers, and managing the component
    lifecycle.

    Basic usage (manual handlers):
        var ctx = ComponentContext.create()
        var count = ctx.use_signal(0)
        ctx.end_setup()

        ctx.register_template(view, "counter")
        var incr = ctx.on_click_add(count, 1)
        var decr = ctx.on_click_sub(count, 1)

    Ergonomic usage (inline events via register_view):
        var ctx = ComponentContext.create()
        var count = ctx.use_signal(0)
        ctx.end_setup()

        ctx.register_view(
            el_div([
                el_h1([dyn_text(0)]),
                el_button([text("Up high!"), onclick_add(count, 1)]),
                el_button([text("Down low!"), onclick_sub(count, 1)]),
            ]),
            "counter",
        )

        # In render:
        var vb = ctx.render_builder()
        vb.add_dyn_text("High-Five counter: " + str(count.peek()))
        var idx = vb.build()  # auto-adds event handlers
    """

    var shell: AppShell
    var scope_id: UInt32
    var template_id: UInt32
    var current_vnode: Int
    var _setup_done: Bool
    var _view_events: List[EventBinding]
    var _auto_bindings: List[AutoBinding]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create an uninitialized context.  Call create() instead."""
        self.shell = AppShell()
        self.scope_id = 0
        self.template_id = 0
        self.current_vnode = -1
        self._setup_done = False
        self._view_events = List[EventBinding]()
        self._auto_bindings = List[AutoBinding]()

    fn __moveinit__(out self, deinit other: Self):
        self.shell = other.shell^
        self.scope_id = other.scope_id
        self.template_id = other.template_id
        self.current_vnode = other.current_vnode
        self._setup_done = other._setup_done
        self._view_events = other._view_events^
        self._auto_bindings = other._auto_bindings^

    @staticmethod
    fn create() -> Self:
        """Create a fully initialized ComponentContext.

        Allocates the AppShell, creates a root scope, and begins the
        first render bracket.  Call use_signal/use_memo/use_effect
        to create hooks, then call end_setup() before registering
        templates and handlers.

        Returns:
            A ready-to-use ComponentContext in setup mode.
        """
        var ctx = ComponentContext()
        ctx.shell = app_shell_create()
        ctx.scope_id = ctx.shell.create_root_scope()
        _ = ctx.shell.begin_render(ctx.scope_id)
        ctx._setup_done = False
        return ctx^

    fn end_setup(mut self):
        """End the initial setup (render bracket).

        Must be called after all use_signal/use_memo/use_effect calls
        and before registering templates and handlers.
        """
        self.shell.end_render(-1)
        self._setup_done = True

    fn destroy(mut self):
        """Free all resources.  Safe to call multiple times."""
        self.shell.destroy()

    # ── Signal hooks ─────────────────────────────────────────────────

    fn use_signal(mut self, initial: Int32) -> SignalI32:
        """Create an Int32 signal and subscribe the root scope to it.

        Must be called during setup (before end_setup).

        This is the ergonomic equivalent of:
            var key = shell.use_signal_i32(initial)
            _ = shell.read_signal_i32(key)  # subscribe scope

        Args:
            initial: The initial Int32 value.

        Returns:
            A SignalI32 handle with operator overloading.
        """
        var key = self.shell.use_signal_i32(initial)
        # Read during render to subscribe the scope
        _ = self.shell.read_signal_i32(key)
        return SignalI32(key, self.shell.runtime)

    fn create_signal(mut self, initial: Int32) -> SignalI32:
        """Create an Int32 signal without the hook system.

        Can be called at any time (not just during setup).
        Does NOT auto-subscribe the scope.

        Args:
            initial: The initial Int32 value.

        Returns:
            A SignalI32 handle.
        """
        var key = self.shell.create_signal_i32(initial)
        return SignalI32(key, self.shell.runtime)

    # ── Bool signal hooks ────────────────────────────────────────────

    fn use_signal_bool(mut self, initial: Bool) -> SignalBool:
        """Create a Bool signal and subscribe the root scope to it.

        Must be called during setup (before end_setup).

        Stores the boolean as Int32 (1 for True, 0 for False) internally,
        but the returned SignalBool handle provides a proper Bool API
        with `get()`, `set(Bool)`, `toggle()`, and `read()`.

        Args:
            initial: The initial Bool value.

        Returns:
            A SignalBool handle with boolean semantics.
        """
        var init_val: Int32
        if initial:
            init_val = 1
        else:
            init_val = 0
        var key = self.shell.use_signal_i32(init_val)
        # Read during render to subscribe the scope
        _ = self.shell.read_signal_i32(key)
        return SignalBool(key, self.shell.runtime)

    fn create_signal_bool(mut self, initial: Bool) -> SignalBool:
        """Create a Bool signal without the hook system.

        Can be called at any time (not just during setup).
        Does NOT auto-subscribe the scope.

        Args:
            initial: The initial Bool value.

        Returns:
            A SignalBool handle.
        """
        var init_val: Int32
        if initial:
            init_val = 1
        else:
            init_val = 0
        var key = self.shell.create_signal_i32(init_val)
        return SignalBool(key, self.shell.runtime)

    # ── String signal hooks ──────────────────────────────────────────

    fn use_signal_string(mut self, initial: String) -> SignalString:
        """Create a String signal and subscribe the root scope to it.

        Must be called during setup (before end_setup).

        Stores the string in the Runtime's StringStore (safe for heap
        types) and uses a companion Int32 version signal for reactivity.
        The returned SignalString handle provides a proper String API
        with `get()`, `set(String)`, `read()`, `peek()`, and `is_empty()`.

        Args:
            initial: The initial String value.

        Returns:
            A SignalString handle with string semantics.
        """
        var keys = self.shell.runtime[0].use_signal_string(initial)
        # Read the version signal during render to subscribe the scope
        _ = self.shell.runtime[0].read_signal[Int32](keys[1])
        return SignalString(keys[0], keys[1], self.shell.runtime)

    fn create_signal_string(mut self, initial: String) -> SignalString:
        """Create a String signal without the hook system.

        Can be called at any time (not just during setup).
        Does NOT auto-subscribe the scope.

        Args:
            initial: The initial String value.

        Returns:
            A SignalString handle.
        """
        var keys = self.shell.runtime[0].create_signal_string(initial)
        return SignalString(keys[0], keys[1], self.shell.runtime)

    # ── Memo hooks ───────────────────────────────────────────────────

    fn use_memo(mut self, initial: Int32) -> MemoI32:
        """Create an Int32 memo and subscribe the root scope to it.

        Must be called during setup (before end_setup).

        This is the ergonomic equivalent of:
            var memo_id = shell.use_memo_i32(initial)
            _ = shell.memo_read_i32(memo_id)  # subscribe scope

        Args:
            initial: The initial cached Int32 value.

        Returns:
            A MemoI32 handle with read/recompute methods.
        """
        var memo_id = self.shell.use_memo_i32(initial)
        # Read during render to subscribe the scope to memo output
        _ = self.shell.memo_read_i32(memo_id)
        return MemoI32(memo_id, self.shell.runtime)

    fn create_memo(mut self, initial: Int32) -> MemoI32:
        """Create an Int32 memo without the hook system.

        Can be called at any time.  Does NOT auto-subscribe.

        Args:
            initial: The initial cached Int32 value.

        Returns:
            A MemoI32 handle.
        """
        var memo_id = self.shell.create_memo_i32(self.scope_id, initial)
        return MemoI32(memo_id, self.shell.runtime)

    # ── MemoBool hooks ───────────────────────────────────────────────

    fn use_memo_bool(mut self, initial: Bool) -> MemoBool:
        """Create a Bool memo and subscribe the root scope to it.

        Must be called during setup (before end_setup).

        Args:
            initial: The initial cached Bool value.

        Returns:
            A MemoBool handle with read/recompute methods.
        """
        var memo_id = self.shell.use_memo_bool(initial)
        # Read during render to subscribe the scope to memo output
        _ = self.shell.memo_read_bool(memo_id)
        return MemoBool(memo_id, self.shell.runtime)

    fn create_memo_bool(mut self, initial: Bool) -> MemoBool:
        """Create a Bool memo without the hook system.

        Can be called at any time.  Does NOT auto-subscribe.

        Args:
            initial: The initial cached Bool value.

        Returns:
            A MemoBool handle.
        """
        var memo_id = self.shell.create_memo_bool(self.scope_id, initial)
        return MemoBool(memo_id, self.shell.runtime)

    # ── MemoString hooks ─────────────────────────────────────────────

    fn use_memo_string(mut self, initial: String) -> MemoString:
        """Create a String memo and subscribe the root scope to it.

        Must be called during setup (before end_setup).

        Args:
            initial: The initial cached String value.

        Returns:
            A MemoString handle with read/recompute methods.
        """
        var memo_id = self.shell.use_memo_string(initial)
        # Read during render to subscribe the scope to memo output
        _ = self.shell.memo_read_string(memo_id)
        return MemoString(memo_id, self.shell.runtime)

    fn create_memo_string(mut self, initial: String) -> MemoString:
        """Create a String memo without the hook system.

        Can be called at any time.  Does NOT auto-subscribe.

        Args:
            initial: The initial cached String value.

        Returns:
            A MemoString handle.
        """
        var memo_id = self.shell.create_memo_string(self.scope_id, initial)
        return MemoString(memo_id, self.shell.runtime)

    # ── Effect hooks ─────────────────────────────────────────────────

    fn use_effect(mut self) -> EffectHandle:
        """Create an effect and register it with the hook system.

        Must be called during setup (before end_setup).

        Returns:
            An EffectHandle for lifecycle management.
        """
        var effect_id = self.shell.use_effect()
        return EffectHandle(effect_id, self.shell.runtime)

    fn create_effect(mut self) -> EffectHandle:
        """Create an effect without the hook system.

        Can be called at any time.

        Returns:
            An EffectHandle.
        """
        var effect_id = self.shell.create_effect(self.scope_id)
        return EffectHandle(effect_id, self.shell.runtime)

    # ── Template registration ────────────────────────────────────────

    fn register_template(mut self, view: Node, name: String):
        """Build a template from a Node tree and register it.

        Stores the template ID in `self.template_id` for later use
        in VNode building.

        This is the low-level method — use `register_view()` for the
        ergonomic API that also handles inline event handlers.

        Args:
            view: The root Node of the template tree (from DSL helpers).
            name: The template name (for deduplication).
        """
        var template = to_template(view, name)
        self.template_id = UInt32(
            self.shell.runtime[0].templates.register(template^)
        )

    fn register_extra_template(mut self, view: Node, name: String) -> UInt32:
        """Register an additional template without setting self.template_id.

        Use this when a component needs multiple templates — for example,
        an app shell template (registered via `register_template` or
        `setup_view`) plus a list item template for keyed children.

        Example (todo app):
            ctx.register_template(app_view, "todo-app")
            var item_tmpl = ctx.register_extra_template(item_view, "todo-item")

        Args:
            view: The root Node of the template tree (from DSL helpers).
            name: The template name (for deduplication).

        Returns:
            The registered template ID for use with `vnode_builder_for()`.
        """
        var template = to_template(view, name)
        return UInt32(self.shell.runtime[0].templates.register(template^))

    fn setup_view(mut self, view: Node, name: String):
        """End setup and register a view in one call.

        Combines `end_setup()` + `register_view()` for maximum
        conciseness.  Call after all `use_signal()` / `use_memo()` /
        `use_effect()` calls.

        Supports auto-numbered `dyn_text()` nodes (no explicit index)
        — indices are assigned in tree-walk order (0, 1, 2, ...).

        Equivalent Dioxus pattern:
            rsx! {
                h1 { "High-Five counter: {count}" }
                button { onclick: move |_| count += 1, "Up high!" }
                button { onclick: move |_| count -= 1, "Down low!" }
            }

        Mojo equivalent:
            ctx.setup_view(
                el_div([
                    el_h1([dyn_text()]),
                    el_button([text("Up high!"), onclick_add(count, 1)]),
                    el_button([text("Down low!"), onclick_sub(count, 1)]),
                ]),
                "counter",
            )

        Args:
            view: The root Node of the template tree (may contain
                  NODE_EVENT and auto-numbered dyn_text nodes).
            name: The template name (for deduplication).
        """
        self.end_setup()
        self.register_view(view, name)

    fn register_view(mut self, view: Node, name: String):
        """Build a template from a Node tree with inline event handlers.

        Processes the Node tree to:
        1. Auto-number any `dyn_text()` nodes with sentinel indices
        2. Find all NODE_EVENT nodes and assign dynamic attr indices
        3. Register event handlers for each NODE_EVENT
        4. Build and register the template

        The registered event handlers are stored internally and
        auto-populated by `render_builder().build()`.

        This is the ergonomic alternative to `register_template()` +
        manual `on_click_add()`/`on_click_sub()` calls.

        Supports both explicit `dyn_text(0)` and auto-numbered
        `dyn_text()` (sentinel DYN_TEXT_AUTO) — they can even be mixed,
        though mixing is not recommended.

        Equivalent Dioxus pattern:
            rsx! {
                h1 { "High-Five counter: {count}" }
                button { onclick: move |_| count += 1, "Up high!" }
                button { onclick: move |_| count -= 1, "Down low!" }
            }

        Mojo equivalent:
            ctx.register_view(
                el_div([
                    el_h1([dyn_text()]),
                    el_button([text("Up high!"), onclick_add(count, 1)]),
                    el_button([text("Down low!"), onclick_sub(count, 1)]),
                ]),
                "counter",
            )

        Args:
            view: The root Node of the template tree (may contain
                  NODE_EVENT nodes from onclick_add, onclick_sub, etc.,
                  and auto-numbered dyn_text() nodes).
            name: The template name (for deduplication).
        """
        # 1. Process tree: auto-number dyn_text, replace NODE_EVENT and
        #    NODE_BIND_VALUE with NODE_DYN_ATTR(auto_index), collect info.
        var events = List[_EventInfo]()
        var value_bindings = List[_ValueBindingInfo]()
        var attr_idx = UInt32(0)
        var text_idx = UInt32(0)
        var processed = _process_view_tree(
            view, events, value_bindings, attr_idx, text_idx
        )

        # 2. Build and register template from processed tree
        var template = to_template(processed, name)
        self.template_id = UInt32(
            self.shell.runtime[0].templates.register(template^)
        )

        # 3. Register handlers and store bindings
        self._view_events = List[EventBinding]()
        self._auto_bindings = List[AutoBinding]()

        # Build combined auto-binding list from events + value bindings
        # in tree-walk order.  We track indices to interleave correctly.
        var ev_idx = 0
        var vb_idx = 0
        var total_auto = len(events) + len(value_bindings)
        for _ in range(total_auto):
            # Determine which comes next by comparing original attr indices.
            # Events and value bindings were collected in tree-walk order,
            # so we interleave by checking which has the lower index.
            var use_event: Bool
            if ev_idx < len(events) and vb_idx < len(value_bindings):
                # Both available — compare stored attr_idx to determine order.
                use_event = (
                    events[ev_idx].attr_idx <= value_bindings[vb_idx].attr_idx
                )
            elif ev_idx < len(events):
                use_event = True
            else:
                use_event = False

            if use_event:
                var handler_id = self.shell.runtime[0].register_handler(
                    HandlerEntry(
                        self.scope_id,
                        events[ev_idx].action,
                        events[ev_idx].signal_key,
                        events[ev_idx].operand,
                        events[ev_idx].event_name,
                    )
                )
                self._view_events.append(
                    EventBinding(events[ev_idx].event_name, handler_id)
                )
                self._auto_bindings.append(
                    AutoBinding.event(events[ev_idx].event_name, handler_id)
                )
                ev_idx += 1
            else:
                self._auto_bindings.append(
                    AutoBinding.value(
                        value_bindings[vb_idx].attr_name,
                        value_bindings[vb_idx].string_key,
                        value_bindings[vb_idx].version_key,
                    )
                )
                vb_idx += 1

    # ── View event query ─────────────────────────────────────────────

    fn view_event_handler_id(self, index: Int) -> UInt32:
        """Return the handler ID for the Nth event registered by register_view.

        After `register_view()` or `setup_view()`, events are stored in
        `_view_events` in tree-walk order.  Use this to retrieve the
        handler ID for custom handlers (ACTION_CUSTOM) that need
        app-specific routing.

        Example (TodoApp — Add button is the 2nd event in the tree):

            ctx.register_view(
                el_div(
                    el_input(bind_value(text), oninput_set_string(text)),
                    el_button(text("Add"), onclick_custom()),
                    el_ul(dyn_node(0)),
                ),
                String("todo-app"),
            )
            var add_handler = ctx.view_event_handler_id(1)  # 2nd event

        Args:
            index: Zero-based index into the view event list.

        Returns:
            The handler ID at the given index.
        """
        return self._view_events[index].handler_id

    # ── Flush convenience ────────────────────────────────────────────

    fn flush(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        new_vnode_idx: UInt32,
    ) -> Int32:
        """Diff old → new VNode, write End sentinel, return byte length.

        Convenience method combining `diff()` + `finalize()`.
        Typical usage in a flush function:

            if not app[0].ctx.consume_dirty():
                return 0
            var new_idx = app[0].render()
            return app[0].ctx.flush(writer_ptr, new_idx)

        Args:
            writer_ptr: Mutation buffer.
            new_vnode_idx: Index of the new VNode from render().

        Returns:
            Byte length of mutation data.
        """
        self.diff(writer_ptr, new_vnode_idx)
        return self.finalize(writer_ptr)

    # ── Handler registration — click events ──────────────────────────

    fn on_click_add(self, signal: SignalI32, delta: Int32) -> UInt32:
        """Register a click handler that adds `delta` to a signal.

        Equivalent to: `onclick: move |_| signal += delta`

        Args:
            signal: The signal to modify.
            delta: The amount to add on each click.

        Returns:
            The handler ID for use in VNode event attributes.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_add(
                self.scope_id, signal.key, delta, String("click")
            )
        )

    fn on_click_sub(self, signal: SignalI32, delta: Int32) -> UInt32:
        """Register a click handler that subtracts `delta` from a signal.

        Equivalent to: `onclick: move |_| signal -= delta`

        Args:
            signal: The signal to modify.
            delta: The amount to subtract on each click.

        Returns:
            The handler ID for use in VNode event attributes.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_sub(
                self.scope_id, signal.key, delta, String("click")
            )
        )

    fn on_click_set(self, signal: SignalI32, value: Int32) -> UInt32:
        """Register a click handler that sets a signal to a fixed value.

        Equivalent to: `onclick: move |_| signal.set(value)`

        Args:
            signal: The signal to modify.
            value: The value to set on each click.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_set(
                self.scope_id, signal.key, value, String("click")
            )
        )

    fn on_click_toggle(self, signal: SignalI32) -> UInt32:
        """Register a click handler that toggles a boolean signal (0 ↔ 1).

        Equivalent to: `onclick: move |_| signal.toggle()`

        Args:
            signal: The boolean signal to toggle.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_toggle(
                self.scope_id, signal.key, String("click")
            )
        )

    # ── Handler registration — generic events ────────────────────────

    fn on_event_add(
        self, event_name: String, signal: SignalI32, delta: Int32
    ) -> UInt32:
        """Register a handler for any event that adds `delta` to a signal.

        Args:
            event_name: The DOM event name (e.g. "click", "input").
            signal: The signal to modify.
            delta: The amount to add.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_add(
                self.scope_id, signal.key, delta, event_name
            )
        )

    fn on_event_sub(
        self, event_name: String, signal: SignalI32, delta: Int32
    ) -> UInt32:
        """Register a handler for any event that subtracts `delta`.

        Args:
            event_name: The DOM event name.
            signal: The signal to modify.
            delta: The amount to subtract.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_sub(
                self.scope_id, signal.key, delta, event_name
            )
        )

    fn on_event_set(
        self, event_name: String, signal: SignalI32, value: Int32
    ) -> UInt32:
        """Register a handler for any event that sets a signal.

        Args:
            event_name: The DOM event name.
            signal: The signal to modify.
            value: The value to set.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_set(
                self.scope_id, signal.key, value, event_name
            )
        )

    fn on_input_set(self, signal: SignalI32) -> UInt32:
        """Register an input handler that sets a signal from event data.

        Equivalent to: `oninput: move |e| signal.set(e.value)`

        Args:
            signal: The signal to update with the input value.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.signal_set_input(
                self.scope_id, signal.key, String("input")
            )
        )

    fn register_handler(self, entry: HandlerEntry) -> UInt32:
        """Register a raw HandlerEntry for full control.

        Use this when the convenience methods don't cover your use case.

        Args:
            entry: The fully configured handler entry.

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(entry)

    fn register_custom_handler(self, event_name: String) -> UInt32:
        """Register a custom (app-routed) event handler under the root scope.

        Creates a HandlerEntry with ACTION_CUSTOM.  When dispatched, the
        runtime marks the scope dirty and returns False — the app's event
        handler then performs custom routing based on the handler ID.

        Use this for handlers that live outside the main view tree (e.g.
        in extra templates registered via register_extra_template).

        Args:
            event_name: The DOM event name (e.g. "click").

        Returns:
            The handler ID.
        """
        return self.shell.runtime[0].register_handler(
            HandlerEntry.custom(self.scope_id, event_name)
        )

    fn mark_dirty(mut self):
        """Manually mark the root scope as dirty.

        Use this when a non-signal state change (e.g. router navigation
        triggered by an external JS call) needs to trigger a re-render.
        Signal writes already mark subscribing scopes dirty automatically,
        so this is only needed for state changes that bypass the reactive
        system.
        """
        self.shell.runtime[0].mark_scope_dirty(self.scope_id)

    # ── VNode building ───────────────────────────────────────────────

    fn vnode_builder(self) -> VNodeBuilder:
        """Create a VNodeBuilder for the context's registered template.

        Returns:
            A VNodeBuilder ready for add_dyn_text/add_dyn_event calls.
        """
        return VNodeBuilder(self.template_id, self.shell.store)

    fn render_builder(mut self) -> RenderBuilder:
        """Create a RenderBuilder that auto-adds registered event handlers
        and value bindings.

        Use this with `register_view()` for the ergonomic rendering API.
        Call `add_dyn_text()` for each dynamic text slot, then `build()`
        to finalize (events and value bindings are added automatically).

        Phase 20 (M20.4): If auto-bindings are present (from `bind_value()`
        or `bind_attr()` nodes in the view), uses the unified auto-binding
        path.  Otherwise falls back to legacy event-only path.

        Usage:
            var vb = ctx.render_builder()
            vb.add_dyn_text("High-Five counter: " + str(count.peek()))
            var idx = vb.build()

        Returns:
            A RenderBuilder wrapping a VNodeBuilder with auto-binding support.
        """
        var vb = VNodeBuilder(self.template_id, self.shell.store)
        if len(self._auto_bindings) > 0:
            return RenderBuilder(
                vb^, self._auto_bindings.copy(), self.shell.runtime
            )
        else:
            return RenderBuilder(vb^, self._view_events.copy())

    fn vnode_builder_keyed(self, key: String) -> VNodeBuilder:
        """Create a keyed VNodeBuilder for the context's template.

        Args:
            key: The key string for keyed diffing.

        Returns:
            A keyed VNodeBuilder.
        """
        return VNodeBuilder(self.template_id, key, self.shell.store)

    fn vnode_builder_for(self, template_id: UInt32) -> VNodeBuilder:
        """Create a VNodeBuilder for a specific template ID.

        Use this when the component has multiple templates.

        Args:
            template_id: The template to instantiate.

        Returns:
            A VNodeBuilder.
        """
        return VNodeBuilder(template_id, self.shell.store)

    # ── Mount / Rebuild lifecycle ────────────────────────────────────

    fn mount(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        vnode_idx: UInt32,
    ) -> Int32:
        """Initial mount: emit templates + create VNode + append to root.

        Call this for the first render of the component.

        Args:
            writer_ptr: Mutation buffer.
            vnode_idx: Index of the VNode to mount.

        Returns:
            Byte length of mutation data.
        """
        self.current_vnode = Int(vnode_idx)
        return self.shell.mount_with_templates(writer_ptr, vnode_idx)

    # ── Event dispatch ───────────────────────────────────────────────

    fn dispatch_event(mut self, handler_id: UInt32, event_type: UInt8) -> Bool:
        """Dispatch an event to a handler.

        Args:
            handler_id: The handler to invoke.
            event_type: The event type tag (EVT_CLICK, etc.).

        Returns:
            True if an action was executed.
        """
        return self.shell.dispatch_event(handler_id, event_type)

    fn dispatch_event_with_string(
        mut self, handler_id: UInt32, event_type: UInt8, value: String
    ) -> Bool:
        """Dispatch an event with a String payload (Phase 20).

        For ACTION_SIGNAL_SET_STRING handlers, writes the string value
        to the target SignalString.  Falls back to normal dispatch for
        other action types.

        Args:
            handler_id: The handler to invoke.
            event_type: The event type tag (EVT_INPUT, etc.).
            value: The String payload from the event (e.g. input.value).

        Returns:
            True if an action was executed.
        """
        return self.shell.dispatch_event_with_string(
            handler_id, event_type, value
        )

    # ── Flush lifecycle ──────────────────────────────────────────────

    fn has_dirty(self) -> Bool:
        """Check if any scopes need re-rendering."""
        return self.shell.has_dirty()

    fn consume_dirty(mut self) -> Bool:
        """Collect and consume all dirty scopes.

        Returns:
            True if any scopes were dirty.
        """
        return self.shell.consume_dirty()

    fn settle_scopes(mut self):
        """Remove dirty scopes with no actual signal changes.

        After memo recomputation, some scopes may have been eagerly
        marked dirty (Phase 36) but none of their subscribed signals
        actually changed value.  This method checks each dirty scope
        and removes those where no subscribed signal changed.

        Call after run_memos() (and run_effects() if applicable) and
        before render() to skip unnecessary re-renders.
        """
        self.shell.runtime[0].settle_scopes()

    # ── Batch signal writes (Phase 38) ───────────────────────────────

    fn begin_batch(mut self):
        """Enter batch mode for signal writes."""
        self.shell.runtime[0].begin_batch()

    fn end_batch(mut self):
        """Exit batch mode and propagate all deferred writes."""
        self.shell.runtime[0].end_batch()

    fn is_batching(self) -> Bool:
        """Return True if currently inside a batch."""
        return self.shell.runtime[0].is_batching()

    fn diff(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        new_vnode_idx: UInt32,
    ):
        """Diff the current VNode against a new one.

        Updates current_vnode to the new index.

        Args:
            writer_ptr: Mutation buffer.
            new_vnode_idx: Index of the new VNode.
        """
        var old_idx = UInt32(self.current_vnode)
        self.shell.diff(writer_ptr, old_idx, new_vnode_idx)
        self.current_vnode = Int(new_vnode_idx)

    fn finalize(
        self, writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]
    ) -> Int32:
        """Write the End sentinel and return byte length.

        Args:
            writer_ptr: Mutation buffer.

        Returns:
            Byte length of mutation data.
        """
        return self.shell.finalize(writer_ptr)

    # ── Child scopes (for keyed list items) ──────────────────────────

    fn create_child_scope(mut self) -> UInt32:
        """Create a child scope under the root scope.

        Each keyed list item typically gets its own child scope so that
        its event handlers can be cleaned up independently when the item
        is removed or the list is rebuilt.

        Returns:
            The new child scope ID.
        """
        return self.shell.create_child_scope(self.scope_id)

    fn destroy_child_scopes(mut self, scope_ids: List[UInt32]):
        """Destroy a list of child scopes and clean up their handlers.

        Call this before rebuilding a keyed list to release the old
        per-item scopes and their registered event handlers.

        Args:
            scope_ids: The child scope IDs to destroy.
        """
        self.shell.destroy_child_scopes(scope_ids)

    # ── Child component composition ──────────────────────────────────

    fn create_child_component(
        mut self, view: Node, name: String
    ) -> ChildComponent:
        """Create a child component with its own scope and template.

        Processes the view tree for inline events (same as register_view),
        registers the template via register_extra_template, creates a
        child scope, and registers event handlers under the child scope.

        The returned ChildComponent can produce VNodes via render_builder()
        that plug into the parent's dyn_node() slots.

        Example:
            var child = ctx.create_child_component(
                el_p(dyn_text()),
                String("display"),
            )

            # In render:
            var cvb = child.render_builder(ctx.store_ptr(), ctx.runtime_ptr())
            cvb.add_dyn_text("Count: " + str(count.peek()))
            var child_idx = cvb.build()
            child.update_vnode(child_idx)
            parent_vb.add_dyn_node(child_idx)

        Args:
            view: The root Node of the child's template tree (may contain
                  NODE_EVENT nodes from onclick_add, onclick_sub, etc.,
                  and auto-numbered dyn_text() nodes).
            name: The template name (for deduplication).

        Returns:
            A ChildComponent handle with its own scope and template.
        """
        # 1. Create a child scope under the root scope
        var child_scope_id = self.shell.create_child_scope(self.scope_id)

        # 2. Process tree: auto-number dyn_text, replace NODE_EVENT and
        #    NODE_BIND_VALUE with NODE_DYN_ATTR, collect info.
        var events = List[_EventInfo]()
        var value_bindings = List[_ValueBindingInfo]()
        var attr_idx = UInt32(0)
        var text_idx = UInt32(0)
        var processed = _process_view_tree(
            view, events, value_bindings, attr_idx, text_idx
        )

        # 3. Build and register template
        var template = to_template(processed, name)
        var tmpl_id = UInt32(
            self.shell.runtime[0].templates.register(template^)
        )

        # 4. Register handlers under the CHILD scope and store bindings
        var event_bindings = List[ChildEventBinding]()
        var auto_bindings = List[ChildAutoBinding]()

        var ev_idx = 0
        var vb_idx = 0
        var total_auto = len(events) + len(value_bindings)
        for _ in range(total_auto):
            var use_event: Bool
            if ev_idx < len(events) and vb_idx < len(value_bindings):
                use_event = (
                    events[ev_idx].attr_idx <= value_bindings[vb_idx].attr_idx
                )
            elif ev_idx < len(events):
                use_event = True
            else:
                use_event = False

            if use_event:
                var handler_id = self.shell.runtime[0].register_handler(
                    HandlerEntry(
                        child_scope_id,
                        events[ev_idx].action,
                        events[ev_idx].signal_key,
                        events[ev_idx].operand,
                        events[ev_idx].event_name,
                    )
                )
                event_bindings.append(
                    ChildEventBinding(events[ev_idx].event_name, handler_id)
                )
                auto_bindings.append(
                    ChildAutoBinding.event(
                        events[ev_idx].event_name, handler_id
                    )
                )
                ev_idx += 1
            else:
                auto_bindings.append(
                    ChildAutoBinding.value(
                        value_bindings[vb_idx].attr_name,
                        value_bindings[vb_idx].string_key,
                        value_bindings[vb_idx].version_key,
                    )
                )
                vb_idx += 1

        return ChildComponent(
            child_scope_id, tmpl_id, event_bindings^, auto_bindings^
        )

    fn destroy_child_component(mut self, child: ChildComponent):
        """Destroy a child component's scope and handlers.

        Convenience method that delegates to `child.destroy()` with
        the context's runtime pointer.

        Args:
            child: The ChildComponent to destroy.
        """
        child.destroy(self.shell.runtime)

    # ── Child component context (self-rendering children) ────────────

    fn create_child_context(
        mut self,
        view: Node,
        name: String,
    ) -> ChildComponentContext:
        """Create a ChildComponentContext for a self-rendering child.

        Same as create_child_component() but returns a richer context
        that supports signal creation, context consumption, and
        self-rendering.

        The returned ChildComponentContext wraps the ChildComponent and
        holds non-owning pointers to the shared Runtime, VNodeStore,
        and ElementIdAllocator.

        Args:
            view: The root Node of the child's template tree.
            name: The template name (for deduplication).

        Returns:
            A ChildComponentContext with signal/context/render APIs.
        """
        var child = self.create_child_component(view, name)
        var child_scope_id = child.scope_id
        return ChildComponentContext(
            child^,
            child_scope_id,
            self.shell.runtime,
            self.shell.store,
            self.shell.eid_alloc,
        )

    fn destroy_child_context(mut self, child_ctx: ChildComponentContext):
        """Destroy a ChildComponentContext and its resources.

        Delegates to the child context's destroy() method which cleans
        up the child scope, its signals, and handlers.

        Args:
            child_ctx: The ChildComponentContext to destroy.
        """
        child_ctx.destroy()

    fn create_child_context_under(
        mut self,
        parent_scope_id: UInt32,
        view: Node,
        name: String,
    ) -> ChildComponentContext:
        """Create a ChildComponentContext under an arbitrary parent scope.

        Like ``create_child_context()`` but the new child scope is
        parented under *parent_scope_id* instead of the root scope.
        This is essential for nested error boundaries: inner children
        must be scoped under the inner boundary so that
        ``propagate_error()`` finds the correct boundary.

        Args:
            parent_scope_id: Scope ID to parent the new child under.
            view: The root Node of the child's template tree.
            name: The template name (for deduplication).

        Returns:
            A ChildComponentContext with signal/context/render APIs.
        """
        # 1. Create a child scope under the specified parent
        var child_scope_id = self.shell.create_child_scope(parent_scope_id)

        # 2. Process tree: auto-number dyn_text, replace NODE_EVENT and
        #    NODE_BIND_VALUE with NODE_DYN_ATTR, collect info.
        var events = List[_EventInfo]()
        var value_bindings = List[_ValueBindingInfo]()
        var attr_idx = UInt32(0)
        var text_idx = UInt32(0)
        var processed = _process_view_tree(
            view, events, value_bindings, attr_idx, text_idx
        )

        # 3. Build and register template
        var template = to_template(processed, name)
        var tmpl_id = UInt32(
            self.shell.runtime[0].templates.register(template^)
        )

        # 4. Register handlers under the CHILD scope and store bindings
        var event_bindings = List[ChildEventBinding]()
        var auto_bindings = List[ChildAutoBinding]()

        var ev_idx = 0
        var vb_idx = 0
        var total_auto = len(events) + len(value_bindings)
        for _ in range(total_auto):
            var use_event: Bool
            if ev_idx < len(events) and vb_idx < len(value_bindings):
                use_event = (
                    events[ev_idx].attr_idx <= value_bindings[vb_idx].attr_idx
                )
            elif ev_idx < len(events):
                use_event = True
            else:
                use_event = False

            if use_event:
                var handler_id = self.shell.runtime[0].register_handler(
                    HandlerEntry(
                        child_scope_id,
                        events[ev_idx].action,
                        events[ev_idx].signal_key,
                        events[ev_idx].operand,
                        events[ev_idx].event_name,
                    )
                )
                event_bindings.append(
                    ChildEventBinding(events[ev_idx].event_name, handler_id)
                )
                auto_bindings.append(
                    ChildAutoBinding.event(
                        events[ev_idx].event_name, handler_id
                    )
                )
                ev_idx += 1
            else:
                auto_bindings.append(
                    ChildAutoBinding.value(
                        value_bindings[vb_idx].attr_name,
                        value_bindings[vb_idx].string_key,
                        value_bindings[vb_idx].version_key,
                    )
                )
                vb_idx += 1

        var child = ChildComponent(
            child_scope_id, tmpl_id, event_bindings^, auto_bindings^
        )
        return ChildComponentContext(
            child^,
            child_scope_id,
            self.shell.runtime,
            self.shell.store,
            self.shell.eid_alloc,
        )

    # ── Context (Dependency Injection) ───────────────────────────────

    fn provide_context(mut self, key: UInt32, value: Int32):
        """Provide a context value at the root scope.

        Stores a key-value pair in the root scope's context map.
        Child scopes (and their descendants) can retrieve this value
        via `consume_context()` which walks up the scope tree.

        If the key already exists, the value is updated.

        Args:
            key: A unique UInt32 identifier for the context entry.
            value: The Int32 value to provide.
        """
        self.shell.runtime[0].scopes.provide_context(self.scope_id, key, value)

    fn consume_context(self, key: UInt32) -> Tuple[Bool, Int32]:
        """Look up a context value walking up the scope tree.

        Starts at the root scope and walks up the parent chain until
        a matching key is found or the root is reached.

        Args:
            key: The context key to look up.

        Returns:
            A tuple of (found: Bool, value: Int32).
        """
        return self.shell.runtime[0].scopes.consume_context(self.scope_id, key)

    fn has_context(self, key: UInt32) -> Bool:
        """Check whether a context value is reachable from the root scope.

        Equivalent to `consume_context(key)[0]`.

        Args:
            key: The context key to check.

        Returns:
            True if a value for the key is found in the scope tree.
        """
        return self.consume_context(key)[0]

    # ── Signal sharing via context ───────────────────────────────────

    fn provide_signal_i32(mut self, key: UInt32, signal: SignalI32):
        """Provide a SignalI32 handle to descendants via context.

        Stores the signal's internal key in the scope's context map
        so that descendants can reconstruct a handle via
        `consume_signal_i32()`.

        Args:
            key: A unique UInt32 context key for this signal prop.
            signal: The SignalI32 to share.
        """
        self.provide_context(key, Int32(signal.key))

    fn provide_signal_bool(mut self, key: UInt32, signal: SignalBool):
        """Provide a SignalBool handle to descendants via context.

        Stores the signal's internal key in the scope's context map.

        Args:
            key: A unique UInt32 context key for this signal prop.
            signal: The SignalBool to share.
        """
        self.provide_context(key, Int32(signal.key))

    fn provide_signal_string(mut self, key: UInt32, signal: SignalString):
        """Provide a SignalString handle to descendants via context.

        Stores the signal's string key in the scope's context map.
        The version key is stored at `key + 1` so that the consumer
        can reconstruct a full SignalString handle.

        Convention: the caller must reserve two consecutive context
        keys — `key` for the string key and `key + 1` for the version
        key.

        Args:
            key: A unique UInt32 context key for this signal prop.
            signal: The SignalString to share.
        """
        self.provide_context(key, Int32(signal.string_key))
        self.provide_context(key + 1, Int32(signal.version_key))

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
        return SignalI32(UInt32(result[1]), self.shell.runtime)

    fn consume_signal_bool(self, key: UInt32) -> SignalBool:
        """Look up a SignalBool from an ancestor's context.

        Retrieves the signal key stored by `provide_signal_bool()` and
        reconstructs a SignalBool handle.

        Args:
            key: The context key used in `provide_signal_bool()`.

        Returns:
            A SignalBool handle for the shared signal.
        """
        var result = self.consume_context(key)
        return SignalBool(UInt32(result[1]), self.shell.runtime)

    fn consume_signal_string(self, key: UInt32) -> SignalString:
        """Look up a SignalString from an ancestor's context.

        Retrieves both the string key (at `key`) and version key
        (at `key + 1`) stored by `provide_signal_string()` and
        reconstructs a SignalString handle.

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
            self.shell.runtime,
        )

    # ── Error Boundary ───────────────────────────────────────────────

    fn use_error_boundary(mut self):
        """Mark the root scope as an error boundary.

        Call during setup (before end_setup / setup_view).  When a
        descendant scope reports an error via ``report_error()``, this
        scope captures it.  Check ``has_error()`` during flush to
        switch between normal content and fallback UI.

        Example::

            self.ctx = ComponentContext.create()
            self.ctx.use_error_boundary()
            # ... use_signal, setup_view, etc. ...
        """
        self.shell.runtime[0].scopes.set_error_boundary(self.scope_id, True)

    fn report_error(mut self, message: String) -> Int:
        """Report an error to the nearest error boundary.

        First checks whether this scope itself is a boundary — if so,
        the error is set directly on it.  Otherwise walks up the parent
        chain via ``propagate_error()``.  In both cases the boundary
        scope is marked dirty so the next flush picks up the change.

        Args:
            message: Description of the error.

        Returns:
            The boundary scope ID as Int, or -1 if no boundary found.
        """
        # If this scope is itself a boundary, set the error directly
        # (propagate_error only checks ancestors, not self).
        if self.shell.runtime[0].scopes.is_error_boundary(self.scope_id):
            self.shell.runtime[0].scopes.set_error(self.scope_id, message)
            self.shell.runtime[0].mark_scope_dirty(self.scope_id)
            return Int(self.scope_id)
        var boundary_id = self.shell.runtime[0].scopes.propagate_error(
            self.scope_id, message
        )
        if boundary_id != -1:
            self.shell.runtime[0].mark_scope_dirty(UInt32(boundary_id))
        return boundary_id

    fn has_error(self) -> Bool:
        """Check whether this scope (as a boundary) has captured an error.

        Returns:
            True if an error has been propagated to this boundary.
        """
        return self.shell.runtime[0].scopes.has_error(self.scope_id)

    fn error_message(self) -> String:
        """Get the error message captured by this boundary.

        Returns:
            The error message string, or empty if no error.
        """
        return self.shell.runtime[0].scopes.get_error_message(self.scope_id)

    fn clear_error(mut self):
        """Clear the error state on this boundary scope.

        After clearing, the next flush should render normal children
        instead of fallback UI.  Marks the scope dirty so the flush
        cycle processes the state change.
        """
        self.shell.runtime[0].scopes.clear_error(self.scope_id)
        self.shell.runtime[0].mark_scope_dirty(self.scope_id)

    # ── Suspense ─────────────────────────────────────────────────────

    fn use_suspense_boundary(mut self):
        """Mark the root scope as a suspense boundary.

        Call during setup (before end_setup / setup_view).  When a
        descendant scope is pending, this boundary should show fallback
        UI.  Check ``has_pending()`` during flush to switch between
        content and skeleton.

        Example::

            self.ctx = ComponentContext.create()
            self.ctx.use_suspense_boundary()
            # ... use_signal, setup_view, etc. ...
        """
        self.shell.runtime[0].scopes.set_suspense_boundary(self.scope_id, True)

    fn set_pending(mut self, pending: Bool):
        """Set the pending (loading) state on the root scope.

        When pending is True, the nearest suspense boundary ancestor
        (or self if self is a boundary) should show fallback UI.
        Marks the boundary scope dirty so the next flush picks up
        the change.

        Args:
            pending: True to enter pending state, False to resolve.
        """
        self.shell.runtime[0].scopes.set_pending(self.scope_id, pending)
        var boundary_id = self.shell.runtime[0].scopes.find_suspense_boundary(
            self.scope_id
        )
        if boundary_id != -1:
            self.shell.runtime[0].mark_scope_dirty(UInt32(boundary_id))
        elif self.shell.runtime[0].scopes.is_suspense_boundary(self.scope_id):
            self.shell.runtime[0].mark_scope_dirty(self.scope_id)

    fn has_pending(self) -> Bool:
        """Check whether any descendant of this scope is pending.

        Scans all live scopes for pending descendants. Used by
        suspense boundaries to decide whether to show fallback.

        Returns:
            True if any descendant scope is in pending state.
        """
        return self.shell.runtime[0].scopes.has_pending_descendant(
            self.scope_id
        )

    fn is_pending(self) -> Bool:
        """Check whether this scope itself is in pending state.

        Returns:
            True if this scope is pending.
        """
        return self.shell.runtime[0].scopes.is_pending(self.scope_id)

    # ── Fragment lifecycle (for dynamic keyed lists) ─────────────────

    fn flush_fragment(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        slot: FragmentSlot,
        new_frag_idx: UInt32,
    ) -> FragmentSlot:
        """Flush a fragment slot: diff old vs new fragment and emit mutations.

        Delegates to `AppShell.flush_fragment()` which handles all three
        transitions: empty→populated, populated→populated, populated→empty.

        Does NOT call `finalize()` — the caller must finalize the
        mutation buffer after this returns.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            slot: The FragmentSlot tracking the current state.
            new_frag_idx: Index of the new Fragment VNode in the store.

        Returns:
            Updated FragmentSlot with new state.
        """
        return self.shell.flush_fragment(writer_ptr, slot, new_frag_idx)

    # ── Conditional slot helpers ─────────────────────────────────────

    fn conditional_slot(self) -> ConditionalSlot:
        """Create an uninitialized ConditionalSlot.

        The slot must be initialized with an anchor ElementId after
        initial mount — extract it from the VNode's dyn_node_ids at
        the slot's dynamic node index.

        Example:
            # In app struct:
            var cond: ConditionalSlot

            # After mount — extract anchor from dyn_node_ids:
            var vnode_ptr = ctx.store_ptr()[0].get_ptr(app_vnode_idx)
            var anchor = vnode_ptr[0].get_dyn_node_id(slot_index)
            self.cond = ConditionalSlot(anchor)

            # During flush — show a branch:
            self.cond = ctx.flush_conditional_slot(writer, self.cond, detail_idx)

            # During flush — hide (back to placeholder):
            self.cond = ctx.flush_conditional_slot_empty(writer, self.cond)

        Returns:
            An uninitialized ConditionalSlot.
        """
        return ConditionalSlot()

    fn flush_conditional_slot(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        slot: ConditionalSlot,
        new_vnode_idx: UInt32,
    ) -> ConditionalSlot:
        """Flush a conditional slot: show or update a branch VNode.

        Handles two transitions:
          1. **empty → branch**: Create new VNode, ReplaceWith anchor.
          2. **branch → branch**: Diff old vs new VNode.

        To hide the branch (back to placeholder), use
        `flush_conditional_slot_empty()` instead.

        Does NOT call `finalize()` — the caller must finalize the
        mutation buffer after this returns.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            slot: The ConditionalSlot tracking the current state.
            new_vnode_idx: Index of the new branch VNode in the store.

        Returns:
            Updated ConditionalSlot with new state.
        """
        return flush_conditional(
            writer_ptr,
            self.shell.eid_alloc,
            self.shell.runtime,
            self.shell.store,
            slot,
            new_vnode_idx,
        )

    fn flush_conditional_slot_empty(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
        slot: ConditionalSlot,
    ) -> ConditionalSlot:
        """Flush a conditional slot: hide the current branch (back to placeholder).

        Handles the transition:
          - **branch → empty**: Create new anchor placeholder,
            InsertBefore old root, remove old VNode.

        If the slot is already empty, this is a no-op.

        Does NOT call `finalize()` — the caller must finalize the
        mutation buffer after this returns.

        Args:
            writer_ptr: Pointer to the MutationWriter for output.
            slot: The ConditionalSlot tracking the current state.

        Returns:
            Updated ConditionalSlot with empty state and new anchor ID.
        """
        return flush_conditional_empty(
            writer_ptr,
            self.shell.eid_alloc,
            self.shell.runtime,
            self.shell.store,
            slot,
        )

    fn build_empty_fragment(self) -> UInt32:
        """Create an empty Fragment VNode in the store.

        Convenience for initializing a FragmentSlot before the first
        list render.

        Returns:
            The VNode index of the empty fragment.
        """
        return self.shell.store[0].push(VNode.fragment())

    fn push_fragment_child(self, frag_idx: UInt32, child_idx: UInt32):
        """Append a child VNode to an existing Fragment VNode.

        Args:
            frag_idx: Index of the Fragment VNode.
            child_idx: Index of the child VNode to append.
        """
        self.shell.store[0].push_fragment_child(frag_idx, child_idx)

    # ── Accessors for WASM exports ───────────────────────────────────

    fn runtime_ptr(self) -> UnsafePointer[Runtime, MutExternalOrigin]:
        """Return the runtime pointer (for WASM export helpers)."""
        return self.shell.runtime

    fn store_ptr(self) -> UnsafePointer[VNodeStore, MutExternalOrigin]:
        """Return the VNode store pointer."""
        return self.shell.store

    fn handler_count(self) -> UInt32:
        """Return the number of live event handlers in the runtime.

        Useful for testing and introspection.
        """
        return self.shell.runtime[0].handler_count()

    fn view_events(self) -> List[EventBinding]:
        """Return a copy of the registered view event bindings.

        Useful for testing and introspection.
        """
        return self._view_events.copy()

    fn auto_bindings(self) -> List[AutoBinding]:
        """Return a copy of the registered auto-bindings (events + values).

        Useful for testing and introspection.  Phase 20 (M20.4).
        """
        return self._auto_bindings.copy()


# ══════════════════════════════════════════════════════════════════════════════
# Private helpers — Event collection and tree reindexing
# ══════════════════════════════════════════════════════════════════════════════


struct _EventInfo(Copyable, Equatable, Writable):
    """Internal: collected event handler info from a NODE_EVENT node."""

    var event_name: String
    var action: UInt8
    var signal_key: UInt32
    var operand: Int32
    var attr_idx: UInt32  # The dyn_attr slot index assigned in _process_view_tree

    fn __init__(
        out self,
        event_name: String,
        action: UInt8,
        signal_key: UInt32,
        operand: Int32,
        attr_idx: UInt32,
    ):
        self.event_name = event_name
        self.action = action
        self.signal_key = signal_key
        self.operand = operand
        self.attr_idx = attr_idx

    fn __copyinit__(out self, other: Self):
        self.event_name = other.event_name
        self.action = other.action
        self.signal_key = other.signal_key
        self.operand = other.operand
        self.attr_idx = other.attr_idx

    fn __moveinit__(out self, deinit other: Self):
        self.event_name = other.event_name^
        self.action = other.action
        self.signal_key = other.signal_key
        self.operand = other.operand
        self.attr_idx = other.attr_idx


struct _ValueBindingInfo(Copyable, Equatable, Writable):
    """Internal: collected value binding info from a NODE_BIND_VALUE node."""

    var attr_name: String
    var string_key: UInt32
    var version_key: UInt32
    var attr_idx: UInt32  # The dyn_attr slot index assigned in _process_view_tree

    fn __init__(
        out self,
        attr_name: String,
        string_key: UInt32,
        version_key: UInt32,
        attr_idx: UInt32,
    ):
        self.attr_name = attr_name
        self.string_key = string_key
        self.version_key = version_key
        self.attr_idx = attr_idx

    fn __copyinit__(out self, other: Self):
        self.attr_name = other.attr_name
        self.string_key = other.string_key
        self.version_key = other.version_key
        self.attr_idx = other.attr_idx

    fn __moveinit__(out self, deinit other: Self):
        self.attr_name = other.attr_name^
        self.string_key = other.string_key
        self.version_key = other.version_key
        self.attr_idx = other.attr_idx


fn _process_view_tree(
    node: Node,
    mut events: List[_EventInfo],
    mut value_bindings: List[_ValueBindingInfo],
    mut attr_idx: UInt32,
    mut text_idx: UInt32,
) -> Node:
    """Recursively clone a Node tree, processing events, value bindings,
    and auto-numbered text.

    Performs three transformations in a single tree walk:

    1. **NODE_EVENT → NODE_DYN_ATTR**: Each NODE_EVENT is collected into
       `events` and replaced with a NODE_DYN_ATTR with an auto-assigned
       dynamic attribute index.

    2. **NODE_BIND_VALUE → NODE_DYN_ATTR**: Each NODE_BIND_VALUE is
       collected into `value_bindings` and replaced with a NODE_DYN_ATTR
       with an auto-assigned dynamic attribute index.  (Phase 20, M20.4)

    3. **Auto-numbered dyn_text**: Any NODE_DYN_TEXT node with
       `dynamic_index == DYN_TEXT_AUTO` (the sentinel from `dyn_text()`)
       is replaced with a properly numbered NODE_DYN_TEXT node using the
       next sequential text index.

    All other nodes are cloned unchanged.  ELEMENT nodes have their
    items recursively processed.

    Args:
        node: The current node to process.
        events: Accumulator for collected event info (mutated in place).
        value_bindings: Accumulator for collected value binding info (mutated).
        attr_idx: Running counter for dynamic attr indices (mutated).
        text_idx: Running counter for auto-numbered dyn_text indices (mutated).

    Returns:
        A new Node tree with events/bindings replaced and dyn_text auto-numbered.
    """
    if node.kind == NODE_EVENT:
        # Collect the event info
        var idx = attr_idx
        attr_idx += 1
        events.append(
            _EventInfo(
                event_name=node.text,
                action=node.tag,
                signal_key=node.dynamic_index,
                operand=node.operand,
                attr_idx=idx,
            )
        )
        # Replace with a dyn_attr at the current index
        return Node.dynamic_attr_node(idx)

    elif node.kind == NODE_BIND_VALUE:
        # Collect the value binding info (Phase 20 — M20.4)
        var idx = attr_idx
        attr_idx += 1
        value_bindings.append(
            _ValueBindingInfo(
                attr_name=node.text,
                string_key=node.dynamic_index,
                version_key=UInt32(node.operand),
                attr_idx=idx,
            )
        )
        # Replace with a dyn_attr at the current index
        return Node.dynamic_attr_node(idx)

    elif node.kind == NODE_DYN_TEXT:
        if node.dynamic_index == DYN_TEXT_AUTO:
            # Auto-assign the next sequential text index
            var idx = text_idx
            text_idx += 1
            return Node.dynamic_text_node(idx)
        else:
            # Explicit index — pass through but still advance counter
            # past it so auto-numbering doesn't collide
            if node.dynamic_index >= text_idx:
                text_idx = node.dynamic_index + 1
            return node.copy()

    elif node.kind == NODE_ELEMENT:
        # Recursively process items (children + attrs)
        var new_items = List[Node]()
        for i in range(len(node.items)):
            new_items.append(
                _process_view_tree(
                    node.items[i], events, value_bindings, attr_idx, text_idx
                )
            )
        return Node(
            kind=NODE_ELEMENT,
            tag=node.tag,
            text=String(""),
            attr_value=String(""),
            dynamic_index=0,
            operand=0,
            items=new_items^,
        )

    else:
        # Pass through unchanged (TEXT, DYN_NODE, STATIC_ATTR, DYN_ATTR)
        return node.copy()
