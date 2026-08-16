# ChildCounterApp — child component composition demo (Phase 29).
#
# A counter app that demonstrates child component composition.
# The parent owns buttons (increment/decrement) and a dyn_node slot.
# A ChildComponent renders the display: <p>Count: N</p>.
#
# Template structure:
#   div                          ← parent template
#     h1 > "Child Counter"       ← static heading
#     button > "Up"              ← onclick_add(count, 1)
#       dynamic_attr[0]
#     button > "Down"            ← onclick_sub(count, 1)
#       dynamic_attr[1]
#     dyn_node[0]                ← child component slot
#
# Child template ("child-display"):
#   p > dynamic_text[0]          ← "Count: N"

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext, ChildComponent
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
    onclick_sub as dsl_onclick_sub,
)


struct ChildCounterApp(Movable):
    """Counter app with the display extracted into a child component.

    Demonstrates the component composition pattern:
    - Parent: toolbar buttons + dyn_node slot (placeholder)
    - Child: `<p>Count: N</p>` with its own scope, managed via ConditionalSlot

    The parent always renders a placeholder for dyn_node[0].  After mount,
    the anchor ElementId is extracted and the child's ConditionalSlot is
    initialized.  On each flush, the child builds its VNode and flushes
    it via flush_conditional (create on first flush, diff on subsequent).
    """

    var ctx: ComponentContext
    var count: _SignalI32
    var child: ChildComponent

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Child Counter"))),
                el_button(
                    dsl_text(String("Up")),
                    dsl_onclick_add(self.count, 1),
                ),
                el_button(
                    dsl_text(String("Down")),
                    dsl_onclick_sub(self.count, 1),
                ),
                dsl_dyn_node(0),
            ),
            String("child-counter"),
        )
        # Create child component with its own scope and template
        self.child = self.ctx.create_child_component(
            el_p(dsl_dyn_text()),
            String("child-display"),
        )

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count^
        self.child = other.child^

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with a placeholder for the child slot.

        The child's content is managed separately via ChildComponent.flush().
        """
        var pvb = self.ctx.render_builder()
        # dyn_node[0] — placeholder for child component
        pvb.add_dyn_placeholder()
        return pvb.build()

    fn build_child_vnode(mut self) -> UInt32:
        """Build the child's VNode with current count value.

        Returns the VNode index for use with child.flush().
        """
        var cvb = self.child.render_builder(
            self.ctx.store_ptr(), self.ctx.runtime_ptr()
        )
        cvb.add_dyn_text(String("Count: ") + String(self.count.peek()))
        return cvb.build()


fn _cc_init() -> UnsafePointer[ChildCounterApp, MutExternalOrigin]:
    var app_ptr = alloc[ChildCounterApp](1)
    app_ptr.init_pointee_move(ChildCounterApp())
    return app_ptr


fn _cc_destroy(app_ptr: UnsafePointer[ChildCounterApp, MutExternalOrigin]):
    app_ptr[0].ctx.destroy_child_component(app_ptr[0].child)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _cc_rebuild(
    mut app: ChildCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the child-counter app.

    Manually performs mount steps WITHOUT intermediate finalize so that
    child flush mutations are included in the same buffer:

    1. Emit templates (RegisterTemplate mutations for both templates).
    2. Create parent VNode tree via CreateEngine.
    3. Append to root element (id 0).
    4. Extract the anchor ElementId from dyn_node_ids[0].
    5. Initialize the child's ConditionalSlot with the anchor.
    6. Build the child VNode and flush it (creates child DOM,
       replaces the placeholder).
    7. Finalize the mutation buffer (single OP_END).
    """
    # 1. Render parent with placeholder for child slot
    var parent_idx = app.render_parent()
    app.ctx.current_vnode = Int(parent_idx)

    # 2. Emit all registered templates (parent + child)
    app.ctx.shell.emit_templates(writer_ptr)

    # 3. Create parent VNode tree (no finalize)
    var engine = _CreateEngine(
        writer_ptr,
        app.ctx.shell.eid_alloc,
        app.ctx.runtime_ptr(),
        app.ctx.store_ptr(),
    )
    var num_roots = engine.create_node(parent_idx)

    # 4. Append to root element (id 0)
    writer_ptr[0].append_children(0, num_roots)

    # 5. Extract anchor for the child slot (dyn_node[0])
    var anchor_id: UInt32 = 0
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    if vnode_ptr[0].dyn_node_id_count() > 0:
        anchor_id = vnode_ptr[0].get_dyn_node_id(0)
    app.child.init_slot(anchor_id)

    # 6. Build and flush the child VNode (create child DOM, replace placeholder)
    var child_idx = app.build_child_vnode()
    app.child.flush(
        writer_ptr,
        app.ctx.shell.eid_alloc,
        app.ctx.runtime_ptr(),
        app.ctx.store_ptr(),
        child_idx,
    )

    # 7. Single finalize for the entire mount + child flush
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _cc_handle_event(
    mut app: ChildCounterApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _cc_flush(
    mut app: ChildCounterApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates.

    1. Diff the parent shell (placeholder stays as placeholder → 0 mutations).
    2. Flush the child VNode (diff old vs new → SetText if count changed).
    3. Finalize the mutation buffer.
    """
    if not app.ctx.consume_dirty():
        return 0

    # 1. Diff parent shell (placeholder → placeholder = no mutations)
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    # 2. Build and flush child VNode
    var child_idx = app.build_child_vnode()
    app.child.flush(
        writer_ptr,
        app.ctx.shell.eid_alloc,
        app.ctx.runtime_ptr(),
        app.ctx.store_ptr(),
        child_idx,
    )

    # 3. Finalize
    return app.ctx.finalize(writer_ptr)
