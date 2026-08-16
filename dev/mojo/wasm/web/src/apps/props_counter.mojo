# PropsCounterApp — self-rendering child with props (Phase 31.3).
#
# A counter app demonstrating the ChildComponentContext pattern:
#   - Parent: h1 + increment/decrement buttons + dyn_node slot
#   - Child (CounterDisplay): self-renders "Count: N" or "Count: 0xN",
#     owns a local show_hex toggle with its own button + handler
#   - Count signal shared from parent to child via context (props)
#   - Child's show_hex signal owned by child scope

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext, ChildComponentContext
from mutations import CreateEngine as _CreateEngine
from signals.handle import SignalI32 as _SignalI32, SignalBool
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
    onclick_sub as dsl_onclick_sub,
    onclick_toggle as dsl_onclick_toggle,
)

comptime _PC_PROP_COUNT: UInt32 = 1


struct CounterDisplay(Movable):
    """Self-rendering child: displays count with format toggle.

    Receives the count signal from the parent via context props.
    Owns a local show_hex: SignalBool for display format toggling.
    Renders itself via child_ctx.render_builder().
    """

    var child_ctx: ChildComponentContext
    var count: _SignalI32  # consumed from parent context
    var show_hex: SignalBool  # child-owned local state

    fn __init__(
        out self,
        var child_ctx: ChildComponentContext,
        var count: _SignalI32,
        var show_hex: SignalBool,
    ):
        self.child_ctx = child_ctx^
        self.count = count^
        self.show_hex = show_hex^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^
        self.count = other.count^
        self.show_hex = other.show_hex^

    fn render(mut self) -> UInt32:
        """Build the child's VNode with current count value."""
        var vb = self.child_ctx.render_builder()
        var val = self.count.peek()
        if self.show_hex.get():
            vb.add_dyn_text(String("Count: 0x") + String(hex(val)))
        else:
            vb.add_dyn_text(String("Count: ") + String(val))
        return vb.build()


struct PropsCounterApp(Movable):
    """Counter app demonstrating props & child-owned state.

    Parent: div > h1("Props Counter") + button("+1") + button("-1") + dyn_node[0]
    Child (CounterDisplay): div > p(dyn_text) + button("Toggle hex")

    The parent provides the count signal via context (prop).
    The child consumes it and also owns a local show_hex toggle.
    """

    var ctx: ComponentContext
    var count: _SignalI32
    var display: CounterDisplay

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        # Provide count to descendants via context
        self.ctx.provide_signal_i32(_PC_PROP_COUNT, self.count)
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Props Counter"))),
                el_button(
                    dsl_text(String("+ 1")),
                    dsl_onclick_add(self.count, 1),
                ),
                el_button(
                    dsl_text(String("- 1")),
                    dsl_onclick_sub(self.count, 1),
                ),
                dsl_dyn_node(0),
            ),
            String("props-counter"),
        )
        # Pre-create the show_hex signal so it can be referenced in
        # the child's view tree (onclick_toggle needs the key).
        var show_hex_key = self.ctx.shell.runtime[0].create_signal[Int32](
            Int32(0)
        )
        var show_hex_handle = SignalBool(show_hex_key, self.ctx.shell.runtime)
        # Create self-rendering child with format toggle button
        var child_ctx = self.ctx.create_child_context(
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Toggle hex")),
                    dsl_onclick_toggle(show_hex_handle),
                ),
            ),
            String("counter-display"),
        )
        # Subscribe the show_hex signal to the child scope so writes
        # mark the child dirty (not the parent).
        self.ctx.shell.runtime[0].signals.subscribe(
            show_hex_key, child_ctx.scope_id
        )
        var prop_count = child_ctx.consume_signal_i32(_PC_PROP_COUNT)
        self.display = CounterDisplay(child_ctx^, prop_count^, show_hex_handle^)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count^
        self.display = other.display^

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with a placeholder for the child slot."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()
        return pvb.build()


fn _pc_init() -> UnsafePointer[PropsCounterApp, MutExternalOrigin]:
    var app_ptr = alloc[PropsCounterApp](1)
    app_ptr.init_pointee_move(PropsCounterApp())
    return app_ptr


fn _pc_destroy(
    app_ptr: UnsafePointer[PropsCounterApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].display.child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _pc_rebuild(
    mut app: PropsCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the props-counter app."""
    # 1. Render parent with placeholder
    var parent_idx = app.render_parent()
    app.ctx.current_vnode = Int(parent_idx)

    # 2. Emit all templates (parent + child)
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

    # 5. Extract anchor for child slot (dyn_node[0])
    var anchor_id: UInt32 = 0
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    if vnode_ptr[0].dyn_node_id_count() > 0:
        anchor_id = vnode_ptr[0].get_dyn_node_id(0)
    app.display.child_ctx.init_slot(anchor_id)

    # 6. Build and flush child (initial render)
    var child_idx = app.display.render()
    app.display.child_ctx.flush(writer_ptr, child_idx)

    # 7. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _pc_handle_event(
    mut app: PropsCounterApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _pc_flush(
    mut app: PropsCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates."""
    var parent_dirty = app.ctx.consume_dirty()
    var child_dirty = app.display.child_ctx.is_dirty()

    if not parent_dirty and not child_dirty:
        return 0

    # Diff parent shell (placeholder → placeholder = no mutations usually)
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    # Build and flush child
    var child_idx = app.display.render()
    app.display.child_ctx.flush(writer_ptr, child_idx)

    return app.ctx.finalize(writer_ptr)
