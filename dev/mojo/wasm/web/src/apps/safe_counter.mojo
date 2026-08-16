# SafeCounterApp — error boundary demo (Phase 32.2).
#
# A counter app with an error boundary.  The parent has a count signal,
# increment/crash buttons, and two child components occupying two dyn_node
# slots.  The "normal" child displays the count; the "fallback" child
# shows the error message and a Retry button.
#
# On Crash: parent calls report_error() → has_error() becomes True →
#   flush hides normal child, shows fallback with error message.
# On Retry: parent calls clear_error() → has_error() becomes False →
#   flush hides fallback, shows normal child (re-creates from scratch).
# Count signal persists across crash/recovery since it lives on the parent.

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext, ChildComponentContext
from mutations import CreateEngine as _CreateEngine
from signals.handle import SignalI32 as _SignalI32
from html import (
    Node,
    el_div,
    el_h1,
    el_p,
    el_button,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    dyn_node as dsl_dyn_node,
    onclick_add as dsl_onclick_add,
    onclick_custom as dsl_onclick_custom,
)


comptime _SC_PROP_COUNT: UInt32 = 20


struct SCNormalChild(Movable):
    """Normal content child: displays count.

    Template: p > dyn_text("Count: N")
    Consumes count signal from parent context.
    """

    var child_ctx: ChildComponentContext
    var count: _SignalI32

    fn __init__(
        out self,
        var child_ctx: ChildComponentContext,
        var count: _SignalI32,
    ):
        self.child_ctx = child_ctx^
        self.count = count^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^
        self.count = other.count^

    fn render(mut self) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Count: ") + String(self.count.peek()))
        return vb.build()


struct SCFallbackChild(Movable):
    """Fallback child: shows error message + retry button.

    Template: div > p(dyn_text("Error: ...")) + button("Retry", onclick_custom)
    The retry button's onclick_custom handler is registered under the
    fallback child's scope by create_child_context.  The parent routes
    the handler ID in _sc_handle_event.
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self, error_msg: String) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Error: ") + error_msg)
        return vb.build()


struct SafeCounterApp(Movable):
    """Counter app with error boundary.

    Parent template:
        div > h1("Safe Counter") + button("+1") + button("Crash", onclick_custom)
              + dyn_node[0] + dyn_node[1]

    dyn_node[0] = normal child slot (p > dyn_text)
    dyn_node[1] = fallback child slot (div > p(dyn_text) + button("Retry"))

    The Crash button triggers report_error().  The parent catches it
    and swaps to fallback UI.  Retry clears the error and restores
    normal rendering.  Count signal persists across crash/recovery.
    """

    var ctx: ComponentContext
    var count: _SignalI32
    var normal: SCNormalChild
    var fallback: SCFallbackChild
    var crash_handler: UInt32
    var retry_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.use_error_boundary()
        self.ctx.provide_signal_i32(_SC_PROP_COUNT, self.count)
        # Use onclick_custom() in the view tree for the crash button.
        # setup_view() will auto-register the handler under the root scope.
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Safe Counter"))),
                el_button(
                    dsl_text(String("+ 1")),
                    dsl_onclick_add(self.count, 1),
                ),
                el_button(
                    dsl_text(String("Crash")),
                    dsl_onclick_custom(),
                ),
                dsl_dyn_node(0),
                dsl_dyn_node(1),
            ),
            String("safe-counter"),
        )
        # Crash handler is the first onclick_custom in the view tree
        # (onclick_add is index 0, onclick_custom is index 1).
        self.crash_handler = self.ctx.view_event_handler_id(1)

        # Normal child: displays count
        var normal_ctx = self.ctx.create_child_context(
            el_p(dsl_dyn_text()),
            String("sc-normal"),
        )
        var prop_count = normal_ctx.consume_signal_i32(_SC_PROP_COUNT)
        self.normal = SCNormalChild(normal_ctx^, prop_count^)

        # Fallback child: shows error + retry button.
        # onclick_custom() in the child view tree gets auto-registered
        # under the fallback child's scope by create_child_context.
        var fallback_ctx = self.ctx.create_child_context(
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Retry")),
                    dsl_onclick_custom(),
                ),
            ),
            String("sc-fallback"),
        )
        # Retry handler is the first (and only) event on the fallback child
        self.retry_handler = fallback_ctx.event_handler_id(0)
        self.fallback = SCFallbackChild(fallback_ctx^)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count^
        self.normal = other.normal^
        self.fallback = other.fallback^
        self.crash_handler = other.crash_handler
        self.retry_handler = other.retry_handler

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with placeholders for both child slots."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()  # dyn_node[0] — normal child
        pvb.add_dyn_placeholder()  # dyn_node[1] — fallback child
        return pvb.build()


fn _sc_init() -> UnsafePointer[SafeCounterApp, MutExternalOrigin]:
    var app_ptr = alloc[SafeCounterApp](1)
    app_ptr.init_pointee_move(SafeCounterApp())
    return app_ptr


fn _sc_destroy(
    app_ptr: UnsafePointer[SafeCounterApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].normal.child_ctx)
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].fallback.child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _sc_rebuild(
    mut app: SafeCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the safe-counter app."""
    # 1. Render parent with placeholders
    var parent_idx = app.render_parent()
    app.ctx.current_vnode = Int(parent_idx)

    # 2. Emit all templates (parent + normal child + fallback child)
    app.ctx.shell.emit_templates(writer_ptr)

    # 3. Create parent VNode tree
    var engine = _CreateEngine(
        writer_ptr,
        app.ctx.shell.eid_alloc,
        app.ctx.runtime_ptr(),
        app.ctx.store_ptr(),
    )
    var num_roots = engine.create_node(parent_idx)

    # 4. Append to root element
    writer_ptr[0].append_children(0, num_roots)

    # 5. Extract anchors for child slots
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    var normal_anchor: UInt32 = 0
    var fallback_anchor: UInt32 = 0
    if vnode_ptr[0].dyn_node_id_count() > 0:
        normal_anchor = vnode_ptr[0].get_dyn_node_id(0)
    if vnode_ptr[0].dyn_node_id_count() > 1:
        fallback_anchor = vnode_ptr[0].get_dyn_node_id(1)
    app.normal.child_ctx.init_slot(normal_anchor)
    app.fallback.child_ctx.init_slot(fallback_anchor)

    # 6. Flush normal child (initial render — no error state)
    var normal_idx = app.normal.render()
    app.normal.child_ctx.flush(writer_ptr, normal_idx)
    # Fallback starts hidden — do NOT flush it

    # 7. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _sc_handle_event(
    mut app: SafeCounterApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    if handler_id == app.crash_handler:
        # Simulate a crash: propagate error to this boundary
        _ = app.ctx.report_error(String("Simulated crash"))
        return True
    elif handler_id == app.retry_handler:
        # Clear error state — next flush restores normal content
        app.ctx.clear_error()
        return True
    else:
        return app.ctx.dispatch_event(handler_id, event_type)


fn _sc_flush(
    mut app: SafeCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with error boundary logic."""
    var parent_dirty = app.ctx.consume_dirty()
    var normal_dirty = app.normal.child_ctx.is_dirty()
    var fallback_dirty = app.fallback.child_ctx.is_dirty()

    if not parent_dirty and not normal_dirty and not fallback_dirty:
        return 0

    # Diff parent shell (placeholder → placeholder = no mutations usually)
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    if app.ctx.has_error():
        # Error state: hide normal, show fallback with error message
        app.normal.child_ctx.flush_empty(writer_ptr)
        var fb_idx = app.fallback.render(app.ctx.error_message())
        app.fallback.child_ctx.flush(writer_ptr, fb_idx)
    else:
        # Normal state: hide fallback, show normal
        app.fallback.child_ctx.flush_empty(writer_ptr)
        var normal_idx = app.normal.render()
        app.normal.child_ctx.flush(writer_ptr, normal_idx)

    return app.ctx.finalize(writer_ptr)
