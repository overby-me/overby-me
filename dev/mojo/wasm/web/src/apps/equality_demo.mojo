# EqualityDemoApp — equality-gated memo chain demo (Phase 37.3).
#
# Demonstrates the equality gate in action.  Uses a chain where an
# intermediate memo frequently stabilizes, showing that downstream
# memos and scopes skip unnecessary work.
#
# Structure:
#   EqualityDemoApp (single root scope, no child components)
#   ├── h1 "Equality Gate"
#   ├── button "+ 1" (onclick_add input)
#   ├── button "- 1" (onclick_sub input)
#   ├── p > dyn_text("Input: N")
#   ├── p > dyn_text("Clamped: N")           ← MemoI32 clamp(input, 0, 10)
#   └── p > dyn_text("Label: high/low")      ← MemoString (clamped > 5 ? "high" : "low")
#
# Interesting behaviour:
#   - Input 0→1:  clamped 0→1 (changed), label "low"→"low" (stable!)
#   - Input 5→6:  clamped 5→6 (changed), label "low"→"high" (changed)
#   - Input 10→11: clamped 10→10 (stable!), label "high"→"high" (stable!)
#   - Input 11→12: clamped 10→10 (stable!), label "high"→"high" (stable!)

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext
from signals.handle import SignalI32 as _SignalI32, MemoI32, MemoString
from html import (
    Node,
    el_div,
    el_h1,
    el_p,
    el_button,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    onclick_add as dsl_onclick_add,
    onclick_sub as dsl_onclick_sub,
)


struct EqualityDemoApp(Movable):
    """Equality-gated memo chain demo app.

    Demonstrates that when an intermediate memo recomputes to the same
    value, downstream memos and scopes skip unnecessary work thanks to
    the Phase 37 equality gate in `end_compute`.

    The input signal is created with `create_signal` (no auto-subscribe)
    so the scope only subscribes to the memo outputs (clamped, label).
    When the memo chain is value-stable (e.g. input above the clamp
    max), `settle_scopes()` removes the scope and flush emits zero
    mutations.
    """

    var ctx: ComponentContext
    var input: _SignalI32
    var clamped: MemoI32
    var label: MemoString
    var incr_handler: UInt32
    var decr_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.input = self.ctx.create_signal(0)
        self.clamped = self.ctx.use_memo(0)
        self.label = self.ctx.use_memo_string(String("low"))
        var children = List[Node]()
        children.append(el_h1(dsl_text(String("Equality Gate"))))
        children.append(
            el_button(
                dsl_text(String("+ 1")),
                dsl_onclick_add(self.input, 1),
            )
        )
        children.append(
            el_button(
                dsl_text(String("- 1")),
                dsl_onclick_sub(self.input, 1),
            )
        )
        children.append(el_p(dsl_dyn_text()))
        children.append(el_p(dsl_dyn_text()))
        self.ctx.setup_view(
            el_div(children^),
            String("equality-demo"),
        )
        self.incr_handler = self.ctx.view_event_handler_id(0)
        self.decr_handler = self.ctx.view_event_handler_id(1)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.input = other.input.copy()
        self.clamped = other.clamped.copy()
        self.label = other.label.copy()
        self.incr_handler = other.incr_handler
        self.decr_handler = other.decr_handler

    fn run_memos(mut self):
        """Recompute all memos in dependency order.

        Chain: input → clamped (clamp(input, 0, 10))
                      → label ("high" if clamped > 5 else "low")

        Thanks to Phase 37 equality gating, if clamped recomputes to
        the same value (e.g. input goes from 10 to 11, clamped stays 10),
        the label memo will also be value-stable and the scope can be
        settled (no re-render needed).
        """
        if self.clamped.is_dirty():
            self.clamped.begin_compute()
            var i = self.input.read()
            var c = i
            if c < 0:
                c = 0
            if c > 10:
                c = 10
            self.clamped.end_compute(c)

        if self.label.is_dirty():
            self.label.begin_compute()
            var c = self.clamped.read()
            if c > 5:
                self.label.end_compute(String("high"))
            else:
                self.label.end_compute(String("low"))

    fn render(mut self) -> UInt32:
        """Build a fresh VNode with 2 dyn_text slots (clamped + label).

        The input signal is NOT displayed here because the scope does
        not subscribe to it (created with `create_signal`).  Displaying
        it via peek() would show stale values when the scope is settled.
        Use `eq_input_value` to read the raw input in tests.
        """
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(String("Clamped: ") + String(self.clamped.peek()))
        vb.add_dyn_text(String("Label: ") + self.label.peek())
        return vb.build()


# ── EqualityDemoApp lifecycle functions ───────────────────────────────────────


fn _eq_init() -> UnsafePointer[EqualityDemoApp, MutExternalOrigin]:
    var app_ptr = alloc[EqualityDemoApp](1)
    app_ptr.init_pointee_move(EqualityDemoApp())
    return app_ptr


fn _eq_destroy(
    app_ptr: UnsafePointer[EqualityDemoApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _eq_rebuild(
    mut app: EqualityDemoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the equality-demo app."""
    # Run initial memo recomputation
    app.run_memos()
    # Render with settled state
    var vnode_idx = app.render()
    var result = app.ctx.mount(writer_ptr, vnode_idx)
    # Consume dirty scopes left over from memo signal writes
    _ = app.ctx.consume_dirty()
    return result


fn _eq_handle_event(
    mut app: EqualityDemoApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _eq_flush(
    mut app: EqualityDemoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with equality-gated memo chain.

    1. has_dirty() gate — bail early if nothing to do
    2. run_memos() — recompute clamped → label (sets _changed_signals)
    3. settle_scopes() — remove scopes whose subscribed signals are
       all value-stable (operates on dirty_scopes BEFORE consume)
    4. consume_dirty() — drain remaining dirty scopes via scheduler
    5. render() + diff + finalize — emit mutations

    The key insight: settle_scopes() must run BEFORE consume_dirty()
    because consume_dirty() drains dirty_scopes.  If settle removes
    all scopes, has_dirty() returns False and we skip render entirely.
    """
    if not app.ctx.has_dirty():
        return 0
    # Recompute memo chain (while scopes are still in dirty_scopes)
    app.run_memos()
    # Phase 37: filter dirty_scopes — remove scopes with no actual changes
    app.ctx.settle_scopes()
    # If all scopes were settled, skip render entirely
    if not app.ctx.has_dirty():
        return 0
    # Consume remaining dirty scopes via scheduler
    _ = app.ctx.consume_dirty()
    # Render with settled state
    var new_idx = app.render()
    app.ctx.diff(writer_ptr, new_idx)
    return app.ctx.finalize(writer_ptr)
