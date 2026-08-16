# DataLoaderApp — suspense demo (Phase 33.2).
#
# Demonstrates suspense with a "Load" button that triggers pending state,
# a skeleton UI shown while pending, and a JS-triggered resolve that
# displays the loaded content.
#
# Uses the same two-slot pattern as SafeCounterApp / ErrorNestApp:
# separate dyn_node slots for content and skeleton.  On pending, the
# content slot is emptied and the skeleton slot is filled (and vice
# versa on resolve).
#
# Structure:
#   DataLoaderApp (root scope = suspense boundary)
#   ├── h1 "Data Loader"
#   ├── button "Load"  (onclick_custom → set_pending)
#   ├── dyn_node[0]  ← content slot
#   ├── dyn_node[1]  ← skeleton slot
#   │
#   DLContentChild:     p > dyn_text("Data: ...")
#   DLSkeletonChild:    p > dyn_text("Loading...")
#
# Lifecycle:
#   1. Init: content shown ("Data: (none)"), skeleton hidden
#   2. Load: set_pending(True) → next flush shows skeleton
#   3. Resolve: JS calls dl_resolve(data) → set_pending(False) → next flush
#      shows content with loaded data
#   4. Re-load: repeat cycle with new data

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext, ChildComponentContext
from mutations import CreateEngine as _CreateEngine
from html import (
    Node,
    el_div,
    el_h1,
    el_p,
    el_button,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    dyn_node as dsl_dyn_node,
    onclick_custom as dsl_onclick_custom,
)


struct DLContentChild(Movable):
    """Content child: displays loaded data.

    Template: p > dyn_text("Data: ...")
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self, data: String) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Data: ") + data)
        return vb.build()


struct DLSkeletonChild(Movable):
    """Skeleton child: loading placeholder.

    Template: p > dyn_text("Loading...")
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Loading..."))
        return vb.build()


struct DataLoaderApp(Movable):
    """Suspense demo app with load/resolve lifecycle.

    Parent: div > h1("Data Loader") + button("Load") + dyn_node[0] + dyn_node[1]
    Content: p > dyn_text("Data: ...")
    Skeleton: p > dyn_text("Loading...")
    """

    var ctx: ComponentContext
    var content: DLContentChild
    var skeleton: DLSkeletonChild
    var data_text: String
    var load_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.ctx.use_suspense_boundary()
        self.data_text = String("(none)")
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Data Loader"))),
                el_button(
                    dsl_text(String("Load")),
                    dsl_onclick_custom(),
                ),
                dsl_dyn_node(0),
                dsl_dyn_node(1),
            ),
            String("data-loader"),
        )
        self.load_handler = self.ctx.view_event_handler_id(0)
        # Content child
        var content_ctx = self.ctx.create_child_context(
            el_p(dsl_dyn_text()),
            String("dl-content"),
        )
        self.content = DLContentChild(content_ctx^)
        # Skeleton child
        var skel_ctx = self.ctx.create_child_context(
            el_p(dsl_dyn_text()),
            String("dl-skeleton"),
        )
        self.skeleton = DLSkeletonChild(skel_ctx^)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.content = other.content^
        self.skeleton = other.skeleton^
        self.data_text = other.data_text^
        self.load_handler = other.load_handler

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with placeholders for both slots."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()  # dyn_node[0] — content slot
        pvb.add_dyn_placeholder()  # dyn_node[1] — skeleton slot
        return pvb.build()


# ── DataLoaderApp lifecycle functions ────────────────────────────────────────


fn _dl_init() -> UnsafePointer[DataLoaderApp, MutExternalOrigin]:
    var app_ptr = alloc[DataLoaderApp](1)
    app_ptr.init_pointee_move(DataLoaderApp())
    return app_ptr


fn _dl_destroy(
    app_ptr: UnsafePointer[DataLoaderApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].content.child_ctx)
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].skeleton.child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _dl_rebuild(
    mut app: DataLoaderApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the data-loader app."""
    # 1. Render parent with placeholders
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

    # 5. Extract anchors for content + skeleton slots
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    var content_anchor: UInt32 = 0
    var skeleton_anchor: UInt32 = 0
    if vnode_ptr[0].dyn_node_id_count() > 0:
        content_anchor = vnode_ptr[0].get_dyn_node_id(0)
    if vnode_ptr[0].dyn_node_id_count() > 1:
        skeleton_anchor = vnode_ptr[0].get_dyn_node_id(1)
    app.content.child_ctx.init_slot(content_anchor)
    app.skeleton.child_ctx.init_slot(skeleton_anchor)

    # 6. Flush content child (initial render — no pending)
    var content_idx = app.content.render(app.data_text)
    app.content.child_ctx.flush(writer_ptr, content_idx)
    # Skeleton starts hidden — do NOT flush it

    # 7. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _dl_handle_event(
    mut app: DataLoaderApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    if handler_id == app.load_handler:
        app.ctx.set_pending(True)
        return True
    else:
        return app.ctx.dispatch_event(handler_id, event_type)


fn _dl_resolve(
    mut app: DataLoaderApp,
    data: String,
):
    """Store resolved data and clear pending state."""
    app.data_text = data
    app.ctx.set_pending(False)


fn _dl_flush(
    mut app: DataLoaderApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with suspense logic.

    Two cases:
      1. Pending: hide content, show skeleton
      2. Not pending: hide skeleton, show content with current data
    """
    var parent_dirty = app.ctx.consume_dirty()
    var content_dirty = app.content.child_ctx.is_dirty()
    var skeleton_dirty = app.skeleton.child_ctx.is_dirty()

    if not parent_dirty and not content_dirty and not skeleton_dirty:
        return 0

    # Diff parent shell (placeholders → placeholders = no mutations usually)
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    if app.ctx.is_pending():
        # ── Pending: hide content, show skeleton ─────────────────────
        app.content.child_ctx.flush_empty(writer_ptr)
        var skel_idx = app.skeleton.render()
        app.skeleton.child_ctx.flush(writer_ptr, skel_idx)
    else:
        # ── Resolved: hide skeleton, show content ────────────────────
        app.skeleton.child_ctx.flush_empty(writer_ptr)
        var content_idx = app.content.render(app.data_text)
        app.content.child_ctx.flush(writer_ptr, content_idx)

    return app.ctx.finalize(writer_ptr)
