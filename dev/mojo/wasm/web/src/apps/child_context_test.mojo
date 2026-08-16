# ChildContextTestApp — ChildComponentContext test harness (Phase 31.2).
#
# A test app demonstrating ChildComponentContext — a self-rendering child
# component with its own signals, context consumption, and rendering.
# The parent provides a count signal via context; the child consumes it
# and also owns a local bool signal (show_hex toggle).

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext, ChildComponentContext
from mutations import CreateEngine as _CreateEngine
from signals.handle import SignalI32 as _SignalI32, SignalBool
from html import (
    el_div,
    el_h1,
    el_p,
    el_button,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    dyn_node as dsl_dyn_node,
    onclick_add as dsl_onclick_add,
)


comptime _CCT_PROP_COUNT: UInt32 = 1


struct ChildContextTestApp(Movable):
    """Test app for ChildComponentContext.

    Parent: root scope with count signal, provided via context.
    Child: ChildComponentContext with consumed count + local show_hex signal.
    """

    var ctx: ComponentContext
    var count: _SignalI32
    var child_ctx: ChildComponentContext
    var child_count: _SignalI32  # consumed from parent context
    var child_show_hex: SignalBool  # child-owned local state

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("ChildContext Test"))),
                el_button(
                    dsl_text(String("+")),
                    dsl_onclick_add(self.count, 1),
                ),
                dsl_dyn_node(0),
            ),
            String("cct-parent"),
        )
        # Provide count signal to descendants
        self.ctx.provide_signal_i32(_CCT_PROP_COUNT, self.count)
        # Create self-rendering child with toggle
        self.child_ctx = self.ctx.create_child_context(
            el_p(dsl_dyn_text()),
            String("cct-child"),
        )
        self.child_count = self.child_ctx.consume_signal_i32(_CCT_PROP_COUNT)
        self.child_show_hex = self.child_ctx.use_signal_bool(False)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count^
        self.child_ctx = other.child_ctx^
        self.child_count = other.child_count^
        self.child_show_hex = other.child_show_hex^

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with a placeholder for the child slot."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()
        return pvb.build()

    fn render_child(mut self) -> UInt32:
        """Build the child's VNode with current count value."""
        var cvb = self.child_ctx.render_builder()
        var val = self.child_count.peek()
        if self.child_show_hex.get():
            cvb.add_dyn_text(String("Count: 0x") + String(hex(val)))
        else:
            cvb.add_dyn_text(String("Count: ") + String(val))
        return cvb.build()


fn _cct_init() -> UnsafePointer[ChildContextTestApp, MutExternalOrigin]:
    var app_ptr = alloc[ChildContextTestApp](1)
    app_ptr.init_pointee_move(ChildContextTestApp())
    return app_ptr


fn _cct_destroy(
    app_ptr: UnsafePointer[ChildContextTestApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _cct_rebuild(
    mut app: ChildContextTestApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the child-context test app."""
    # 1. Render parent with placeholder
    var parent_idx = app.render_parent()
    app.ctx.current_vnode = Int(parent_idx)

    # 2. Emit all templates
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

    # 5. Extract anchor for child slot
    var anchor_id: UInt32 = 0
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    if vnode_ptr[0].dyn_node_id_count() > 0:
        anchor_id = vnode_ptr[0].get_dyn_node_id(0)
    app.child_ctx.init_slot(anchor_id)

    # 6. Build and flush child
    var child_idx = app.render_child()
    app.child_ctx.flush(writer_ptr, child_idx)

    # 7. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _cct_handle_event(
    mut app: ChildContextTestApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _cct_flush(
    mut app: ChildContextTestApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates."""
    var parent_dirty = app.ctx.consume_dirty()
    var child_dirty = app.child_ctx.is_dirty()

    if not parent_dirty and not child_dirty:
        return 0

    # Diff parent shell
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    # Build and flush child
    var child_idx = app.render_child()
    app.child_ctx.flush(writer_ptr, child_idx)

    return app.ctx.finalize(writer_ptr)
