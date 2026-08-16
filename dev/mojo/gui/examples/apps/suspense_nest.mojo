# SuspenseNestApp — nested suspense boundaries demo (Phase 33.3).
#
# Demonstrates nested suspense boundaries where inner and outer boundaries
# independently show/hide skeletons based on their descendants' pending
# states.
#
# Uses the same two-slot pattern as DataLoaderApp / ErrorNestApp: each
# boundary level has separate dyn_node slots for content and skeleton.
# On pending, the content slot is emptied and the skeleton slot is filled
# (and vice versa on resolve).
#
# Structure:
#   SuspenseNestApp (outer boundary)
#   ├── h1 "Nested Suspense"
#   ├── button "Outer Load"   (sets outer pending)
#   ├── dyn_node[0]  ← outer content slot
#   ├── dyn_node[1]  ← outer skeleton slot
#   │
#   SNOuterContentChild (inner boundary)
#   ├── p > dyn_text("Outer: {data}")
#   ├── button "Inner Load"   (sets inner pending)
#   ├── dyn_node[1]  ← inner content slot
#   ├── dyn_node[2]  ← inner skeleton slot
#   │
#   SNInnerContentChild:     p > dyn_text("Inner: {data}")
#   SNInnerSkeletonChild:    p > dyn_text("Inner loading...")
#   SNOuterSkeletonChild:    p > dyn_text("Outer loading...")

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


struct SNInnerContentChild(Movable):
    """Inner content child: displays loaded inner data.

    Template: p > dyn_text("Inner: {data}")
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self, data: String) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Inner: ") + data)
        return vb.build()


struct SNInnerSkeletonChild(Movable):
    """Inner skeleton child: loading placeholder.

    Template: p > dyn_text("Inner loading...")
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Inner loading..."))
        return vb.build()


struct SNOuterContentChild(Movable):
    """Outer content child: inner suspense boundary.

    This child IS the inner suspense boundary.  It manages
    InnerContentChild and InnerSkeletonChild via its own dyn_node slots.

    Template: div > p(dyn_text("Outer: ...")) + button("Inner Load", onclick_custom) + dyn_node[1] + dyn_node[2]
    """

    var child_ctx: ChildComponentContext
    var inner_content: SNInnerContentChild
    var inner_skeleton: SNInnerSkeletonChild

    fn __init__(
        out self,
        var child_ctx: ChildComponentContext,
        var inner_content: SNInnerContentChild,
        var inner_skeleton: SNInnerSkeletonChild,
    ):
        self.child_ctx = child_ctx^
        self.inner_content = inner_content^
        self.inner_skeleton = inner_skeleton^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^
        self.inner_content = other.inner_content^
        self.inner_skeleton = other.inner_skeleton^

    fn render(mut self, data: String) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Outer: ") + data)
        vb.add_dyn_placeholder()  # dyn_node[1] — inner content slot
        vb.add_dyn_placeholder()  # dyn_node[2] — inner skeleton slot
        return vb.build()


struct SNOuterSkeletonChild(Movable):
    """Outer skeleton child: loading placeholder.

    Template: p > dyn_text("Outer loading...")
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Outer loading..."))
        return vb.build()


struct SuspenseNestApp(Movable):
    """Nested suspense boundary demo app.

    Outer boundary on root scope; inner boundary on outer_content child.
    Inner load → inner boundary shows inner skeleton, outer unaffected.
    Outer load → outer boundary shows outer skeleton (hides inner tree).
    Resolve at each level is independent.
    """

    var ctx: ComponentContext
    var outer_content: SNOuterContentChild
    var outer_skeleton: SNOuterSkeletonChild
    var outer_data: String
    var inner_data: String
    var outer_load_handler: UInt32
    var inner_load_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.ctx.use_suspense_boundary()
        self.outer_data = String("ready")
        self.inner_data = String("(none)")

        # Root view: h1 + Outer Load button + dyn_node[0] + dyn_node[1]
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Nested Suspense"))),
                el_button(
                    dsl_text(String("Outer Load")),
                    dsl_onclick_custom(),
                ),
                dsl_dyn_node(0),
                dsl_dyn_node(1),
            ),
            String("suspense-nest"),
        )
        # Outer load handler = first onclick_custom in root view
        self.outer_load_handler = self.ctx.view_event_handler_id(0)

        # ── Outer Content child (inner boundary) ────────────────────
        # div > p(dyn_text) + button("Inner Load") + dyn_node[1] + dyn_node[2]
        var outer_content_ctx = self.ctx.create_child_context(
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Inner Load")),
                    dsl_onclick_custom(),
                ),
                dsl_dyn_node(1),
                dsl_dyn_node(2),
            ),
            String("sn-outer-content"),
        )
        # Inner load handler on the outer_content child
        self.inner_load_handler = outer_content_ctx.event_handler_id(0)

        # Mark outer_content as the inner suspense boundary
        outer_content_ctx.use_suspense_boundary()

        # ── Inner Content child (under outer_content scope) ──────────
        var inner_content_ctx = self.ctx.create_child_context_under(
            outer_content_ctx.scope_id,
            el_p(dsl_dyn_text()),
            String("sn-inner-content"),
        )
        var inner_content = SNInnerContentChild(inner_content_ctx^)

        # ── Inner Skeleton child (under outer_content scope) ─────────
        var inner_skeleton_ctx = self.ctx.create_child_context_under(
            outer_content_ctx.scope_id,
            el_p(dsl_dyn_text()),
            String("sn-inner-skeleton"),
        )
        var inner_skeleton = SNInnerSkeletonChild(inner_skeleton_ctx^)

        self.outer_content = SNOuterContentChild(
            outer_content_ctx^, inner_content^, inner_skeleton^
        )

        # ── Outer Skeleton child ─────────────────────────────────────
        var outer_skeleton_ctx = self.ctx.create_child_context(
            el_p(dsl_dyn_text()),
            String("sn-outer-skeleton"),
        )
        self.outer_skeleton = SNOuterSkeletonChild(outer_skeleton_ctx^)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.outer_content = other.outer_content^
        self.outer_skeleton = other.outer_skeleton^
        self.outer_data = other.outer_data^
        self.inner_data = other.inner_data^
        self.outer_load_handler = other.outer_load_handler
        self.inner_load_handler = other.inner_load_handler

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with placeholders for both outer slots."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()  # dyn_node[0] — outer content slot
        pvb.add_dyn_placeholder()  # dyn_node[1] — outer skeleton slot
        return pvb.build()


# ── SuspenseNestApp lifecycle functions ──────────────────────────────────────


fn _sn_init() -> UnsafePointer[SuspenseNestApp, MutExternalOrigin]:
    var app_ptr = alloc[SuspenseNestApp](1)
    app_ptr.init_pointee_move(SuspenseNestApp())
    return app_ptr


fn _sn_destroy(
    app_ptr: UnsafePointer[SuspenseNestApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(
        app_ptr[0].outer_content.inner_content.child_ctx
    )
    app_ptr[0].ctx.destroy_child_context(
        app_ptr[0].outer_content.inner_skeleton.child_ctx
    )
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].outer_content.child_ctx)
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].outer_skeleton.child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _sn_rebuild(
    mut app: SuspenseNestApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the suspense-nest app."""
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

    # 5. Extract anchors for outer content + outer skeleton slots
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    var outer_content_anchor: UInt32 = 0
    var outer_skeleton_anchor: UInt32 = 0
    if vnode_ptr[0].dyn_node_id_count() > 0:
        outer_content_anchor = vnode_ptr[0].get_dyn_node_id(0)
    if vnode_ptr[0].dyn_node_id_count() > 1:
        outer_skeleton_anchor = vnode_ptr[0].get_dyn_node_id(1)
    app.outer_content.child_ctx.init_slot(outer_content_anchor)
    app.outer_skeleton.child_ctx.init_slot(outer_skeleton_anchor)

    # 6. Flush outer content child (initial render — no pending)
    var outer_content_idx = app.outer_content.render(app.outer_data)
    app.outer_content.child_ctx.flush(writer_ptr, outer_content_idx)

    # 7. Extract anchors for inner content + inner skeleton slots
    var oc_vnode_ptr = app.outer_content.child_ctx.store[0].get_ptr(
        outer_content_idx
    )
    # dyn_node_ids[0] = text node (dyn_text[0] = "Outer: ready")
    # dyn_node_ids[1] = placeholder (dyn_node[1] = inner content slot)
    # dyn_node_ids[2] = placeholder (dyn_node[2] = inner skeleton slot)
    var inner_content_anchor: UInt32 = 0
    var inner_skeleton_anchor: UInt32 = 0
    if oc_vnode_ptr[0].dyn_node_id_count() > 1:
        inner_content_anchor = oc_vnode_ptr[0].get_dyn_node_id(1)
    if oc_vnode_ptr[0].dyn_node_id_count() > 2:
        inner_skeleton_anchor = oc_vnode_ptr[0].get_dyn_node_id(2)
    app.outer_content.inner_content.child_ctx.init_slot(inner_content_anchor)
    app.outer_content.inner_skeleton.child_ctx.init_slot(inner_skeleton_anchor)

    # 8. Flush inner content child (initial render — no inner pending)
    var inner_content_idx = app.outer_content.inner_content.render(
        app.inner_data
    )
    app.outer_content.inner_content.child_ctx.flush(
        writer_ptr, inner_content_idx
    )
    # Inner skeleton starts hidden — do NOT flush it
    # Outer skeleton starts hidden — do NOT flush it

    # 9. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _sn_handle_event(
    mut app: SuspenseNestApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    if handler_id == app.outer_load_handler:
        app.ctx.set_pending(True)
        return True
    elif handler_id == app.inner_load_handler:
        app.outer_content.child_ctx.set_pending(True)
        return True
    else:
        return app.ctx.dispatch_event(handler_id, event_type)


fn _sn_outer_resolve(
    mut app: SuspenseNestApp,
    data: String,
):
    """Store resolved outer data and clear outer pending state."""
    app.outer_data = data
    app.ctx.set_pending(False)


fn _sn_inner_resolve(
    mut app: SuspenseNestApp,
    data: String,
):
    """Store resolved inner data and clear inner pending state."""
    app.inner_data = data
    app.outer_content.child_ctx.set_pending(False)


fn _sn_flush(
    mut app: SuspenseNestApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with nested suspense boundary logic.

    Each boundary level uses two separate dyn_node slots (one for content,
    one for skeleton).  On pending the content slot is emptied and the
    skeleton slot is filled.  On resolve the reverse happens.

    Three cases for the outer boundary:
      1. Outer pending → hide inner children, hide outer content,
         show outer skeleton.
      2. Outer NOT pending, outer content NOT mounted (recovering) →
         hide outer skeleton, restore outer content + inner tree.
      3. Outer content already mounted → leave outer_content alone,
         handle inner state changes only.
    """
    var parent_dirty = app.ctx.consume_dirty()
    var oc_dirty = app.outer_content.child_ctx.is_dirty()
    var os_dirty = app.outer_skeleton.child_ctx.is_dirty()
    var ic_dirty = app.outer_content.inner_content.child_ctx.is_dirty()
    var is_dirty = app.outer_content.inner_skeleton.child_ctx.is_dirty()

    if (
        not parent_dirty
        and not oc_dirty
        and not os_dirty
        and not ic_dirty
        and not is_dirty
    ):
        return 0

    # Diff parent shell (placeholders → placeholders = no mutations usually)
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    if app.ctx.is_pending():
        # ── Case 1: Outer pending ────────────────────────────────────
        # Hide inner children first (while outer_content still mounted)
        app.outer_content.inner_content.child_ctx.flush_empty(writer_ptr)
        app.outer_content.inner_skeleton.child_ctx.flush_empty(writer_ptr)
        # Hide outer content
        app.outer_content.child_ctx.flush_empty(writer_ptr)
        # Show outer skeleton
        var os_idx = app.outer_skeleton.render()
        app.outer_skeleton.child_ctx.flush(writer_ptr, os_idx)
    elif not app.outer_content.child_ctx.is_mounted():
        # ── Case 2: Recovering from outer pending ────────────────────
        # Hide outer skeleton
        app.outer_skeleton.child_ctx.flush_empty(writer_ptr)

        # Restore outer content
        var oc_idx = app.outer_content.render(app.outer_data)
        app.outer_content.child_ctx.flush(writer_ptr, oc_idx)

        # Re-extract inner anchors (outer_content was recreated)
        var oc_vnode_ptr = app.outer_content.child_ctx.store[0].get_ptr(oc_idx)
        # dyn_node_ids[0] = text node, [1] = inner content, [2] = inner skeleton
        var inner_content_anchor: UInt32 = 0
        var inner_skeleton_anchor: UInt32 = 0
        if oc_vnode_ptr[0].dyn_node_id_count() > 1:
            inner_content_anchor = oc_vnode_ptr[0].get_dyn_node_id(1)
        if oc_vnode_ptr[0].dyn_node_id_count() > 2:
            inner_skeleton_anchor = oc_vnode_ptr[0].get_dyn_node_id(2)
        app.outer_content.inner_content.child_ctx.init_slot(
            inner_content_anchor
        )
        app.outer_content.inner_skeleton.child_ctx.init_slot(
            inner_skeleton_anchor
        )

        # Render inner state
        if app.outer_content.child_ctx.is_pending():
            # Inner pending persisted while outer was pending
            var is_idx = app.outer_content.inner_skeleton.render()
            app.outer_content.inner_skeleton.child_ctx.flush(writer_ptr, is_idx)
            # inner content slot stays as placeholder — don't flush it
        else:
            # No inner pending — show inner content
            var ic_idx = app.outer_content.inner_content.render(app.inner_data)
            app.outer_content.inner_content.child_ctx.flush(writer_ptr, ic_idx)
            # inner skeleton slot stays as placeholder — don't flush it
    else:
        # ── Case 3: Outer content mounted — inner changes only ───────
        if app.outer_content.child_ctx.is_pending():
            # Inner pending: hide inner content, show inner skeleton
            app.outer_content.inner_content.child_ctx.flush_empty(writer_ptr)
            var is_idx = app.outer_content.inner_skeleton.render()
            app.outer_content.inner_skeleton.child_ctx.flush(writer_ptr, is_idx)
        else:
            # No inner pending: hide inner skeleton, show inner content
            app.outer_content.inner_skeleton.child_ctx.flush_empty(writer_ptr)
            var ic_idx = app.outer_content.inner_content.render(app.inner_data)
            app.outer_content.inner_content.child_ctx.flush(writer_ptr, ic_idx)

    return app.ctx.finalize(writer_ptr)
