# EffectMemoApp — signal → memo → effect → signal chain demo (Phase 34.2).
#
# Demonstrates the full reactive chain: signal → memo → effect → signal.
# An input signal feeds a memo (tripled = input * 3), and an effect reads
# the memo output to produce a label ("small" if tripled < 10, "big"
# otherwise).
#
# Structure:
#   EffectMemoApp (single root scope, no child components)
#   ├── h1 "Effect + Memo"
#   ├── button "+ 1"  (onclick_add input)
#   ├── p > dyn_text("Input: N")
#   ├── p > dyn_text("Tripled: N")     ← memo output (input * 3)
#   └── p > dyn_text("Label: ...")     ← effect reads tripled, writes label
#
# Chain:
#   input signal → tripled memo (input * 3)
#                    → label effect reads tripled.read()
#                      → writes label signal ("small" / "big")
#                        → render
#
# Lifecycle:
#   1. Init: input=0, tripled=0, label="small", effect starts pending
#   2. Rebuild: recompute memo + run effect → mount
#   3. Increment: input += 1 → scope dirty + memo dirty
#   4. Flush: consume_dirty → recompute memo → run effect → render → diff

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext
from signals.handle import (
    SignalI32 as _SignalI32,
    SignalString,
    MemoI32,
    EffectHandle,
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


struct EffectMemoApp(Movable):
    """Effect + memo chain demo app.

    Demonstrates the signal → memo → effect → signal reactive chain
    where a memo derives a value and an effect reads it to produce
    further derived state.
    """

    var ctx: ComponentContext
    var input: _SignalI32
    var tripled: MemoI32
    var label: SignalString
    var label_effect: EffectHandle
    var incr_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.input = self.ctx.use_signal(0)
        self.tripled = self.ctx.use_memo(0)
        self.label = self.ctx.use_signal_string(String("small"))
        self.label_effect = self.ctx.use_effect()
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Effect + Memo"))),
                el_button(
                    dsl_text(String("+ 1")),
                    dsl_onclick_add(self.input, 1),
                ),
                el_p(dsl_dyn_text()),
                el_p(dsl_dyn_text()),
                el_p(dsl_dyn_text()),
            ),
            String("effect-memo"),
        )
        self.incr_handler = self.ctx.view_event_handler_id(0)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.input = other.input.copy()
        self.tripled = other.tripled.copy()
        self.label = other.label^
        self.label_effect = other.label_effect.copy()
        self.incr_handler = other.incr_handler

    fn run_memos_and_effects(mut self):
        """Recompute memos, then drain and execute pending effects.

        Order matters: memos must be recomputed first so that effects
        reading their output see the fresh value.  The memo recomputation
        may mark the effect pending (if the output value changed).

        Chain: input → tripled memo → label effect → label signal.
        """
        # Step 1: Recompute tripled memo if dirty
        if self.tripled.is_dirty():
            self.tripled.begin_compute()
            var i = self.input.read()  # re-subscribe memo to input
            self.tripled.end_compute(i * 3)

        # Step 2: Run label effect if pending
        if self.label_effect.is_pending():
            self.label_effect.begin_run()
            var t = self.tripled.read()  # re-subscribe effect to tripled output
            if t < 10:
                self.label.set(String("small"))
            else:
                self.label.set(String("big"))
            self.label_effect.end_run()

    fn render(mut self) -> UInt32:
        """Build a fresh VNode with 3 dyn_text slots."""
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(String("Input: ") + String(self.input.peek()))
        vb.add_dyn_text(String("Tripled: ") + String(self.tripled.peek()))
        vb.add_dyn_text(String("Label: ") + String(self.label.peek()))
        return vb.build()


# ── EffectMemoApp lifecycle functions ────────────────────────────────────────


fn _em_init() -> UnsafePointer[EffectMemoApp, MutExternalOrigin]:
    var app_ptr = alloc[EffectMemoApp](1)
    app_ptr.init_pointee_move(EffectMemoApp())
    return app_ptr


fn _em_destroy(
    app_ptr: UnsafePointer[EffectMemoApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _em_rebuild(
    mut app: EffectMemoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the effect-memo app.

    Recomputes memo and runs effects to settle derived state, then
    renders and mounts.
    """
    # Run initial memo recomputation + effects
    app.run_memos_and_effects()
    # Render with settled state
    var vnode_idx = app.render()
    var result = app.ctx.mount(writer_ptr, vnode_idx)
    # Consume dirty scopes left over from memo/effect signal writes
    _ = app.ctx.consume_dirty()
    return result


fn _em_handle_event(
    mut app: EffectMemoApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _em_flush(
    mut app: EffectMemoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with memo + effect drain-and-run pattern.

    1. has_dirty() gate — bail early if nothing to do
    2. run_memos_and_effects() — recompute memos, then run effects
    3. settle_scopes() — remove scopes with no actual signal changes
    4. consume_dirty() — drain remaining dirty scopes via scheduler
    5. render() + diff + finalize — emit mutations
    """
    if not app.ctx.has_dirty():
        return 0
    # Recompute memos + run pending effects (while scopes still in dirty_scopes)
    app.run_memos_and_effects()
    # Phase 37: filter dirty_scopes before consuming
    app.ctx.settle_scopes()
    if not app.ctx.has_dirty():
        return 0
    _ = app.ctx.consume_dirty()
    # Render with settled state
    var new_idx = app.render()
    app.ctx.diff(writer_ptr, new_idx)
    return app.ctx.finalize(writer_ptr)
