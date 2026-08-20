# BatchDemoApp — batch signal writes demo (Phase 38.2).
#
# Demonstrates batch writes for a multi-field form.  Two string signals
# (first_name, last_name) feed into a MemoString (full_name), and a
# SignalI32 (write_count) tracks how many batch operations have occurred.
#
# `set_names` and `reset` wrap their writes in begin_batch/end_batch so
# that all signal writes happen as a single propagation pass.
#
# Structure:
#   BatchDemoApp (single root scope, no child components)
#   ├── h1 "Batch Demo"
#   ├── button "Set Names" (onclick_custom)
#   ├── button "Reset" (onclick_custom)
#   ├── p > dyn_text("Full: ...")           ← MemoString
#   └── p > dyn_text("Writes: N")          ← SignalI32
#
# Lifecycle:
#   1. Init: first_name="", last_name="", full_name=" ", write_count=0.
#   2. Rebuild: recompute memo → render → mount.
#   3. set_names("Alice", "Smith"): batch writes first/last name + bumps
#      write_count.  One propagation pass.  full_name = "Alice Smith".
#   4. reset(): batch writes both names to "" + write_count to 0.
#      full_name = " ".

from std.memory import Pointer
from std.memory.alloc import unsafe_alloc
from bridge import MutationWriter
from component import ComponentContext
from signals.handle import SignalI32 as _SignalI32, SignalString, MemoString
from html import (
    Node,
    el_div,
    el_h1,
    el_p,
    el_button,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    onclick_custom as dsl_onclick_custom,
)


struct BatchDemoApp(Movable):
    """Batch signal writes demo app — multi-field form with batched updates.

    Demonstrates Phase 38 batch signal writes where multiple signal
    writes are grouped into a single propagation pass via
    begin_batch/end_batch.
    """

    var ctx: ComponentContext
    var first_name: SignalString
    var last_name: SignalString
    var full_name: MemoString
    var write_count: _SignalI32
    var set_handler: UInt32
    var reset_handler: UInt32

    def __init__(out self):
        self.ctx = ComponentContext.create()
        # Create string signals without auto-subscribing the scope —
        # the scope subscribes to memo outputs and write_count instead.
        self.first_name = self.ctx.create_signal_string(String(""))
        self.last_name = self.ctx.create_signal_string(String(""))
        self.full_name = self.ctx.use_memo_string(String(" "))
        self.write_count = self.ctx.use_signal(0)
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Batch Demo"))),
                el_button(
                    dsl_text(String("Set Names")),
                    dsl_onclick_custom(),
                ),
                el_button(
                    dsl_text(String("Reset")),
                    dsl_onclick_custom(),
                ),
                el_p(dsl_dyn_text()),
                el_p(dsl_dyn_text()),
            ),
            String("batch-demo"),
        )
        self.set_handler = self.ctx.view_event_handler_id(0)
        self.reset_handler = self.ctx.view_event_handler_id(1)

    def __init__(out self, *, deinit move: Self):
        self.ctx = move.ctx^
        self.first_name = move.first_name^
        self.last_name = move.last_name^
        self.full_name = move.full_name.copy()
        self.write_count = move.write_count.copy()
        self.set_handler = move.set_handler
        self.reset_handler = move.reset_handler

    def run_memos(mut self):
        """Recompute the full_name memo if dirty.

        Chain: first_name + last_name → full_name (concatenation)
        """
        if self.full_name.is_dirty():
            self.full_name.begin_compute()
            var f = self.first_name.read()
            var l = self.last_name.read()
            self.full_name.end_compute(f + String(" ") + l)

    def set_names(mut self, first: String, last: String):
        """Set both names and bump write_count in a single batch.

        All three signal writes happen inside begin_batch/end_batch,
        so only one propagation pass occurs at end_batch.
        """
        self.ctx.begin_batch()
        self.first_name.set(first)
        self.last_name.set(last)
        self.write_count += 1
        self.ctx.end_batch()

    def reset(mut self):
        """Reset all state in a single batch.

        Writes first_name="", last_name="", write_count=0.
        """
        self.ctx.begin_batch()
        self.first_name.set(String(""))
        self.last_name.set(String(""))
        self.write_count.set(0)
        self.ctx.end_batch()

    def render(mut self) -> UInt32:
        """Build a fresh VNode with 2 dyn_text slots."""
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(String("Full: ") + self.full_name.peek())
        vb.add_dyn_text(String("Writes: ") + String(self.write_count.peek()))
        return vb.build()


# ── BatchDemoApp lifecycle functions ─────────────────────────────────────────


def _bd_init() -> Pointer[BatchDemoApp, MutUntrackedOrigin]:
    var app_ptr = unsafe_alloc[BatchDemoApp](1)
    app_ptr.unsafe_write(BatchDemoApp())
    return app_ptr


def _bd_destroy(
    app_ptr: Pointer[BatchDemoApp, MutUntrackedOrigin],
):
    app_ptr[unsafe_offset=0].ctx.destroy()
    app_ptr.unsafe_deinit_pointee()
    app_ptr.unsafe_free()


def _bd_rebuild(
    mut app: BatchDemoApp,
    writer_ptr: Pointer[MutationWriter, MutUntrackedOrigin],
) -> Int32:
    """Initial render (mount) of the batch-demo app.

    Recomputes memo to settle derived state, then renders and mounts.
    """
    # Run initial memo recomputation
    app.run_memos()
    # Render with settled state
    var vnode_idx = app.render()
    var result = app.ctx.mount(writer_ptr, vnode_idx)
    # Consume dirty scopes left over from memo signal writes
    _ = app.ctx.consume_dirty()
    return result


def _bd_handle_event(
    mut app: BatchDemoApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


def _bd_flush(
    mut app: BatchDemoApp,
    writer_ptr: Pointer[MutationWriter, MutUntrackedOrigin],
) -> Int32:
    """Flush pending updates with batch-aware memo chain.

    1. has_dirty() gate — bail early if nothing to do
    2. run_memos() — recompute full_name
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
