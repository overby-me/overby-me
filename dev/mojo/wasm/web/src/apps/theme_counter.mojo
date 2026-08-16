# ThemeCounterApp — shared context + cross-component demo (Phase 31.4).
#
# A parent app with a theme toggle (dark/light) and two child components
# that both consume the theme context:
#   - CounterChild: displays count with theme-dependent label
#   - SummaryChild: displays summary text with theme-dependent class
#   - Parent receives upward communication: CounterChild has a "Reset"
#     button that writes to a callback signal consumed by the parent.
#
# Template structure:
#   Parent: div > button("Toggle theme") + button("Increment") + dyn_node[0] + dyn_node[1]
#   CounterChild: div > p(dyn_text) + button("Reset")
#   SummaryChild: p(dyn_text, dyn_attr[0])

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
    dyn_attr as dsl_dyn_attr,
    onclick_add as dsl_onclick_add,
    onclick_toggle as dsl_onclick_toggle,
    onclick_set as dsl_onclick_set,
)


comptime _TC_CTX_THEME: UInt32 = 10  # 0 = light, 1 = dark
comptime _TC_CTX_COUNT: UInt32 = 11  # count signal key
comptime _TC_CTX_ON_RESET: UInt32 = 12  # callback signal key


struct TCCounterChild(Movable):
    """Child component: displays count with theme-dependent label.

    Consumes CTX_COUNT, CTX_THEME from parent context.
    Has a Reset button that writes to CTX_ON_RESET callback signal.
    Template: div > p(dyn_text) + button("Reset")
    """

    var child_ctx: ChildComponentContext
    var count: _SignalI32  # consumed from parent
    var theme: SignalBool  # consumed from parent

    fn __init__(
        out self,
        var child_ctx: ChildComponentContext,
        var count: _SignalI32,
        var theme: SignalBool,
    ):
        self.child_ctx = child_ctx^
        self.count = count^
        self.theme = theme^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^
        self.count = other.count^
        self.theme = other.theme^

    fn render(mut self) -> UInt32:
        """Build the child's VNode with count + optional theme label."""
        var vb = self.child_ctx.render_builder()
        var val = self.count.peek()
        if self.theme.get():
            vb.add_dyn_text(String("Theme: dark, Count: ") + String(val))
        else:
            vb.add_dyn_text(String("Count: ") + String(val))
        return vb.build()


struct TCSummaryChild(Movable):
    """Child component: displays summary text with theme-dependent class.

    Consumes CTX_COUNT, CTX_THEME from parent context.
    Template: p(dyn_text, dyn_attr[0])
    """

    var child_ctx: ChildComponentContext
    var count: _SignalI32  # consumed from parent
    var theme: SignalBool  # consumed from parent

    fn __init__(
        out self,
        var child_ctx: ChildComponentContext,
        var count: _SignalI32,
        var theme: SignalBool,
    ):
        self.child_ctx = child_ctx^
        self.count = count^
        self.theme = theme^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^
        self.count = other.count^
        self.theme = other.theme^

    fn render(mut self) -> UInt32:
        """Build the child's VNode with summary text + class attr."""
        var vb = self.child_ctx.render_builder()
        var val = self.count.peek()
        vb.add_dyn_text(String(val) + String(" clicks so far"))
        if self.theme.get():
            vb.add_dyn_text_attr(String("class"), String("dark"))
        else:
            vb.add_dyn_text_attr(String("class"), String("light"))
        return vb.build()


struct ThemeCounterApp(Movable):
    """Counter app with theme toggle and two child components sharing context.

    Parent: div > button("Toggle theme") + button("Increment") + dyn_node[0] + dyn_node[1]
    CounterChild: div > p(dyn_text) + button("Reset")
    SummaryChild: p(dyn_text, dyn_attr[0])

    Demonstrates:
    - Multiple children consuming the same parent context signals
    - Upward communication via callback signal (reset)
    - Theme-dependent rendering in children
    """

    var ctx: ComponentContext
    var count: _SignalI32
    var theme: SignalBool
    var on_reset: _SignalI32  # callback: child writes 1 to request reset
    var counter_child: TCCounterChild
    var summary_child: TCSummaryChild

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.theme = self.ctx.use_signal_bool(False)
        self.on_reset = self.ctx.use_signal(0)

        # Provide signals to descendants via context
        self.ctx.provide_signal_i32(_TC_CTX_COUNT, self.count)
        self.ctx.provide_signal_bool(_TC_CTX_THEME, self.theme)
        self.ctx.provide_signal_i32(_TC_CTX_ON_RESET, self.on_reset)

        self.ctx.setup_view(
            el_div(
                el_button(
                    dsl_text(String("Toggle theme")),
                    dsl_onclick_toggle(self.theme),
                ),
                el_button(
                    dsl_text(String("Increment")),
                    dsl_onclick_add(self.count, 1),
                ),
                dsl_dyn_node(0),  # counter child slot
                dsl_dyn_node(1),  # summary child slot
            ),
            String("theme-counter"),
        )

        # Create counter child: div > p(dyn_text) + button("Reset")
        var counter_ctx = self.ctx.create_child_context(
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Reset")),
                    dsl_onclick_set(self.on_reset, 1),
                ),
            ),
            String("counter-display"),
        )
        var cc_count = counter_ctx.consume_signal_i32(_TC_CTX_COUNT)
        var cc_theme = counter_ctx.consume_signal_bool(_TC_CTX_THEME)
        self.counter_child = TCCounterChild(counter_ctx^, cc_count^, cc_theme^)

        # Create summary child: p(dyn_text, dyn_attr[0])
        var summary_ctx = self.ctx.create_child_context(
            el_p(dsl_dyn_text(), dsl_dyn_attr(0)),
            String("summary-display"),
        )
        var sc_count = summary_ctx.consume_signal_i32(_TC_CTX_COUNT)
        var sc_theme = summary_ctx.consume_signal_bool(_TC_CTX_THEME)
        self.summary_child = TCSummaryChild(summary_ctx^, sc_count^, sc_theme^)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count^
        self.theme = other.theme^
        self.on_reset = other.on_reset^
        self.counter_child = other.counter_child^
        self.summary_child = other.summary_child^

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with placeholders for child slots."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()  # dyn_node[0] — counter child
        pvb.add_dyn_placeholder()  # dyn_node[1] — summary child
        return pvb.build()


fn _tc_init() -> UnsafePointer[ThemeCounterApp, MutExternalOrigin]:
    var app_ptr = alloc[ThemeCounterApp](1)
    app_ptr.init_pointee_move(ThemeCounterApp())
    return app_ptr


fn _tc_destroy(
    app_ptr: UnsafePointer[ThemeCounterApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].counter_child.child_ctx)
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].summary_child.child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _tc_rebuild(
    mut app: ThemeCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the theme-counter app."""
    # 1. Render parent with placeholders
    var parent_idx = app.render_parent()
    app.ctx.current_vnode = Int(parent_idx)

    # 2. Emit all templates (parent + both children)
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

    # 5. Extract anchors for child slots (dyn_node[0] and dyn_node[1])
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    var counter_anchor: UInt32 = 0
    var summary_anchor: UInt32 = 0
    if vnode_ptr[0].dyn_node_id_count() > 0:
        counter_anchor = vnode_ptr[0].get_dyn_node_id(0)
    if vnode_ptr[0].dyn_node_id_count() > 1:
        summary_anchor = vnode_ptr[0].get_dyn_node_id(1)
    app.counter_child.child_ctx.init_slot(counter_anchor)
    app.summary_child.child_ctx.init_slot(summary_anchor)

    # 6. Build and flush counter child (initial render)
    var counter_idx = app.counter_child.render()
    app.counter_child.child_ctx.flush(writer_ptr, counter_idx)

    # 7. Build and flush summary child (initial render)
    var summary_idx = app.summary_child.render()
    app.summary_child.child_ctx.flush(writer_ptr, summary_idx)

    # 8. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _tc_handle_event(
    mut app: ThemeCounterApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _tc_flush(
    mut app: ThemeCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates, handling reset callback."""
    # Check for reset callback from counter child
    if app.on_reset.peek() != 0:
        app.count.set(0)
        app.on_reset.set(0)

    var parent_dirty = app.ctx.consume_dirty()
    var counter_dirty = app.counter_child.child_ctx.is_dirty()
    var summary_dirty = app.summary_child.child_ctx.is_dirty()

    if not parent_dirty and not counter_dirty and not summary_dirty:
        return 0

    # Diff parent shell
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    # Build and flush counter child
    var counter_idx = app.counter_child.render()
    app.counter_child.child_ctx.flush(writer_ptr, counter_idx)

    # Build and flush summary child
    var summary_idx = app.summary_child.render()
    app.summary_child.child_ctx.flush(writer_ptr, summary_idx)

    return app.ctx.finalize(writer_ptr)
