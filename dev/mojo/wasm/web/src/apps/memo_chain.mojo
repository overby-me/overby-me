# MemoChainApp — mixed-type memo chain demo (Phase 35.3).
#
# Demonstrates a multi-level mixed-type memo chain:
#   SignalI32 → MemoI32 → MemoBool → MemoString
#
# Validates that dirtiness propagates correctly across memo types and
# that recomputation order is deterministic.
#
# Structure:
#   MemoChainApp (single root scope, no child components)
#   ├── h1 "Memo Chain"
#   ├── button "+ 1" (onclick_add input)
#   ├── p > dyn_text("Input: N")
#   ├── p > dyn_text("Doubled: N")          ← MemoI32 (input * 2)
#   ├── p > dyn_text("Is Big: true/false")  ← MemoBool (doubled >= 10)
#   └── p > dyn_text("Label: small/BIG")    ← MemoString (is_big ? "BIG" : "small")
#
# Lifecycle:
#   1. Init: input=0, doubled=0, is_big=False, label="small". All memos dirty.
#   2. Rebuild: recompute chain → render → mount.
#   3. Increment to 5: input=5 → doubled=10 → is_big=True → label="BIG".
#   4. Increment to 6: input=6 → doubled=12 → is_big=True → label="BIG".

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext
from signals.handle import (
    SignalI32 as _SignalI32,
    MemoI32,
    MemoBool,
    MemoString,
)
from html import (
    Node,
    el_div,
    el_h1,
    el_p,
    el_button,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    onclick_add as dsl_onclick_add,
)


struct MemoChainApp(Movable):
    """Mixed-type memo chain demo app.

    Demonstrates the full SignalI32 → MemoI32 → MemoBool → MemoString
    reactive chain where each memo derives from the previous one's
    output, exercising cross-type memo propagation.
    """

    var ctx: ComponentContext
    var input: _SignalI32
    var doubled: MemoI32
    var is_big: MemoBool
    var label: MemoString
    var incr_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.input = self.ctx.use_signal(0)
        self.doubled = self.ctx.use_memo(0)
        self.is_big = self.ctx.use_memo_bool(False)
        self.label = self.ctx.use_memo_string(String("small"))
        var children = List[Node]()
        children.append(el_h1(dsl_text(String("Memo Chain"))))
        children.append(
            el_button(
                dsl_text(String("+ 1")),
                dsl_onclick_add(self.input, 1),
            )
        )
        children.append(el_p(dsl_dyn_text()))
        children.append(el_p(dsl_dyn_text()))
        children.append(el_p(dsl_dyn_text()))
        children.append(el_p(dsl_dyn_text()))
        self.ctx.setup_view(
            el_div(children^),
            String("memo-chain"),
        )
        self.incr_handler = self.ctx.view_event_handler_id(0)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.input = other.input.copy()
        self.doubled = other.doubled.copy()
        self.is_big = other.is_big.copy()
        self.label = other.label.copy()
        self.incr_handler = other.incr_handler

    fn run_memos(mut self):
        """Recompute all memos in dependency order.

        Chain: input → doubled (input * 2)
                      → is_big (doubled >= 10)
                        → label ("BIG" if is_big else "small")

        Order matters: each memo must see the fresh value from the
        previous memo in the chain.  The runtime automatically
        propagates dirtiness through memo → memo chains (Phase 36),
        so each memo checks is_dirty() independently.
        """
        # Each memo checks is_dirty() independently — the runtime's
        # worklist-based propagation (Phase 36) marks all downstream
        # memos dirty when the input signal is written.
        if self.doubled.is_dirty():
            self.doubled.begin_compute()
            var i = self.input.read()  # subscribes memo to input
            self.doubled.end_compute(i * 2)

        if self.is_big.is_dirty():
            self.is_big.begin_compute()
            var d = self.doubled.read()  # subscribes memo to doubled
            self.is_big.end_compute(d >= 10)

        if self.label.is_dirty():
            self.label.begin_compute()
            var big = self.is_big.read()  # subscribes memo to is_big
            if big:
                self.label.end_compute(String("BIG"))
            else:
                self.label.end_compute(String("small"))

    fn render(mut self) -> UInt32:
        """Build a fresh VNode with 4 dyn_text slots."""
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(String("Input: ") + String(self.input.peek()))
        vb.add_dyn_text(String("Doubled: ") + String(self.doubled.peek()))
        if self.is_big.peek():
            vb.add_dyn_text(String("Is Big: true"))
        else:
            vb.add_dyn_text(String("Is Big: false"))
        vb.add_dyn_text(String("Label: ") + self.label.peek())
        return vb.build()


# ── MemoChainApp lifecycle functions ─────────────────────────────────────────


fn _mc_init() -> UnsafePointer[MemoChainApp, MutExternalOrigin]:
    var app_ptr = alloc[MemoChainApp](1)
    app_ptr.init_pointee_move(MemoChainApp())
    return app_ptr


fn _mc_destroy(
    app_ptr: UnsafePointer[MemoChainApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _mc_rebuild(
    mut app: MemoChainApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the memo-chain app.

    Recomputes the full memo chain to settle derived state, then
    renders and mounts.
    """
    # Run initial memo recomputation
    app.run_memos()
    # Render with settled state
    var vnode_idx = app.render()
    var result = app.ctx.mount(writer_ptr, vnode_idx)
    # Consume dirty scopes left over from memo signal writes
    _ = app.ctx.consume_dirty()
    return result


fn _mc_handle_event(
    mut app: MemoChainApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _mc_flush(
    mut app: MemoChainApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with memo chain recomputation.

    1. has_dirty() gate — bail early if nothing to do
    2. run_memos() — recompute doubled → is_big → label
    3. settle_scopes() — remove scopes with no actual signal changes
    4. consume_dirty() — drain remaining dirty scopes via scheduler
    5. render() + diff + finalize — emit mutations
    """
    if not app.ctx.has_dirty():
        return 0
    # Recompute memo chain (while scopes are still in dirty_scopes)
    app.run_memos()
    # Phase 37: filter dirty_scopes before consuming
    app.ctx.settle_scopes()
    if not app.ctx.has_dirty():
        return 0
    _ = app.ctx.consume_dirty()
    # Render with settled state
    var new_idx = app.render()
    app.ctx.diff(writer_ptr, new_idx)
    return app.ctx.finalize(writer_ptr)
