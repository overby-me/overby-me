# ErrorNestApp — nested error boundaries demo (Phase 32.3).
#
# Demonstrates nested error boundaries where inner boundaries catch inner
# errors and outer boundaries catch outer errors.
#
# Uses the same two-slot pattern as SafeCounterApp: each boundary level
# has separate dyn_node slots for normal and fallback content.  On error,
# the normal slot is emptied and the fallback slot is filled (and vice
# versa on recovery).
#
# Structure:
#   ErrorNestApp (outer boundary)
#   ├── h1 "Nested Boundaries"
#   ├── button "Outer Crash"   (crashes to outer boundary)
#   ├── dyn_node[0]  ← outer normal slot
#   ├── dyn_node[1]  ← outer fallback slot
#   │
#   OuterNormal (inner boundary child)
#   ├── p > dyn_text("Status: OK")
#   ├── button "Inner Crash"   (crashes to inner boundary)
#   ├── dyn_node[0]  ← inner normal slot
#   ├── dyn_node[1]  ← inner fallback slot
#   │
#   InnerNormalChild:     p > dyn_text("Inner: working")
#   InnerFallbackChild:   p > dyn_text("Inner error: ...") + button("Inner Retry")
#   OuterFallbackChild:   p > dyn_text("Outer error: ...") + button("Outer Retry")

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


struct ENInnerNormalChild(Movable):
    """Inner normal content: displays "Inner: working".

    Template: p > dyn_text("Inner: working")
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Inner: working"))
        return vb.build()


struct ENInnerFallbackChild(Movable):
    """Inner fallback: shows inner error message + Inner Retry button.

    Template: div > p(dyn_text("Inner error: ...")) + button("Inner Retry", onclick_custom)
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self, error_msg: String) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Inner error: ") + error_msg)
        return vb.build()


struct ENOuterNormalChild(Movable):
    """Outer normal content: inner error boundary.

    This child IS the inner error boundary.  It manages
    InnerNormalChild and InnerFallbackChild via its own dyn_node slot.

    Template: div > p(dyn_text("Status: OK")) + button("Inner Crash", onclick_custom) + dyn_node[0]
    """

    var child_ctx: ChildComponentContext
    var inner_normal: ENInnerNormalChild
    var inner_fallback: ENInnerFallbackChild

    fn __init__(
        out self,
        var child_ctx: ChildComponentContext,
        var inner_normal: ENInnerNormalChild,
        var inner_fallback: ENInnerFallbackChild,
    ):
        self.child_ctx = child_ctx^
        self.inner_normal = inner_normal^
        self.inner_fallback = inner_fallback^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^
        self.inner_normal = other.inner_normal^
        self.inner_fallback = other.inner_fallback^

    fn render(mut self) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Status: OK"))
        vb.add_dyn_placeholder()  # dyn_node[1] — inner normal slot
        vb.add_dyn_placeholder()  # dyn_node[2] — inner fallback slot
        return vb.build()


struct ENOuterFallbackChild(Movable):
    """Outer fallback: shows outer error message + Outer Retry button.

    Template: div > p(dyn_text("Outer error: ...")) + button("Outer Retry", onclick_custom)
    """

    var child_ctx: ChildComponentContext

    fn __init__(out self, var child_ctx: ChildComponentContext):
        self.child_ctx = child_ctx^

    fn __moveinit__(out self, deinit other: Self):
        self.child_ctx = other.child_ctx^

    fn render(mut self, error_msg: String) -> UInt32:
        var vb = self.child_ctx.render_builder()
        vb.add_dyn_text(String("Outer error: ") + error_msg)
        return vb.build()


struct ErrorNestApp(Movable):
    """Nested error boundary demo app.

    Outer boundary on root scope; inner boundary on outer_normal child.
    Inner crash → inner boundary catches → inner slot swaps.
    Outer crash → outer boundary catches → outer slot swaps (hides inner).
    Recovery at each level is independent.
    """

    var ctx: ComponentContext
    var outer_normal: ENOuterNormalChild
    var outer_fallback: ENOuterFallbackChild
    var outer_crash_handler: UInt32
    var inner_crash_handler: UInt32
    var outer_retry_handler: UInt32
    var inner_retry_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.ctx.use_error_boundary()

        # Root view: h1 + Outer Crash button + dyn_node[0] + dyn_node[1]
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Nested Boundaries"))),
                el_button(
                    dsl_text(String("Outer Crash")),
                    dsl_onclick_custom(),
                ),
                dsl_dyn_node(0),
                dsl_dyn_node(1),
            ),
            String("error-nest"),
        )
        # Outer crash handler = first onclick_custom in root view
        self.outer_crash_handler = self.ctx.view_event_handler_id(0)

        # ── Outer Normal child (inner boundary) ─────────────────────
        # div > p(dyn_text) + button("Inner Crash") + dyn_node[0] + dyn_node[1]
        var outer_normal_ctx = self.ctx.create_child_context(
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Inner Crash")),
                    dsl_onclick_custom(),
                ),
                dsl_dyn_node(1),
                dsl_dyn_node(2),
            ),
            String("en-outer-normal"),
        )
        # Inner crash handler on the outer_normal child
        self.inner_crash_handler = outer_normal_ctx.event_handler_id(0)

        # Mark outer_normal as the inner error boundary
        outer_normal_ctx.use_error_boundary()

        # ── Inner Normal child (under outer_normal scope) ────────────
        var inner_normal_ctx = self.ctx.create_child_context_under(
            outer_normal_ctx.scope_id,
            el_p(dsl_dyn_text()),
            String("en-inner-normal"),
        )
        var inner_normal = ENInnerNormalChild(inner_normal_ctx^)

        # ── Inner Fallback child (under outer_normal scope) ──────────
        var inner_fallback_ctx = self.ctx.create_child_context_under(
            outer_normal_ctx.scope_id,
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Inner Retry")),
                    dsl_onclick_custom(),
                ),
            ),
            String("en-inner-fallback"),
        )
        self.inner_retry_handler = inner_fallback_ctx.event_handler_id(0)
        var inner_fallback = ENInnerFallbackChild(inner_fallback_ctx^)

        self.outer_normal = ENOuterNormalChild(
            outer_normal_ctx^, inner_normal^, inner_fallback^
        )

        # ── Outer Fallback child ─────────────────────────────────────
        var outer_fallback_ctx = self.ctx.create_child_context(
            el_div(
                el_p(dsl_dyn_text()),
                el_button(
                    dsl_text(String("Outer Retry")),
                    dsl_onclick_custom(),
                ),
            ),
            String("en-outer-fallback"),
        )
        self.outer_retry_handler = outer_fallback_ctx.event_handler_id(0)
        self.outer_fallback = ENOuterFallbackChild(outer_fallback_ctx^)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.outer_normal = other.outer_normal^
        self.outer_fallback = other.outer_fallback^
        self.outer_crash_handler = other.outer_crash_handler
        self.inner_crash_handler = other.inner_crash_handler
        self.outer_retry_handler = other.outer_retry_handler
        self.inner_retry_handler = other.inner_retry_handler

    fn render_parent(mut self) -> UInt32:
        """Build the parent VNode with placeholders for both outer slots."""
        var pvb = self.ctx.render_builder()
        pvb.add_dyn_placeholder()  # dyn_node[0] — outer normal slot
        pvb.add_dyn_placeholder()  # dyn_node[1] — outer fallback slot
        return pvb.build()


# ── ErrorNestApp lifecycle functions ─────────────────────────────────────────


fn _en_init() -> UnsafePointer[ErrorNestApp, MutExternalOrigin]:
    var app_ptr = alloc[ErrorNestApp](1)
    app_ptr.init_pointee_move(ErrorNestApp())
    return app_ptr


fn _en_destroy(
    app_ptr: UnsafePointer[ErrorNestApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy_child_context(
        app_ptr[0].outer_normal.inner_normal.child_ctx
    )
    app_ptr[0].ctx.destroy_child_context(
        app_ptr[0].outer_normal.inner_fallback.child_ctx
    )
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].outer_normal.child_ctx)
    app_ptr[0].ctx.destroy_child_context(app_ptr[0].outer_fallback.child_ctx)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _en_rebuild(
    mut app: ErrorNestApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the error-nest app."""
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

    # 5. Extract anchors for outer normal + outer fallback slots
    var vnode_ptr = app.ctx.store_ptr()[0].get_ptr(parent_idx)
    var outer_normal_anchor: UInt32 = 0
    var outer_fallback_anchor: UInt32 = 0
    if vnode_ptr[0].dyn_node_id_count() > 0:
        outer_normal_anchor = vnode_ptr[0].get_dyn_node_id(0)
    if vnode_ptr[0].dyn_node_id_count() > 1:
        outer_fallback_anchor = vnode_ptr[0].get_dyn_node_id(1)
    app.outer_normal.child_ctx.init_slot(outer_normal_anchor)
    app.outer_fallback.child_ctx.init_slot(outer_fallback_anchor)

    # 6. Flush outer normal child (initial render — no error)
    var outer_normal_idx = app.outer_normal.render()
    app.outer_normal.child_ctx.flush(writer_ptr, outer_normal_idx)

    # 7. Extract anchors for inner normal + inner fallback slots
    var on_vnode_ptr = app.outer_normal.child_ctx.store[0].get_ptr(
        outer_normal_idx
    )
    # dyn_node_ids[0] = text node (dyn_text[0] = "Status: OK")
    # dyn_node_ids[1] = placeholder (dyn_node[1] = inner normal slot)
    # dyn_node_ids[2] = placeholder (dyn_node[2] = inner fallback slot)
    var inner_normal_anchor: UInt32 = 0
    var inner_fallback_anchor: UInt32 = 0
    if on_vnode_ptr[0].dyn_node_id_count() > 1:
        inner_normal_anchor = on_vnode_ptr[0].get_dyn_node_id(1)
    if on_vnode_ptr[0].dyn_node_id_count() > 2:
        inner_fallback_anchor = on_vnode_ptr[0].get_dyn_node_id(2)
    app.outer_normal.inner_normal.child_ctx.init_slot(inner_normal_anchor)
    app.outer_normal.inner_fallback.child_ctx.init_slot(inner_fallback_anchor)

    # 8. Flush inner normal child
    var inner_normal_idx = app.outer_normal.inner_normal.render()
    app.outer_normal.inner_normal.child_ctx.flush(writer_ptr, inner_normal_idx)
    # Inner fallback starts hidden — do NOT flush it
    # Outer fallback starts hidden — do NOT flush it

    # 9. Finalize
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn _en_handle_event(
    mut app: ErrorNestApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    if handler_id == app.outer_crash_handler:
        _ = app.ctx.report_error(String("Outer crash"))
        return True
    elif handler_id == app.inner_crash_handler:
        # Inner crash: report from inner_crash button's scope (outer_normal)
        # to the inner boundary (outer_normal itself).
        # Since the inner crash button is registered under outer_normal's
        # scope and outer_normal IS the boundary, report_error on
        # outer_normal sets the error on itself.
        _ = app.outer_normal.child_ctx.report_error(String("Inner crash"))
        return True
    elif handler_id == app.outer_retry_handler:
        app.ctx.clear_error()
        return True
    elif handler_id == app.inner_retry_handler:
        app.outer_normal.child_ctx.clear_error()
        return True
    else:
        return app.ctx.dispatch_event(handler_id, event_type)


fn _en_flush(
    mut app: ErrorNestApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with nested error boundary logic.

    Each boundary level uses two separate dyn_node slots (one for normal
    content, one for fallback).  On error the normal slot is emptied and
    the fallback slot is filled.  On recovery the reverse happens.

    Three cases for the outer boundary:
      1. Outer error active → hide inner children, hide outer normal,
         show outer fallback.
      2. Outer normal NOT mounted (recovering from outer error) →
         hide outer fallback, restore outer normal + inner tree.
      3. Outer normal already mounted → leave outer_normal alone,
         handle inner state changes only.
    """
    var parent_dirty = app.ctx.consume_dirty()
    var on_dirty = app.outer_normal.child_ctx.is_dirty()
    var of_dirty = app.outer_fallback.child_ctx.is_dirty()
    var in_dirty = app.outer_normal.inner_normal.child_ctx.is_dirty()
    var if_dirty = app.outer_normal.inner_fallback.child_ctx.is_dirty()

    if (
        not parent_dirty
        and not on_dirty
        and not of_dirty
        and not in_dirty
        and not if_dirty
    ):
        return 0

    # Diff parent shell (placeholders → placeholders = no mutations usually)
    var new_parent_idx = app.render_parent()
    app.ctx.diff(writer_ptr, new_parent_idx)

    if app.ctx.has_error():
        # ── Case 1: Outer error ──────────────────────────────────────
        # Hide inner children first (while outer_normal still mounted)
        app.outer_normal.inner_normal.child_ctx.flush_empty(writer_ptr)
        app.outer_normal.inner_fallback.child_ctx.flush_empty(writer_ptr)
        # Hide outer normal
        app.outer_normal.child_ctx.flush_empty(writer_ptr)
        # Show outer fallback
        var of_idx = app.outer_fallback.render(app.ctx.error_message())
        app.outer_fallback.child_ctx.flush(writer_ptr, of_idx)
    elif not app.outer_normal.child_ctx.is_mounted():
        # ── Case 2: Recovering from outer error ──────────────────────
        # Hide outer fallback
        app.outer_fallback.child_ctx.flush_empty(writer_ptr)

        # Restore outer normal
        var on_idx = app.outer_normal.render()
        app.outer_normal.child_ctx.flush(writer_ptr, on_idx)

        # Re-extract inner anchors (outer_normal was recreated)
        var on_vnode_ptr = app.outer_normal.child_ctx.store[0].get_ptr(on_idx)
        # dyn_node_ids[0] = text node, [1] = inner normal, [2] = inner fallback
        var inner_normal_anchor: UInt32 = 0
        var inner_fallback_anchor: UInt32 = 0
        if on_vnode_ptr[0].dyn_node_id_count() > 1:
            inner_normal_anchor = on_vnode_ptr[0].get_dyn_node_id(1)
        if on_vnode_ptr[0].dyn_node_id_count() > 2:
            inner_fallback_anchor = on_vnode_ptr[0].get_dyn_node_id(2)
        app.outer_normal.inner_normal.child_ctx.init_slot(inner_normal_anchor)
        app.outer_normal.inner_fallback.child_ctx.init_slot(
            inner_fallback_anchor
        )

        # Render inner state
        if app.outer_normal.child_ctx.has_error():
            # Inner error persisted while outer was in error
            var if_idx = app.outer_normal.inner_fallback.render(
                app.outer_normal.child_ctx.error_message()
            )
            app.outer_normal.inner_fallback.child_ctx.flush(writer_ptr, if_idx)
            # inner normal slot stays as placeholder — don't flush it
        else:
            # No inner error — show inner normal
            var in_idx = app.outer_normal.inner_normal.render()
            app.outer_normal.inner_normal.child_ctx.flush(writer_ptr, in_idx)
            # inner fallback slot stays as placeholder — don't flush it
    else:
        # ── Case 3: Outer normal already mounted — inner changes only ─
        if app.outer_normal.child_ctx.has_error():
            # Inner error: hide inner normal, show inner fallback
            app.outer_normal.inner_normal.child_ctx.flush_empty(writer_ptr)
            var if_idx = app.outer_normal.inner_fallback.render(
                app.outer_normal.child_ctx.error_message()
            )
            app.outer_normal.inner_fallback.child_ctx.flush(writer_ptr, if_idx)
        else:
            # No inner error: hide inner fallback, show inner normal
            app.outer_normal.inner_fallback.child_ctx.flush_empty(writer_ptr)
            var in_idx = app.outer_normal.inner_normal.render()
            app.outer_normal.inner_normal.child_ctx.flush(writer_ptr, in_idx)

    return app.ctx.finalize(writer_ptr)
