# MultiViewApp — Single-page app with client-side routing.
#
# Phase 3.9 (Step 3.9.4): Refactored to implement the GuiApp trait.
# Free functions have been removed — the @export wrappers in
# web/src/main.mojo now use the generic gui_app_exports helpers
# which call GuiApp trait methods directly (Step 3.9.5).
#
# Phase 30: Demonstrates URL-based view switching within a single WASM
# instance.  Hosts a counter view and a todo-like view behind route
# switches, with a persistent nav bar.
#
# Architecture:
#   - Router maps "/" → branch 0 (counter), "/todo" → branch 1 (todo)
#   - Nav bar with two links rendered as buttons with onclick_custom()
#   - Content area managed by Router's ConditionalSlot (dyn_node slot)
#   - Each view has its own template registered via register_extra_template()
#   - Route transitions use flush_conditional (create/diff/remove)
#
# Template structure (main app shell):
#   div
#     nav
#       button("Counter")   ← onclick_custom → navigate to "/"
#         dynamic_attr[0]   ← onclick handler (auto-registered)
#       button("Todo")      ← onclick_custom → navigate to "/todo"
#         dynamic_attr[1]   ← onclick handler (auto-registered)
#     div                   ← content area
#       dyn_node[0]         ← routed view placeholder
#
# Counter view template ("mv-counter"):
#   div
#     h1 > dynamic_text[0]  ← "Count: N"
#     button("+ 1")
#       dynamic_attr[0]     ← onclick → increment (manually registered)
#     button("- 1")
#       dynamic_attr[1]     ← onclick → decrement (manually registered)
#
# Todo view template ("mv-todo"):
#   div
#     h2 > dynamic_text[0]  ← "Items: N"
#     button("Add item")
#       dynamic_attr[0]     ← onclick → add item
#     ul > dyn_node[0]      ← item list placeholder (simple text list)
#
# Compare with a Dioxus-style router:
#
#     fn App() -> Element {
#         rsx! {
#             Router::<Route> {}
#         }
#     }
#
#     #[derive(Routable)]
#     enum Route {
#         #[route("/")]
#         Counter {},
#         #[route("/todo")]
#         Todo {},
#     }

from memory import UnsafePointer
from bridge import MutationWriter
from component import (
    ComponentContext,
    ConditionalSlot,
    Router,
)
from component.lifecycle import flush_conditional, flush_conditional_empty
from mutations import CreateEngine
from signals import SignalI32
from signals.runtime import Runtime
from vdom import VNodeStore
from platform import GuiApp
from html import (
    Node,
    VNodeBuilder,
    el_div,
    el_nav,
    el_h1,
    el_h2,
    el_p,
    el_ul,
    el_li,
    el_button,
    text,
    dyn_text,
    dyn_node,
    dyn_attr,
    onclick_custom,
)


# ── Route branch constants ───────────────────────────────────────────────────

comptime BRANCH_COUNTER: UInt8 = 0
comptime BRANCH_TODO: UInt8 = 1


struct MultiViewApp(GuiApp):
    """Single-page app with counter and todo views behind client-side routes.

    Implements the GuiApp trait for unified cross-platform launch().

    All setup — context creation, signal creation, view registration,
    router setup, and event handler binding — happens in __init__.

    The app shell has a nav bar (two buttons) and a content area (dyn_node).
    The Router manages which view is shown based on the current path.

    Phase 30: Demonstrates the Router struct + ConditionalSlot integration
    for URL-based view switching.

    Phase 3.9: Implements GuiApp trait — mount(), handle_event(), flush(),
    has_dirty(), consume_dirty(), destroy() are now struct methods instead
    of free functions.
    """

    var ctx: ComponentContext
    var router: Router

    # Counter view state
    var count: SignalI32
    var counter_tmpl: UInt32

    # Todo view state
    var todo_count: SignalI32
    var todo_next_id: Int
    var todo_tmpl: UInt32

    # Navigation handler IDs (from the app shell's onclick_custom events)
    var nav_counter_handler: UInt32
    var nav_todo_handler: UInt32

    # Counter view handler IDs (manually registered — register_extra_template
    # does not process NODE_EVENT nodes like register_view does)
    var counter_incr_handler: UInt32
    var counter_decr_handler: UInt32

    # Todo add handler ID (from the todo view — registered as a custom handler)
    var todo_add_handler: UInt32

    fn __init__(out self):
        """Initialize the multi-view app with all reactive state and views.

        Sets up:
        1. ComponentContext with root scope
        2. App shell template (nav bar + content area)
        3. Counter view template (h1 + buttons)
        4. Todo view template (h2 + add button + list)
        5. Router with "/" → counter and "/todo" → todo
        6. Navigation and action handler IDs
        """
        self.ctx = ComponentContext.create()

        # Signals for counter view
        self.count = self.ctx.use_signal(0)

        # Signals for todo view
        self.todo_count = self.ctx.use_signal(0)
        self.todo_next_id = 0

        # App shell template — nav bar + content area with dyn_node
        # dyn_node(0) is the routed view placeholder
        self.ctx.setup_view(
            el_div(
                el_nav(
                    el_button(text(String("Counter")), onclick_custom()),
                    el_button(text(String("Todo")), onclick_custom()),
                ),
                el_div(dyn_node(0)),
            ),
            String("mv-app"),
        )

        # Retrieve navigation handler IDs (tree-walk order: counter=0, todo=1)
        self.nav_counter_handler = self.ctx.view_event_handler_id(0)
        self.nav_todo_handler = self.ctx.view_event_handler_id(1)

        # Counter view template — standalone template for the counter branch.
        # NOTE: register_extra_template does NOT process NODE_EVENT nodes
        # (onclick_add/onclick_sub), so we use dyn_attr placeholders and
        # register the handlers manually below.
        self.counter_tmpl = self.ctx.register_extra_template(
            el_div(
                el_h1(dyn_text(0)),
                el_button(text(String("+ 1")), dyn_attr(0)),
                el_button(text(String("- 1")), dyn_attr(1)),
            ),
            String("mv-counter"),
        )

        # Manually register counter signal handlers (same as what
        # register_view would auto-do for onclick_add/onclick_sub)
        self.counter_incr_handler = self.ctx.on_click_add(self.count, 1)
        self.counter_decr_handler = self.ctx.on_click_sub(self.count, 1)

        # Todo view template — standalone template for the todo branch
        # Uses onclick_custom for the add button (app handles it)
        self.todo_tmpl = self.ctx.register_extra_template(
            el_div(
                el_h2(dyn_text(0)),
                el_button(text(String("Add item")), onclick_custom()),
                el_p(dyn_text(1)),
            ),
            String("mv-todo"),
        )

        # The todo add handler must be registered manually since it's not
        # part of the main view (it's in an extra template).  We register
        # it under the root scope with ACTION_CUSTOM.
        self.todo_add_handler = self.ctx.register_custom_handler(
            String("click")
        )

        # Set up router
        self.router = Router()
        self.router.add_route(String("/"), BRANCH_COUNTER)
        self.router.add_route(String("/todo"), BRANCH_TODO)
        # Navigate to initial route
        _ = self.router.navigate(String("/"))

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.router = other.router^
        self.count = other.count^
        self.counter_tmpl = other.counter_tmpl
        self.counter_incr_handler = other.counter_incr_handler
        self.counter_decr_handler = other.counter_decr_handler
        self.todo_count = other.todo_count^
        self.todo_next_id = other.todo_next_id
        self.todo_tmpl = other.todo_tmpl
        self.nav_counter_handler = other.nav_counter_handler
        self.nav_todo_handler = other.nav_todo_handler
        self.todo_add_handler = other.todo_add_handler

    fn render(mut self) -> UInt32:
        """Build a fresh VNode for the app shell.

        The app shell always renders a placeholder for dyn_node[0] — the
        Router's ConditionalSlot manages the actual routed content.

        Returns the VNode index in the store.
        """
        var vb = self.ctx.render_builder()
        # dyn_node[0] — placeholder for routed content
        vb.add_dyn_placeholder()
        return vb.build()

    fn build_counter_view(mut self) -> UInt32:
        """Build the counter view VNode.

        Returns the VNode index in the store.
        """
        var vb = VNodeBuilder(self.counter_tmpl, self.ctx.store_ptr())
        vb.add_dyn_text(String("Count: ") + String(self.count.peek()))
        # Manually add event handlers for +1 and -1 buttons (dyn_attr slots 0, 1)
        vb.add_dyn_event(String("click"), self.counter_incr_handler)
        vb.add_dyn_event(String("click"), self.counter_decr_handler)
        return vb.index()

    fn build_todo_view(mut self) -> UInt32:
        """Build the todo view VNode.

        Returns the VNode index in the store.
        """
        var count = self.todo_count.peek()
        var vb = VNodeBuilder(self.todo_tmpl, self.ctx.store_ptr())
        vb.add_dyn_text(String("Items: ") + String(count))
        # dyn_text[1] — item listing summary
        if count == 0:
            vb.add_dyn_text(String("No items yet — click Add!"))
        else:
            var items = String("")
            for i in range(count):
                if i > 0:
                    items = items + String(", ")
                items = items + String("Item ") + String(i + 1)
            vb.add_dyn_text(items)
        # dyn_attr for add button — manually add handler
        vb.add_dyn_event(String("click"), self.todo_add_handler)
        return vb.index()

    fn build_view_for_branch(mut self) -> UInt32:
        """Build the VNode for the currently active branch.

        Returns the VNode index in the store.
        """
        if self.router.current == BRANCH_COUNTER:
            return self.build_counter_view()
        elif self.router.current == BRANCH_TODO:
            return self.build_todo_view()
        # Fallback — should not happen with well-formed routes
        return self.build_counter_view()

    fn navigate(mut self, path: String) -> Bool:
        """Navigate to a URL path.

        Updates the router and marks the app scope as dirty so the next
        flush will rebuild the view.

        Args:
            path: The URL path to navigate to (e.g. "/" or "/todo").

        Returns:
            True if the path matched a registered route.
        """
        var result = self.router.navigate(path)
        if result and self.router.dirty:
            # Mark the app scope dirty so flush picks up the change
            self.ctx.mark_dirty()
        return result

    # ── GuiApp trait: Event dispatch ─────────────────────────────────

    fn handle_event(
        mut self, handler_id: UInt32, event_type: UInt8, value: String
    ) -> Bool:
        """Dispatch a user interaction event (GuiApp trait method).

        Unified event entry point. Routes navigation clicks, todo add,
        and falls back to signal-based handlers (counter +1/-1) via
        ComponentContext dispatch.

        The `value` parameter is accepted for GuiApp trait conformance.
        If a non-empty value is provided, it is dispatched via the string
        path for forward compatibility.

        Args:
            handler_id: The handler to invoke (from HandlerRegistry).
            event_type: The event type tag (EVT_CLICK, etc.).
            value: String payload from the event. Empty for click events.

        Returns:
            True if the handler was recognized and acted upon.
        """
        # String events go through the string dispatch path
        if len(value) > 0:
            return self.ctx.dispatch_event_with_string(
                handler_id, event_type, value
            )

        # App-level routing (nav clicks, todo add)
        if handler_id == self.nav_counter_handler:
            _ = self.navigate(String("/"))
            return True
        elif handler_id == self.nav_todo_handler:
            _ = self.navigate(String("/todo"))
            return True
        elif handler_id == self.todo_add_handler:
            # Add a todo item
            self.todo_next_id += 1
            self.todo_count.set(self.todo_count.peek() + 1)
            return True

        # Fall back to signal-based handlers (counter +1/-1)
        return self.ctx.dispatch_event(handler_id, event_type)

    # ── GuiApp trait: Mount lifecycle ────────────────────────────────

    fn mount(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ) -> Int32:
        """Initial render (mount) of the multi-view app (GuiApp trait method).

        Emits RegisterTemplate mutations for all templates (app shell +
        counter view + todo view), mounts the app shell, then immediately
        flushes the initial route's view into the ConditionalSlot — all
        in one mutation buffer before finalize.

        Uses a manual mount sequence (emit_templates + CreateEngine +
        AppendChildren) instead of ctx.mount() so that the initial route
        view can be flushed into the ConditionalSlot BEFORE the End
        sentinel is written.

        Args:
            writer_ptr: Pointer to the MutationWriter for the mutation
                        buffer.

        Returns the byte offset (length) of the mutation data written.
        """
        # 1. Render the app shell VNode
        var vnode_idx = self.render()
        self.ctx.current_vnode = Int(vnode_idx)

        # 2. Emit templates (without finalize)
        self.ctx.shell.emit_templates(writer_ptr)

        # 3. Create the app shell VNode in the DOM (without finalize)
        var engine = CreateEngine(
            writer_ptr,
            self.ctx.shell.eid_alloc,
            self.ctx.shell.runtime,
            self.ctx.shell.store,
        )
        var num_roots = engine.create_node(vnode_idx)
        writer_ptr[0].append_children(0, num_roots)

        # 4. Extract the anchor ElementId for the router's ConditionalSlot
        #    dyn_node[0] is the routed content placeholder
        var anchor_id: UInt32 = 0
        var app_vnode_ptr = self.ctx.store_ptr()[0].get_ptr(vnode_idx)
        if app_vnode_ptr[0].dyn_node_id_count() > 0:
            anchor_id = app_vnode_ptr[0].get_dyn_node_id(0)
        self.router.init_slot(anchor_id)

        # 5. Build and flush the initial route's view (still before finalize)
        var view_idx = self.build_view_for_branch()
        self.router.slot = self.ctx.flush_conditional_slot(
            writer_ptr, self.router.slot, view_idx
        )
        # Consume the dirty flag from initial navigate
        _ = self.router.consume_dirty()

        # 6. Finalize — one End sentinel for the entire mount + initial view
        writer_ptr[0].finalize()
        return Int32(writer_ptr[0].offset)

    # ── GuiApp trait: Flush lifecycle ────────────────────────────────

    fn flush(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ) -> Int32:
        """Re-render dirty scopes and write update mutations (GuiApp trait method).

        Handles two kinds of updates:
        1. Route change: rebuild the view for the new branch via ConditionalSlot
        2. In-view update: re-render + diff the current view (e.g. counter ±1)

        Args:
            writer_ptr: Pointer to the MutationWriter for the mutation
                        buffer (reset to offset 0 by the caller).

        Returns the byte offset (length) of the mutation data written,
        or 0 if there was nothing to update.
        """
        var route_changed = self.router.consume_dirty()
        var scope_dirty = self.ctx.consume_dirty()

        if not route_changed and not scope_dirty:
            return 0

        # 1. Re-render and diff the app shell (updates dyn_node placeholder)
        var new_idx = self.render()
        self.ctx.diff(writer_ptr, new_idx)

        # 2. Handle routed content
        if route_changed:
            # Route changed — build new view for the target branch
            var view_idx = self.build_view_for_branch()
            self.router.slot = self.ctx.flush_conditional_slot(
                writer_ptr, self.router.slot, view_idx
            )
        elif scope_dirty:
            # Same route but data changed — rebuild current view and diff
            var view_idx = self.build_view_for_branch()
            self.router.slot = self.ctx.flush_conditional_slot(
                writer_ptr, self.router.slot, view_idx
            )

        # 3. Finalize mutation buffer
        return self.ctx.finalize(writer_ptr)

    # ── GuiApp trait: Dirty state queries ────────────────────────────

    fn has_dirty(self) -> Bool:
        """Check if any scopes or the router need re-rendering.

        Returns:
            True if at least one scope is marked dirty or the router
            has a pending route change.
        """
        return self.ctx.has_dirty() or self.router.dirty

    fn consume_dirty(mut self) -> Bool:
        """Collect and consume all dirty scopes.

        Note: This only consumes scope-level dirty state. Router dirty
        state is consumed separately inside flush() via
        router.consume_dirty(). This method is provided for GuiApp
        trait conformance; the event loop should use has_dirty() to
        check whether flush() should be called.

        Returns:
            True if any scopes were dirty (and consumed).
        """
        return self.ctx.consume_dirty()

    # ── GuiApp trait: Cleanup ────────────────────────────────────────

    fn destroy(mut self):
        """Release all resources held by the multi-view app."""
        self.ctx.destroy()
