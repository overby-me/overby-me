# EffectDemoApp — effect-in-flush pattern demo (Phase 34.1).
#
# Demonstrates reactive effects running in the flush cycle.  A count signal
# drives an effect that computes derived state (doubled, parity).
#
# Structure:
#   EffectDemoApp (single root scope, no child components)
#   ├── h1 "Effect Demo"
#   ├── button "+ 1"  (onclick_add count)
#   ├── p > dyn_text("Count: N")
#   ├── p > dyn_text("Doubled: N")
#   └── p > dyn_text("Parity: even/odd")
#
# Lifecycle:
#   1. Init: count=0, doubled=0, parity="even", effect starts pending
#   2. Rebuild: run_effects (sets doubled=0, parity="even") → render → mount
#   3. Increment: count += 1 → scope dirty + effect pending
#   4. Flush: consume_dirty → run_effects → render → diff → mutations
#
# The effect reads count (re-subscribing each run), then writes doubled
# and parity.  The drain-and-run pattern ensures all derived state is
# settled before render().

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext
from signals.handle import SignalI32 as _SignalI32, SignalString, EffectHandle
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


struct EffectDemoApp(Movable):
    """Effect demo app — count signal with derived doubled + parity via effect.

    Demonstrates the effect-in-flush pattern where effects run between
    consume_dirty() and render() to settle derived state.
    """

    var ctx: ComponentContext
    var count: _SignalI32
    var doubled: _SignalI32
    var parity: SignalString
    var count_effect: EffectHandle
    var incr_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.doubled = self.ctx.use_signal(0)
        self.parity = self.ctx.use_signal_string(String("even"))
        self.count_effect = self.ctx.use_effect()
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Effect Demo"))),
                el_button(
                    dsl_text(String("+ 1")),
                    dsl_onclick_add(self.count, 1),
                ),
                el_p(dsl_dyn_text()),
                el_p(dsl_dyn_text()),
                el_p(dsl_dyn_text()),
            ),
            String("effect-demo"),
        )
        self.incr_handler = self.ctx.view_event_handler_id(0)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count.copy()
        self.doubled = other.doubled.copy()
        self.parity = other.parity^
        self.count_effect = other.count_effect.copy()
        self.incr_handler = other.incr_handler

    fn run_effects(mut self):
        """Drain and execute pending effects.

        The count_effect reads count (re-subscribing), then writes
        doubled and parity signals.  This is the drain-and-run pattern:
        effects run after consume_dirty() and before render().
        """
        if self.count_effect.is_pending():
            self.count_effect.begin_run()
            var c = self.count.read()  # re-subscribe to count
            self.doubled.set(c * 2)
            if c % 2 == 0:
                self.parity.set(String("even"))
            else:
                self.parity.set(String("odd"))
            self.count_effect.end_run()

    fn render(mut self) -> UInt32:
        """Build a fresh VNode with 3 dyn_text slots."""
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(String("Count: ") + String(self.count.peek()))
        vb.add_dyn_text(String("Doubled: ") + String(self.doubled.peek()))
        vb.add_dyn_text(String("Parity: ") + String(self.parity.peek()))
        return vb.build()


# ── EffectDemoApp lifecycle functions ────────────────────────────────────────


fn _ed_init() -> UnsafePointer[EffectDemoApp, MutExternalOrigin]:
    var app_ptr = alloc[EffectDemoApp](1)
    app_ptr.init_pointee_move(EffectDemoApp())
    return app_ptr


fn _ed_destroy(
    app_ptr: UnsafePointer[EffectDemoApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _ed_rebuild(
    mut app: EffectDemoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the effect-demo app.

    Runs effects first to settle derived state (doubled, parity), then
    renders and mounts.
    """
    # Run initial effects — effect starts pending, so this sets doubled=0, parity="even"
    app.run_effects()
    # Render with settled state
    var vnode_idx = app.render()
    var result = app.ctx.mount(writer_ptr, vnode_idx)
    # Consume dirty scopes left over from effect signal writes —
    # the DOM is already correct (rendered with settled state).
    _ = app.ctx.consume_dirty()
    return result


fn _ed_handle_event(
    mut app: EffectDemoApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _ed_flush(
    mut app: EffectDemoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with effect drain-and-run pattern.

    1. consume_dirty() — collect dirty scopes
    2. run_effects() — effects may write signals (more dirty is OK)
    3. render() — build VNode with all state settled
    4. diff + finalize — emit mutations
    """
    if not app.ctx.consume_dirty():
        return 0
    # Run pending effects — they may write signals
    app.run_effects()
    # Render with settled state
    var new_idx = app.render()
    app.ctx.diff(writer_ptr, new_idx)
    return app.ctx.finalize(writer_ptr)
