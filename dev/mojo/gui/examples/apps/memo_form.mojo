# MemoFormApp — MemoBool + MemoString form validation demo (Phase 35.2).
#
# Demonstrates MemoBool and MemoString in a practical form-validation
# scenario.  A string input feeds two derived memos:
#   - is_valid (MemoBool): True when input is non-empty
#   - status (MemoString): "✓ Valid: {input}" or "✗ Empty"
#
# Structure:
#   MemoFormApp (single root scope, no child components)
#   ├── h1 "Form Validation"
#   ├── input (type="text", bind_value + oninput_set_string → input signal)
#   ├── p > dyn_text("Valid: true/false")         ← MemoBool output
#   └── p > dyn_text("Status: ✓ Valid: .../✗ Empty")  ← MemoString output
#
# Lifecycle:
#   1. Init: input="", is_valid=False, status="✗ Empty". Both memos start dirty.
#   2. Rebuild: run_memos → render → mount.
#   3. Type "hi": input="hi" → scope dirty + both memos dirty.
#      Flush → is_valid=True, status="✓ Valid: hi" → render → diff → SetText.
#   4. Clear input: input="" → is_valid=False, status="✗ Empty".

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from component import ComponentContext
from signals.handle import SignalString, MemoBool, MemoString
from html import (
    Node,
    el_div,
    el_h1,
    el_p,
    el_input as dsl_el_input,
    text as dsl_text,
    dyn_text as dsl_dyn_text,
    attr as dsl_attr,
    bind_value as dsl_bind_value,
    oninput_set_string as dsl_oninput_set_string,
)


struct MemoFormApp(Movable):
    """Form validation demo app — input with MemoBool + MemoString derived state.

    Demonstrates memo type expansion (Phase 35) in a practical
    form-validation scenario where a string input feeds two derived
    memos of different types.
    """

    var ctx: ComponentContext
    var input: SignalString
    var is_valid: MemoBool
    var status: MemoString
    var input_handler: UInt32

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.input = self.ctx.use_signal_string(String(""))
        self.is_valid = self.ctx.use_memo_bool(False)
        self.status = self.ctx.use_memo_string(String("✗ Empty"))
        self.ctx.setup_view(
            el_div(
                el_h1(dsl_text(String("Form Validation"))),
                dsl_el_input(
                    dsl_attr(String("type"), String("text")),
                    dsl_bind_value(self.input),
                    dsl_oninput_set_string(self.input),
                ),
                el_p(dsl_dyn_text()),
                el_p(dsl_dyn_text()),
            ),
            String("memo-form"),
        )
        # oninput_set_string is registered as an event handler.
        # In tree-walk order: bind_value is auto binding[0],
        # oninput_set_string is auto binding[1] (event).
        # view_event_handler_id(0) returns the first event handler.
        self.input_handler = self.ctx.view_event_handler_id(0)

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.input = other.input^
        self.is_valid = other.is_valid.copy()
        self.status = other.status.copy()
        self.input_handler = other.input_handler

    fn run_memos(mut self):
        """Recompute both memos if dirty.

        Order matters: is_valid must be recomputed before status,
        because status reads is_valid.

        Chain: input signal → is_valid memo (len > 0)
                             → status memo (reads input + is_valid)
        """
        # Step 1: Recompute is_valid (depends on input)
        if self.is_valid.is_dirty():
            self.is_valid.begin_compute()
            var txt = self.input.read()  # subscribes memo to input
            self.is_valid.end_compute(len(txt) > 0)

        # Step 2: Recompute status (depends on input + is_valid)
        if self.status.is_dirty():
            self.status.begin_compute()
            var txt = self.input.read()  # subscribes memo to input
            var valid = self.is_valid.read()  # subscribes memo to is_valid
            if valid:
                self.status.end_compute(String("✓ Valid: ") + txt)
            else:
                self.status.end_compute(String("✗ Empty"))

    fn render(mut self) -> UInt32:
        """Build a fresh VNode with 2 dyn_text slots."""
        var vb = self.ctx.render_builder()
        if self.is_valid.peek():
            vb.add_dyn_text(String("Valid: true"))
        else:
            vb.add_dyn_text(String("Valid: false"))
        vb.add_dyn_text(String("Status: ") + self.status.peek())
        return vb.build()


# ── MemoFormApp lifecycle functions ──────────────────────────────────────────


fn _mf_init() -> UnsafePointer[MemoFormApp, MutExternalOrigin]:
    var app_ptr = alloc[MemoFormApp](1)
    app_ptr.init_pointee_move(MemoFormApp())
    return app_ptr


fn _mf_destroy(
    app_ptr: UnsafePointer[MemoFormApp, MutExternalOrigin],
):
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn _mf_rebuild(
    mut app: MemoFormApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the memo-form app.

    Recomputes memos to settle derived state, then renders and mounts.
    """
    # Run initial memo recomputation
    app.run_memos()
    # Render with settled state
    var vnode_idx = app.render()
    var result = app.ctx.mount(writer_ptr, vnode_idx)
    # Consume dirty scopes left over from memo signal writes
    _ = app.ctx.consume_dirty()
    return result


fn _mf_handle_event(
    mut app: MemoFormApp,
    handler_id: UInt32,
    event_type: UInt8,
) -> Bool:
    return app.ctx.dispatch_event(handler_id, event_type)


fn _mf_handle_event_string(
    mut app: MemoFormApp,
    handler_id: UInt32,
    event_type: UInt8,
    value: String,
) -> Bool:
    return app.ctx.dispatch_event_with_string(handler_id, event_type, value)


fn _mf_flush(
    mut app: MemoFormApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates with memo recomputation.

    1. has_dirty() gate — bail early if nothing to do
    2. run_memos() — recompute is_valid and status
    3. settle_scopes() — remove scopes with no actual signal changes
    4. consume_dirty() — drain remaining dirty scopes via scheduler
    5. render() + diff + finalize — emit mutations
    """
    if not app.ctx.has_dirty():
        return 0
    # Recompute memos (while scopes are still in dirty_scopes)
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
