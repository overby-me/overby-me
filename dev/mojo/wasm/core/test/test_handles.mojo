# Tests for reactive handles (SignalI32, MemoI32, EffectHandle, SignalString)
# and ComponentContext.
#
# Validates:
#   - SignalI32: peek, read, set, +=, -=, *=, //=, %=, toggle, version, __str__
#   - MemoI32: read, peek, is_dirty, begin_compute, end_compute, recompute_from
#   - EffectHandle: is_pending, begin_run, end_run
#   - SignalBool: get, set, toggle, read, peek_i32, version, __str__, copy
#   - SignalString: get, peek, read, set, version, is_empty, __str__, copy
#   - StringStore: create, read, write, destroy, count, contains, reuse
#   - ComponentContext: create, use_signal, use_signal_bool, use_signal_string,
#     use_memo, use_effect, end_setup, register_template, setup_view,
#     register_view, on_click_add, on_click_sub, on_click_set, on_click_toggle,
#     on_input_set, vnode_builder, render_builder, mount, dispatch_event, flush,
#     has_dirty, consume_dirty, diff, finalize, destroy

from memory import UnsafePointer, alloc
from testing import assert_equal, assert_true, assert_false
from signals import (
    Runtime,
    StringStore,
    create_runtime,
    destroy_runtime,
    SignalI32,
    SignalBool,
    SignalString,
    MemoI32,
    EffectHandle,
)
from component import ComponentContext, EventBinding, AppShell, app_shell_create
from html import (
    Node,
    el_div,
    el_span,
    el_h1,
    el_button,
    text,
    dyn_text,
    dyn_attr,
    onclick_add,
    onclick_sub,
    class_if,
    class_when,
    text_when,
)
from bridge import MutationWriter


# ══════════════════════════════════════════════════════════════════════════════
# Helpers
# ══════════════════════════════════════════════════════════════════════════════


fn _create_runtime() -> UnsafePointer[Runtime, MutExternalOrigin]:
    return create_runtime()


fn _destroy_runtime(rt: UnsafePointer[Runtime, MutExternalOrigin]):
    destroy_runtime(rt)


fn _alloc_writer() -> UnsafePointer[MutationWriter, MutExternalOrigin]:
    var buf_ptr = alloc[UInt8](8192)
    var writer_ptr = alloc[MutationWriter](1)
    writer_ptr.init_pointee_move(MutationWriter(buf_ptr, 8192))
    return writer_ptr


fn _free_writer(writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin]):
    writer_ptr[0].buf.free()
    writer_ptr.destroy_pointee()
    writer_ptr.free()


# ══════════════════════════════════════════════════════════════════════════════
# SignalI32 tests
# ══════════════════════════════════════════════════════════════════════════════


def test_signal_i32_peek():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(42))
    var sig = SignalI32(key, rt)
    assert_equal(sig.peek(), 42, "peek should return initial value")
    _destroy_runtime(rt)


def test_signal_i32_read():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    _ = rt[0].begin_scope_render(scope_id)
    var key = rt[0].create_signal[Int32](Int32(10))
    var sig = SignalI32(key, rt)
    var val = sig.read()
    assert_equal(val, 10, "read should return initial value")
    rt[0].end_scope_render(-1)
    _destroy_runtime(rt)


def test_signal_i32_set():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(0))
    var sig = SignalI32(key, rt)
    sig.set(99)
    assert_equal(sig.peek(), 99, "set should update value")
    _destroy_runtime(rt)


def test_signal_i32_iadd():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(10))
    var sig = SignalI32(key, rt)
    sig += 5
    assert_equal(sig.peek(), 15, "+= should add to value")
    _destroy_runtime(rt)


def test_signal_i32_isub():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(10))
    var sig = SignalI32(key, rt)
    sig -= 3
    assert_equal(sig.peek(), 7, "-= should subtract from value")
    _destroy_runtime(rt)


def test_signal_i32_imul():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(5))
    var sig = SignalI32(key, rt)
    sig *= 4
    assert_equal(sig.peek(), 20, "*= should multiply value")
    _destroy_runtime(rt)


def test_signal_i32_ifloordiv():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(17))
    var sig = SignalI32(key, rt)
    sig //= 5
    assert_equal(sig.peek(), 3, "//= should floor-divide value")
    _destroy_runtime(rt)


def test_signal_i32_imod():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(17))
    var sig = SignalI32(key, rt)
    sig %= 5
    assert_equal(sig.peek(), 2, "%= should modulo value")
    _destroy_runtime(rt)


def test_signal_i32_toggle_from_zero():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(0))
    var sig = SignalI32(key, rt)
    sig.toggle()
    assert_equal(sig.peek(), 1, "toggle from 0 should become 1")
    _destroy_runtime(rt)


def test_signal_i32_toggle_from_one():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(1))
    var sig = SignalI32(key, rt)
    sig.toggle()
    assert_equal(sig.peek(), 0, "toggle from 1 should become 0")
    _destroy_runtime(rt)


def test_signal_i32_toggle_round_trip():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(0))
    var sig = SignalI32(key, rt)
    sig.toggle()
    sig.toggle()
    assert_equal(sig.peek(), 0, "double toggle should return to 0")
    _destroy_runtime(rt)


def test_signal_i32_version_increments():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(0))
    var sig = SignalI32(key, rt)
    var v0 = sig.version()
    sig.set(1)
    var v1 = sig.version()
    sig.set(2)
    var v2 = sig.version()
    assert_true(Int(v1) > Int(v0), "version should increase after write")
    assert_true(Int(v2) > Int(v1), "version should increase after second write")
    _destroy_runtime(rt)


def test_signal_i32_str():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(42))
    var sig = SignalI32(key, rt)
    var s = String(sig)
    assert_equal(s, "42", "__str__ should return value as string")
    _destroy_runtime(rt)


def test_signal_i32_copy():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(5))
    var sig1 = SignalI32(key, rt)
    var sig2 = sig1.copy()
    sig2 += 10
    # Both handles point to the same signal
    assert_equal(sig1.peek(), 15, "copy shares underlying signal")
    assert_equal(sig2.peek(), 15, "copy shares underlying signal")
    _destroy_runtime(rt)


def test_signal_i32_chained_ops():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(100))
    var sig = SignalI32(key, rt)
    sig += 50
    sig -= 30
    sig *= 2
    sig //= 3
    sig %= 7
    # (100 + 50 - 30) * 2 = 240; 240 // 3 = 80; 80 % 7 = 3
    assert_equal(sig.peek(), 3, "chained operators should compose correctly")
    _destroy_runtime(rt)


def test_signal_i32_marks_scope_dirty():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    _ = rt[0].begin_scope_render(scope_id)
    var key = rt[0].create_signal[Int32](Int32(0))
    var sig = SignalI32(key, rt)
    _ = sig.read()  # subscribe scope
    rt[0].end_scope_render(-1)
    assert_false(rt[0].has_dirty(), "no dirty scopes initially")
    sig += 1
    assert_true(rt[0].has_dirty(), "signal write should mark scope dirty")
    _destroy_runtime(rt)


def test_signal_i32_negative_values():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](Int32(-10))
    var sig = SignalI32(key, rt)
    assert_equal(sig.peek(), -10, "negative initial value")
    sig -= 5
    assert_equal(sig.peek(), -15, "subtraction into deeper negative")
    _destroy_runtime(rt)


# ══════════════════════════════════════════════════════════════════════════════
# MemoI32 tests
# ══════════════════════════════════════════════════════════════════════════════


def test_memo_i32_is_dirty_initially():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)
    assert_true(memo.is_dirty(), "memo should be dirty initially")
    _destroy_runtime(rt)


def test_memo_i32_compute_clears_dirty():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)
    memo.begin_compute()
    memo.end_compute(42)
    assert_false(memo.is_dirty(), "memo should not be dirty after compute")
    _destroy_runtime(rt)


def test_memo_i32_read_after_compute():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)
    memo.begin_compute()
    memo.end_compute(77)
    var val = memo.read()
    assert_equal(val, 77, "read should return computed value")
    _destroy_runtime(rt)


def test_memo_i32_peek_after_compute():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)
    memo.begin_compute()
    memo.end_compute(33)
    var val = memo.peek()
    assert_equal(
        val, 33, "peek should return computed value without subscribing"
    )
    _destroy_runtime(rt)


def test_memo_i32_recompute_from():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)
    memo.recompute_from(55)
    assert_false(memo.is_dirty(), "recompute_from should clear dirty")
    assert_equal(memo.peek(), 55, "recompute_from should cache value")
    _destroy_runtime(rt)


def test_memo_i32_str():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)
    memo.recompute_from(99)
    var s = String(memo)
    assert_equal(s, "99", "__str__ should return cached value as string")
    _destroy_runtime(rt)


def test_memo_i32_tracks_signal_deps():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var sig_key = rt[0].create_signal[Int32](Int32(5))
    var sig = SignalI32(sig_key, rt)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var memo = MemoI32(memo_id, rt)

    # First compute: read the signal to subscribe
    memo.begin_compute()
    var val = sig.read()
    memo.end_compute(val * 2)
    assert_equal(memo.peek(), 10, "first compute: 5*2=10")
    assert_false(memo.is_dirty(), "clean after compute")

    # Write signal → memo should become dirty
    sig.set(7)
    assert_true(memo.is_dirty(), "memo dirty after signal write")

    # Recompute
    memo.begin_compute()
    var val2 = sig.read()
    memo.end_compute(val2 * 2)
    assert_equal(memo.peek(), 14, "recompute: 7*2=14")
    assert_false(memo.is_dirty(), "clean after recompute")
    _destroy_runtime(rt)


def test_memo_i32_copy():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var memo_id = rt[0].create_memo_i32(scope_id, Int32(0))
    var m1 = MemoI32(memo_id, rt)
    var m2 = m1.copy()
    m1.recompute_from(123)
    assert_equal(m2.peek(), 123, "copy shares underlying memo")
    _destroy_runtime(rt)


# ══════════════════════════════════════════════════════════════════════════════
# EffectHandle tests
# ══════════════════════════════════════════════════════════════════════════════


def test_effect_handle_is_pending_initially():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var effect_id = rt[0].create_effect(scope_id)
    var fx = EffectHandle(UInt32(effect_id), rt)
    assert_true(fx.is_pending(), "effect should be pending initially")
    _destroy_runtime(rt)


def test_effect_handle_begin_end_run_clears_pending():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var effect_id = rt[0].create_effect(scope_id)
    var fx = EffectHandle(UInt32(effect_id), rt)
    fx.begin_run()
    fx.end_run()
    assert_false(fx.is_pending(), "effect should not be pending after run")
    _destroy_runtime(rt)


def test_effect_handle_tracks_signal_deps():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var sig_key = rt[0].create_signal[Int32](Int32(0))
    var sig = SignalI32(sig_key, rt)
    var effect_id = rt[0].create_effect(scope_id)
    var fx = EffectHandle(UInt32(effect_id), rt)

    # First run: read signal to subscribe
    fx.begin_run()
    _ = sig.read()
    fx.end_run()
    assert_false(fx.is_pending(), "not pending after run")

    # Write signal → effect should become pending
    sig.set(1)
    assert_true(fx.is_pending(), "pending after signal write")
    _destroy_runtime(rt)


def test_effect_handle_copy():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    var effect_id = rt[0].create_effect(scope_id)
    var fx1 = EffectHandle(UInt32(effect_id), rt)
    var fx2 = fx1.copy()
    fx1.begin_run()
    fx1.end_run()
    assert_false(fx2.is_pending(), "copy shares underlying effect")
    _destroy_runtime(rt)


# ══════════════════════════════════════════════════════════════════════════════
# SignalBool tests
# ══════════════════════════════════════════════════════════════════════════════


def test_signal_bool_get_false():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    assert_false(sig.get(), "signal initialized to 0 should be False")
    _destroy_runtime(rt)


def test_signal_bool_get_true():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](1)
    var sig = SignalBool(UInt32(key), rt)
    assert_true(sig.get(), "signal initialized to 1 should be True")
    _destroy_runtime(rt)


def test_signal_bool_set_true():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    sig.set(True)
    assert_true(sig.get(), "after set(True) should be True")
    _destroy_runtime(rt)


def test_signal_bool_set_false():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](1)
    var sig = SignalBool(UInt32(key), rt)
    sig.set(False)
    assert_false(sig.get(), "after set(False) should be False")
    _destroy_runtime(rt)


def test_signal_bool_toggle_from_false():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    sig.toggle()
    assert_true(sig.get(), "toggle from False should be True")
    _destroy_runtime(rt)


def test_signal_bool_toggle_from_true():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](1)
    var sig = SignalBool(UInt32(key), rt)
    sig.toggle()
    assert_false(sig.get(), "toggle from True should be False")
    _destroy_runtime(rt)


def test_signal_bool_toggle_round_trip():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    sig.toggle()
    assert_true(sig.get(), "first toggle: True")
    sig.toggle()
    assert_false(sig.get(), "second toggle: False")
    sig.toggle()
    assert_true(sig.get(), "third toggle: True")
    _destroy_runtime(rt)


def test_signal_bool_read_subscribes():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    _ = rt[0].begin_scope_render(scope_id)
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    _ = sig.read()  # subscribe scope
    rt[0].end_scope_render(-1)
    assert_false(rt[0].has_dirty(), "no dirty scopes initially")
    sig.set(True)
    assert_true(
        rt[0].has_dirty(),
        "scope should be dirty after bool write",
    )
    _destroy_runtime(rt)


def test_signal_bool_peek_i32():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    assert_equal(sig.peek_i32(), Int32(0), "peek_i32 should return 0 for False")
    sig.set(True)
    assert_equal(sig.peek_i32(), Int32(1), "peek_i32 should return 1 for True")
    _destroy_runtime(rt)


def test_signal_bool_version_increments():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    var v0 = sig.version()
    sig.set(True)
    var v1 = sig.version()
    assert_true(v1 > v0, "version should increment after write")
    sig.toggle()
    var v2 = sig.version()
    assert_true(v2 > v1, "version should increment after toggle")
    _destroy_runtime(rt)


def test_signal_bool_str_true():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](1)
    var sig = SignalBool(UInt32(key), rt)
    assert_equal(String(sig), String("true"), "__str__ should return 'true'")
    _destroy_runtime(rt)


def test_signal_bool_str_false():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig = SignalBool(UInt32(key), rt)
    assert_equal(String(sig), String("false"), "__str__ should return 'false'")
    _destroy_runtime(rt)


def test_signal_bool_copy():
    var rt = _create_runtime()
    var key = rt[0].create_signal[Int32](0)
    var sig1 = SignalBool(UInt32(key), rt)
    var sig2 = sig1.copy()
    sig1.set(True)
    assert_true(sig2.get(), "copy shares underlying signal")
    _destroy_runtime(rt)


# ══════════════════════════════════════════════════════════════════════════════
# Conditional helper tests
# ══════════════════════════════════════════════════════════════════════════════


def test_class_if_true():
    var result = class_if(True, String("active"))
    assert_equal(result, String("active"), "class_if(True) should return name")


def test_class_if_false():
    var result = class_if(False, String("active"))
    assert_equal(result, String(""), "class_if(False) should return empty")


def test_class_when_true():
    var result = class_when(True, String("open"), String("closed"))
    assert_equal(
        result, String("open"), "class_when(True) should return true_class"
    )


def test_class_when_false():
    var result = class_when(False, String("open"), String("closed"))
    assert_equal(
        result, String("closed"), "class_when(False) should return false_class"
    )


def test_text_when_true():
    var result = text_when(True, String("yes"), String("no"))
    assert_equal(
        result, String("yes"), "text_when(True) should return true_text"
    )


def test_text_when_false():
    var result = text_when(False, String("yes"), String("no"))
    assert_equal(
        result, String("no"), "text_when(False) should return false_text"
    )


def test_class_if_with_empty_name():
    var result = class_if(True, String(""))
    assert_equal(result, String(""), "class_if with empty name returns empty")


def test_text_when_with_expression():
    var x = 5
    var result = text_when(x > 3, String("big"), String("small"))
    assert_equal(result, String("big"), "text_when with expression condition")


# ══════════════════════════════════════════════════════════════════════════════
# ComponentContext tests
# ══════════════════════════════════════════════════════════════════════════════


# ── ComponentContext — use_signal_bool / create_signal_bool ───────────────────


def test_ctx_use_signal_bool_true():
    var ctx = ComponentContext.create()
    var flag = ctx.use_signal_bool(True)
    ctx.end_setup()
    assert_true(flag.get(), "use_signal_bool(True) should be True")
    ctx.destroy()


def test_ctx_use_signal_bool_false():
    var ctx = ComponentContext.create()
    var flag = ctx.use_signal_bool(False)
    ctx.end_setup()
    assert_false(flag.get(), "use_signal_bool(False) should be False")
    ctx.destroy()


def test_ctx_use_signal_bool_subscribes_scope():
    var ctx = ComponentContext.create()
    var flag = ctx.use_signal_bool(False)
    ctx.end_setup()
    # Writing should mark the scope dirty
    flag.set(True)
    assert_true(ctx.consume_dirty(), "scope should be dirty after bool write")
    ctx.destroy()


def test_ctx_create_signal_bool_true():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var flag = ctx.create_signal_bool(True)
    assert_true(flag.get(), "create_signal_bool(True) should be True")
    ctx.destroy()


def test_ctx_create_signal_bool_false():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var flag = ctx.create_signal_bool(False)
    assert_false(flag.get(), "create_signal_bool(False) should be False")
    ctx.destroy()


def test_ctx_signal_bool_toggle_lifecycle():
    """Test full SignalBool lifecycle through ComponentContext."""
    var ctx = ComponentContext.create()
    var flag = ctx.use_signal_bool(False)
    ctx.end_setup()
    assert_false(flag.get(), "initial: False")
    flag.toggle()
    assert_true(flag.get(), "after toggle: True")
    assert_true(ctx.consume_dirty(), "dirty after toggle")
    flag.toggle()
    assert_false(flag.get(), "after second toggle: False")
    ctx.destroy()


def test_ctx_create_destroy():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    assert_true(ctx.shell.is_alive(), "shell should be alive after create")
    ctx.destroy()
    assert_false(
        ctx.shell.is_alive(), "shell should not be alive after destroy"
    )


def test_ctx_use_signal():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    assert_equal(count.peek(), 0, "signal initial value")
    count += 5
    assert_equal(count.peek(), 5, "signal updated via handle")
    ctx.destroy()


def test_ctx_use_signal_subscribes_scope():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    assert_false(ctx.has_dirty(), "no dirty initially")
    count.set(1)
    assert_true(ctx.has_dirty(), "scope dirty after signal write")
    ctx.destroy()


def test_ctx_use_memo():
    var ctx = ComponentContext.create()
    var doubled = ctx.use_memo(0)
    ctx.end_setup()
    assert_true(doubled.is_dirty(), "memo dirty initially")
    doubled.recompute_from(42)
    assert_equal(doubled.peek(), 42, "memo value after recompute")
    ctx.destroy()


def test_ctx_use_memo_subscribes_scope():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(5)
    var doubled = ctx.use_memo(0)
    ctx.end_setup()
    # Compute the memo (reads count signal, subscribing memo to it)
    doubled.begin_compute()
    var val = count.read()
    doubled.end_compute(val * 2)
    # Consume the dirty state from initial setup
    _ = ctx.consume_dirty()
    assert_false(ctx.has_dirty(), "clean after consume")
    # Writing the input signal should mark the memo dirty AND the scope
    # dirty (scope is subscribed to count signal via use_signal)
    count.set(10)
    assert_true(
        ctx.has_dirty(), "scope dirty after signal write (memo dep chain)"
    )
    ctx.destroy()


def test_ctx_use_effect():
    var ctx = ComponentContext.create()
    var fx = ctx.use_effect()
    ctx.end_setup()
    assert_true(fx.is_pending(), "effect pending initially")
    fx.begin_run()
    fx.end_run()
    assert_false(fx.is_pending(), "effect not pending after run")
    ctx.destroy()


def test_ctx_create_signal_no_hook():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var sig = ctx.create_signal(100)
    assert_equal(sig.peek(), 100, "create_signal without hook system")
    sig += 1
    assert_equal(sig.peek(), 101, "operator works on non-hook signal")
    ctx.destroy()


def test_ctx_create_memo_no_hook():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var memo = ctx.create_memo(0)
    assert_true(memo.is_dirty(), "non-hook memo starts dirty")
    memo.recompute_from(77)
    assert_equal(memo.peek(), 77, "non-hook memo value")
    ctx.destroy()


def test_ctx_create_effect_no_hook():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var fx = ctx.create_effect()
    assert_true(fx.is_pending(), "non-hook effect starts pending")
    ctx.destroy()


def test_ctx_register_template():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var view = el_div(
        [
            el_span([dyn_text(0)]),
        ]
    )
    ctx.register_template(view, String("test-tmpl"))
    assert_true(Int(ctx.template_id) >= 0, "template ID should be non-negative")
    ctx.destroy()


def test_ctx_on_click_add():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    var handler_id = ctx.on_click_add(count, 1)
    assert_true(Int(handler_id) >= 0, "handler ID should be valid")
    # Dispatch the click event
    var executed = ctx.dispatch_event(handler_id, 0)  # EVT_CLICK = 0
    assert_true(executed, "handler should execute")
    assert_equal(count.peek(), 1, "count should be 1 after click add")
    ctx.destroy()


def test_ctx_on_click_sub():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(10)
    ctx.end_setup()
    var handler_id = ctx.on_click_sub(count, 3)
    _ = ctx.dispatch_event(handler_id, 0)
    assert_equal(count.peek(), 7, "count should be 7 after click sub")
    ctx.destroy()


def test_ctx_on_click_set():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    var handler_id = ctx.on_click_set(count, 42)
    _ = ctx.dispatch_event(handler_id, 0)
    assert_equal(count.peek(), 42, "count should be 42 after click set")
    ctx.destroy()


def test_ctx_on_click_toggle():
    var ctx = ComponentContext.create()
    var flag = ctx.use_signal(0)
    ctx.end_setup()
    var handler_id = ctx.on_click_toggle(flag)
    _ = ctx.dispatch_event(handler_id, 0)
    assert_equal(flag.peek(), 1, "flag should be 1 after toggle")
    _ = ctx.dispatch_event(handler_id, 0)
    assert_equal(flag.peek(), 0, "flag should be 0 after second toggle")
    ctx.destroy()


def test_ctx_on_input_set():
    var ctx = ComponentContext.create()
    var text_sig = ctx.use_signal(0)
    ctx.end_setup()
    var handler_id = ctx.on_input_set(text_sig)
    assert_true(Int(handler_id) >= 0, "input handler ID should be valid")
    ctx.destroy()


def test_ctx_vnode_builder():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var view = el_div(
        [
            el_span([dyn_text(0)]),
            el_button([text(String("+")), dyn_attr(0)]),
        ]
    )
    ctx.register_template(view, String("vb-test"))
    var vb = ctx.vnode_builder()
    vb.add_dyn_text(String("hello"))
    vb.add_dyn_event(String("click"), UInt32(0))
    var idx = vb.index()
    assert_true(Int(idx) >= 0, "VNode index should be valid")
    ctx.destroy()


def test_ctx_mount_produces_mutations():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var view = el_div(
        [
            el_span([dyn_text(0)]),
        ]
    )
    ctx.register_template(view, String("mount-test"))
    var vb = ctx.vnode_builder()
    vb.add_dyn_text(String("test"))
    var vnode_idx = vb.index()
    var writer_ptr = _alloc_writer()
    var len = ctx.mount(writer_ptr, vnode_idx)
    assert_true(Int(len) > 0, "mount should produce mutations")
    _free_writer(writer_ptr)
    ctx.destroy()


def test_ctx_consume_dirty_after_signal_write():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    count.set(1)
    assert_true(ctx.has_dirty(), "dirty after write")
    var consumed = ctx.consume_dirty()
    assert_true(consumed, "consume_dirty should return True")
    assert_false(ctx.has_dirty(), "not dirty after consume")
    ctx.destroy()


def test_ctx_consume_dirty_when_clean():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    var consumed = ctx.consume_dirty()
    assert_false(consumed, "consume_dirty returns False when clean")
    ctx.destroy()


def test_ctx_full_counter_lifecycle():
    """End-to-end test: create counter, mount, click, flush, verify."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()

    # Register template
    var view = el_div(
        [
            el_span([dyn_text(0)]),
            el_button([text(String("+")), dyn_attr(0)]),
            el_button([text(String("-")), dyn_attr(1)]),
        ]
    )
    ctx.register_template(view, String("counter-lc"))

    # Register handlers
    var incr = ctx.on_click_add(count, 1)
    var decr = ctx.on_click_sub(count, 1)

    # Initial mount
    var vb = ctx.vnode_builder()
    vb.add_dyn_text(String("Count: 0"))
    vb.add_dyn_event(String("click"), incr)
    vb.add_dyn_event(String("click"), decr)
    var vnode_idx = vb.index()

    var writer_ptr = _alloc_writer()
    var mount_len = ctx.mount(writer_ptr, vnode_idx)
    assert_true(Int(mount_len) > 0, "mount should produce mutations")
    _free_writer(writer_ptr)

    # Click increment 3 times
    for _ in range(3):
        _ = ctx.dispatch_event(incr, 0)
    assert_equal(count.peek(), 3, "count should be 3 after 3 increments")

    # Flush (consume dirty + diff)
    assert_true(ctx.has_dirty(), "dirty after clicks")
    _ = ctx.consume_dirty()

    var vb2 = ctx.vnode_builder()
    vb2.add_dyn_text(String("Count: 3"))
    vb2.add_dyn_event(String("click"), incr)
    vb2.add_dyn_event(String("click"), decr)
    var new_idx = vb2.index()

    var writer_ptr2 = _alloc_writer()
    ctx.diff(writer_ptr2, new_idx)
    var flush_len = ctx.finalize(writer_ptr2)
    assert_true(Int(flush_len) > 0, "diff should produce mutations")
    _free_writer(writer_ptr2)

    # Click decrement
    _ = ctx.dispatch_event(decr, 0)
    assert_equal(count.peek(), 2, "count should be 2 after decrement")

    ctx.destroy()


def test_ctx_multiple_signals():
    var ctx = ComponentContext.create()
    var a = ctx.use_signal(10)
    var b = ctx.use_signal(20)
    var c = ctx.use_signal(30)
    ctx.end_setup()

    a += 1
    b -= 1
    c *= 2

    assert_equal(a.peek(), 11, "signal a")
    assert_equal(b.peek(), 19, "signal b")
    assert_equal(c.peek(), 60, "signal c")
    ctx.destroy()


def test_ctx_signal_and_memo_integration():
    """Test signal → memo dependency chain via ComponentContext."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(5)
    var doubled = ctx.use_memo(0)
    ctx.end_setup()

    # First memo compute
    assert_true(doubled.is_dirty(), "memo dirty initially")
    doubled.begin_compute()
    var val = count.read()
    doubled.end_compute(val * 2)
    assert_equal(doubled.peek(), 10, "5 * 2 = 10")

    # Write signal → memo dirty
    count.set(8)
    assert_true(doubled.is_dirty(), "memo dirty after signal write")

    # Recompute
    doubled.begin_compute()
    var val2 = count.read()
    doubled.end_compute(val2 * 2)
    assert_equal(doubled.peek(), 16, "8 * 2 = 16")
    ctx.destroy()


def test_ctx_signal_memo_effect_integration():
    """Test signal → memo → effect chain via ComponentContext."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    var doubled = ctx.use_memo(0)
    var fx = ctx.use_effect()
    ctx.end_setup()

    # Initial effect run: subscribe to count signal
    fx.begin_run()
    _ = count.read()
    fx.end_run()
    assert_false(fx.is_pending(), "effect not pending after run")

    # Write signal → effect pending
    count.set(1)
    assert_true(fx.is_pending(), "effect pending after signal write")

    # Run effect again
    fx.begin_run()
    _ = count.read()
    fx.end_run()
    assert_false(fx.is_pending(), "effect not pending after re-run")
    ctx.destroy()


def test_ctx_on_event_add_generic():
    """Test generic event handler registration."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    var handler_id = ctx.on_event_add(String("keydown"), count, 10)
    assert_true(Int(handler_id) >= 0, "handler ID valid")
    # Dispatch with keydown event type (EVT_KEY_DOWN = 2)
    var executed = ctx.dispatch_event(handler_id, 2)
    assert_true(executed, "handler should execute")
    assert_equal(count.peek(), 10, "count should be 10 after event")
    ctx.destroy()


def test_ctx_on_event_sub_generic():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(100)
    ctx.end_setup()
    var handler_id = ctx.on_event_sub(String("keydown"), count, 25)
    _ = ctx.dispatch_event(handler_id, 2)
    assert_equal(count.peek(), 75, "count should be 75 after event sub")
    ctx.destroy()


def test_ctx_on_event_set_generic():
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.end_setup()
    var handler_id = ctx.on_event_set(String("submit"), count, 999)
    _ = ctx.dispatch_event(handler_id, 7)  # EVT_SUBMIT = 7
    assert_equal(count.peek(), 999, "count should be 999 after event set")
    ctx.destroy()


def test_ctx_runtime_ptr():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    assert_true(
        ctx.runtime_ptr() == ctx.shell.runtime,
        "runtime_ptr should match shell.runtime",
    )
    ctx.destroy()


def test_ctx_store_ptr():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    assert_true(
        ctx.store_ptr() == ctx.shell.store,
        "store_ptr should match shell.store",
    )
    ctx.destroy()


def test_ctx_double_destroy():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    ctx.destroy()
    ctx.destroy()  # Should be safe


def test_ctx_setup_view_basic():
    """Test setup_view() combines end_setup + register_view."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    # setup_view combines end_setup + register_view
    ctx.setup_view(
        el_div(
            [
                el_h1([dyn_text(0)]),
                el_button(
                    [
                        text(String("Up")),
                        onclick_add(count, 1),
                    ]
                ),
            ]
        ),
        String("setup-view-test"),
    )
    assert_true(Int(ctx.template_id) >= 0, "template ID valid after setup_view")
    var events = ctx.view_events()
    assert_equal(len(events), 1, "one event binding registered")
    assert_equal(events[0].event_name, "click", "event is click")
    ctx.destroy()


def test_ctx_setup_view_auto_dyn_text():
    """Test auto-numbered dyn_text() via setup_view."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    # Use dyn_text() without explicit index — auto-numbered
    ctx.setup_view(
        el_div(
            [
                el_h1([dyn_text()]),
                el_span([dyn_text()]),
            ]
        ),
        String("auto-dyn-text-test"),
    )
    assert_true(Int(ctx.template_id) >= 0, "template ID valid")
    # Build a VNode and verify both dyn_text slots work
    var vb = ctx.render_builder()
    vb.add_dyn_text(String("first"))
    vb.add_dyn_text(String("second"))
    var idx = vb.build()
    assert_true(Int(idx) >= 0, "VNode index valid")
    ctx.destroy()


def test_ctx_setup_view_with_render_builder():
    """Test setup_view + render_builder end-to-end."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.setup_view(
        el_div(
            [
                el_h1([dyn_text()]),
                el_button(
                    [
                        text(String("Up high!")),
                        onclick_add(count, 1),
                    ]
                ),
                el_button(
                    [
                        text(String("Down low!")),
                        onclick_sub(count, 1),
                    ]
                ),
            ]
        ),
        String("render-builder-test"),
    )
    var events = ctx.view_events()
    assert_equal(len(events), 2, "two event bindings (incr + decr)")

    # Build VNode via render_builder (auto-adds event attrs)
    var vb = ctx.render_builder()
    vb.add_dyn_text(String("High-Five counter: 0"))
    var idx = vb.build()
    assert_true(Int(idx) >= 0, "VNode index valid from render_builder")
    ctx.destroy()


def test_ctx_flush_convenience():
    """Test flush() combines diff + finalize."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.setup_view(
        el_div(
            [
                el_h1([dyn_text()]),
                el_button(
                    [
                        text(String("+")),
                        onclick_add(count, 1),
                    ]
                ),
            ]
        ),
        String("flush-test"),
    )

    # Initial mount
    var vb = ctx.render_builder()
    vb.add_dyn_text(String("Count: 0"))
    var vnode_idx = vb.build()
    var writer_ptr = _alloc_writer()
    var mount_len = ctx.mount(writer_ptr, vnode_idx)
    assert_true(Int(mount_len) > 0, "mount should produce mutations")
    _free_writer(writer_ptr)

    # Dispatch click
    var events = ctx.view_events()
    _ = ctx.dispatch_event(events[0].handler_id, 0)
    assert_equal(count.peek(), 1, "count should be 1 after click")

    # Flush using convenience method
    assert_true(ctx.consume_dirty(), "dirty after click")
    var vb2 = ctx.render_builder()
    vb2.add_dyn_text(String("Count: 1"))
    var new_idx = vb2.build()
    var writer_ptr2 = _alloc_writer()
    var flush_len = ctx.flush(writer_ptr2, new_idx)
    assert_true(Int(flush_len) > 0, "flush should produce mutations")
    _free_writer(writer_ptr2)

    ctx.destroy()


def test_ctx_dioxus_style_counter_lifecycle():
    """Full Dioxus-style counter lifecycle: init, mount, click, flush.

    Mirrors the pattern used by CounterApp.__init__() with setup_view(),
    auto-numbered dyn_text(), render_builder(), and flush().
    """
    # ── Setup (like CounterApp.__init__) ──
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    ctx.setup_view(
        el_div(
            [
                el_h1([dyn_text()]),
                el_button(
                    [
                        text(String("Up high!")),
                        onclick_add(count, 1),
                    ]
                ),
                el_button(
                    [
                        text(String("Down low!")),
                        onclick_sub(count, 1),
                    ]
                ),
            ]
        ),
        String("dioxus-counter"),
    )

    var events = ctx.view_events()
    assert_equal(len(events), 2, "two event bindings")
    var incr = events[0].handler_id
    var decr = events[1].handler_id

    # ── Mount (like counter_app_rebuild) ──
    var vb = ctx.render_builder()
    vb.add_dyn_text(String("High-Five counter: 0"))
    var vnode_idx = vb.build()
    var writer_ptr = _alloc_writer()
    var mount_len = ctx.mount(writer_ptr, vnode_idx)
    assert_true(Int(mount_len) > 0, "mount produces mutations")
    _free_writer(writer_ptr)

    # ── Increment 3 times ──
    for _ in range(3):
        _ = ctx.dispatch_event(incr, 0)
    assert_equal(count.peek(), 3, "count is 3 after 3 increments")

    # ── Flush (like counter_app_flush) ──
    assert_true(ctx.consume_dirty(), "dirty after increments")
    var vb2 = ctx.render_builder()
    vb2.add_dyn_text(String("High-Five counter: 3"))
    var new_idx = vb2.build()
    var writer_ptr2 = _alloc_writer()
    var flush_len = ctx.flush(writer_ptr2, new_idx)
    assert_true(Int(flush_len) > 0, "flush produces mutations")
    _free_writer(writer_ptr2)

    # ── Decrement ──
    _ = ctx.dispatch_event(decr, 0)
    assert_equal(count.peek(), 2, "count is 2 after decrement")

    # ── Flush again ──
    assert_true(ctx.consume_dirty(), "dirty after decrement")
    var vb3 = ctx.render_builder()
    vb3.add_dyn_text(String("High-Five counter: 2"))
    var new_idx2 = vb3.build()
    var writer_ptr3 = _alloc_writer()
    var flush_len2 = ctx.flush(writer_ptr3, new_idx2)
    assert_true(Int(flush_len2) > 0, "second flush produces mutations")
    _free_writer(writer_ptr3)

    # ── No-op flush when clean ──
    assert_false(ctx.consume_dirty(), "not dirty when no events")

    ctx.destroy()


def test_ctx_vnode_builder_keyed():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var view = el_div([dyn_text(0)])
    ctx.register_template(view, String("keyed-test"))
    var vb = ctx.vnode_builder_keyed(String("item-1"))
    vb.add_dyn_text(String("hello"))
    var idx = vb.index()
    assert_true(Int(idx) >= 0, "keyed VNode index should be valid")
    ctx.destroy()


def test_ctx_vnode_builder_for():
    """Test building a VNode for a specific template ID."""
    var ctx = ComponentContext.create()
    ctx.end_setup()
    # Register two templates
    var view1 = el_div([dyn_text(0)])
    ctx.register_template(view1, String("tmpl-1"))
    var tmpl1_id = ctx.template_id

    var view2 = el_span([dyn_text(0)])
    # Register second template manually
    from html import to_template

    var tmpl2 = to_template(view2, String("tmpl-2"))
    var tmpl2_id = UInt32(ctx.shell.runtime[0].templates.register(tmpl2^))

    var vb = ctx.vnode_builder_for(tmpl2_id)
    vb.add_dyn_text(String("span text"))
    var idx = vb.index()
    assert_true(Int(idx) >= 0, "VNode for specific template should be valid")
    ctx.destroy()


# ══════════════════════════════════════════════════════════════════════════════
# StringStore unit tests
# ══════════════════════════════════════════════════════════════════════════════


def test_string_store_create_and_read():
    var store = StringStore()
    var key = store.create(String("hello"))
    assert_equal(
        store.read(key), String("hello"), "should read back initial value"
    )


def test_string_store_write():
    var store = StringStore()
    var key = store.create(String("hello"))
    store.write(key, String("world"))
    assert_equal(store.read(key), String("world"), "should read updated value")


def test_string_store_count():
    var store = StringStore()
    assert_equal(store.count(), 0, "empty store count should be 0")
    var k1 = store.create(String("a"))
    assert_equal(store.count(), 1, "count should be 1 after first create")
    var k2 = store.create(String("b"))
    assert_equal(store.count(), 2, "count should be 2 after second create")


def test_string_store_contains():
    var store = StringStore()
    var key = store.create(String("hi"))
    assert_true(store.contains(key), "should contain created key")
    assert_false(store.contains(UInt32(999)), "should not contain unknown key")


def test_string_store_destroy():
    var store = StringStore()
    var key = store.create(String("doomed"))
    assert_equal(store.count(), 1, "count before destroy")
    store.destroy(key)
    assert_equal(store.count(), 0, "count after destroy")
    assert_false(store.contains(key), "should not contain destroyed key")


def test_string_store_reuse_slot():
    """Destroying a slot should allow it to be reused by the next create."""
    var store = StringStore()
    var k1 = store.create(String("first"))
    store.destroy(k1)
    var k2 = store.create(String("second"))
    # The freed slot should be reused, so k2 == k1
    assert_equal(k2, k1, "should reuse freed slot")
    assert_equal(store.read(k2), String("second"), "reused slot has new value")


def test_string_store_multiple_entries():
    var store = StringStore()
    var k1 = store.create(String("alpha"))
    var k2 = store.create(String("beta"))
    var k3 = store.create(String("gamma"))
    assert_equal(store.read(k1), String("alpha"), "first entry")
    assert_equal(store.read(k2), String("beta"), "second entry")
    assert_equal(store.read(k3), String("gamma"), "third entry")
    assert_equal(store.count(), 3, "count should be 3")


def test_string_store_empty_string():
    var store = StringStore()
    var key = store.create(String(""))
    assert_equal(store.read(key), String(""), "should handle empty string")


def test_string_store_overwrite_multiple_times():
    var store = StringStore()
    var key = store.create(String("v1"))
    store.write(key, String("v2"))
    store.write(key, String("v3"))
    store.write(key, String("v4"))
    assert_equal(store.read(key), String("v4"), "should have latest value")


# ══════════════════════════════════════════════════════════════════════════════
# SignalString unit tests
# ══════════════════════════════════════════════════════════════════════════════


def test_signal_string_get():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("hello"))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_equal(sig.get(), String("hello"), "get should return initial value")
    _destroy_runtime(rt)


def test_signal_string_peek():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("world"))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_equal(
        sig.peek(), String("world"), "peek should return initial value"
    )
    _destroy_runtime(rt)


def test_signal_string_set():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("old"))
    var sig = SignalString(keys[0], keys[1], rt)
    sig.set(String("new"))
    assert_equal(sig.get(), String("new"), "get should return updated value")
    _destroy_runtime(rt)


def test_signal_string_set_empty():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("not empty"))
    var sig = SignalString(keys[0], keys[1], rt)
    sig.set(String(""))
    assert_equal(sig.get(), String(""), "set to empty string should work")
    _destroy_runtime(rt)


def test_signal_string_read_subscribes():
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    _ = rt[0].begin_scope_render(scope_id)
    var keys = rt[0].create_signal_string(String("initial"))
    var sig = SignalString(keys[0], keys[1], rt)
    _ = sig.read()  # subscribe scope via version signal
    rt[0].end_scope_render(-1)
    assert_false(rt[0].has_dirty(), "no dirty scopes initially")
    sig.set(String("changed"))
    assert_true(
        rt[0].has_dirty(),
        "scope should be dirty after string write",
    )
    _destroy_runtime(rt)


def test_signal_string_read_returns_value():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("readable"))
    var sig = SignalString(keys[0], keys[1], rt)
    var val = sig.read()
    assert_equal(val, String("readable"), "read should return current value")
    _destroy_runtime(rt)


def test_signal_string_version_increments():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("v0"))
    var sig = SignalString(keys[0], keys[1], rt)
    var v0 = sig.version()
    sig.set(String("v1"))
    var v1 = sig.version()
    assert_true(v1 > v0, "version should increment after first write")
    sig.set(String("v2"))
    var v2 = sig.version()
    assert_true(v2 > v1, "version should increment after second write")
    _destroy_runtime(rt)


def test_signal_string_is_empty_true():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_true(sig.is_empty(), "empty string should return True")
    _destroy_runtime(rt)


def test_signal_string_is_empty_false():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("content"))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_false(sig.is_empty(), "non-empty string should return False")
    _destroy_runtime(rt)


def test_signal_string_is_empty_after_set():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("something"))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_false(sig.is_empty(), "before clear")
    sig.set(String(""))
    assert_true(sig.is_empty(), "after set to empty")
    _destroy_runtime(rt)


def test_signal_string_str():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("display me"))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_equal(
        String(sig), String("display me"), "__str__ should return value"
    )
    _destroy_runtime(rt)


def test_signal_string_str_empty():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt)
    assert_equal(String(sig), String(""), "__str__ of empty string")
    _destroy_runtime(rt)


def test_signal_string_copy():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("shared"))
    var sig1 = SignalString(keys[0], keys[1], rt)
    var sig2 = sig1.copy()
    sig1.set(String("modified"))
    assert_equal(
        sig2.get(),
        String("modified"),
        "copy shares underlying signal",
    )
    _destroy_runtime(rt)


def test_signal_string_multiple_writes():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String(""))
    var sig = SignalString(keys[0], keys[1], rt)
    sig.set(String("one"))
    sig.set(String("two"))
    sig.set(String("three"))
    assert_equal(sig.get(), String("three"), "should have latest value")
    _destroy_runtime(rt)


def test_signal_string_concatenation_pattern():
    """Test the common pattern of building a display string from a signal."""
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("world"))
    var sig = SignalString(keys[0], keys[1], rt)
    var display = String("Hello, ") + sig.get() + String("!")
    assert_equal(display, String("Hello, world!"), "string concat pattern")
    _destroy_runtime(rt)


# ── Runtime string signal count ──────────────────────────────────────────────


def test_runtime_string_signal_count():
    var rt = _create_runtime()
    assert_equal(rt[0].string_signal_count(), 0, "initial count should be 0")
    var keys1 = rt[0].create_signal_string(String("a"))
    assert_equal(rt[0].string_signal_count(), 1, "count after first create")
    var keys2 = rt[0].create_signal_string(String("b"))
    assert_equal(rt[0].string_signal_count(), 2, "count after second create")
    _destroy_runtime(rt)


def test_runtime_destroy_signal_string():
    var rt = _create_runtime()
    var keys = rt[0].create_signal_string(String("temp"))
    assert_equal(rt[0].string_signal_count(), 1, "before destroy")
    rt[0].destroy_signal_string(keys[0], keys[1])
    assert_equal(rt[0].string_signal_count(), 0, "after destroy")
    _destroy_runtime(rt)


# ── Runtime hook-based string signal ─────────────────────────────────────────


def test_runtime_use_signal_string():
    """Hook-based string signal creation stores keys in scope hooks."""
    var rt = _create_runtime()
    var scope_id = rt[0].create_scope(0, -1)
    _ = rt[0].begin_scope_render(scope_id)
    var keys = rt[0].use_signal_string(String("hooked"))
    rt[0].end_scope_render(-1)
    # Verify the string was stored
    assert_equal(
        rt[0].peek_signal_string(keys[0]),
        String("hooked"),
        "hook should create string signal with initial value",
    )
    assert_equal(rt[0].string_signal_count(), 1, "one string signal created")
    _destroy_runtime(rt)


# ══════════════════════════════════════════════════════════════════════════════
# ComponentContext SignalString integration tests
# ══════════════════════════════════════════════════════════════════════════════


def test_ctx_use_signal_string():
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String("hello"))
    ctx.end_setup()
    assert_equal(name.get(), String("hello"), "initial value")
    ctx.destroy()


def test_ctx_use_signal_string_empty():
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String(""))
    ctx.end_setup()
    assert_equal(name.get(), String(""), "empty initial value")
    assert_true(name.is_empty(), "should be empty")
    ctx.destroy()


def test_ctx_use_signal_string_subscribes_scope():
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String("initial"))
    ctx.end_setup()
    # Drain any initial dirty scopes
    _ = ctx.consume_dirty()
    # Write to the string signal should make scope dirty
    name.set(String("changed"))
    assert_true(
        ctx.has_dirty(),
        "scope should be dirty after string write",
    )
    ctx.destroy()


def test_ctx_create_signal_string():
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var name = ctx.create_signal_string(String("created"))
    assert_equal(name.get(), String("created"), "create_signal_string value")
    ctx.destroy()


def test_ctx_create_signal_string_no_subscribe():
    """Create_signal_string should NOT auto-subscribe the scope."""
    var ctx = ComponentContext.create()
    ctx.end_setup()
    var name = ctx.create_signal_string(String("initial"))
    _ = ctx.consume_dirty()
    name.set(String("changed"))
    # No subscription means scope should not be dirty
    assert_false(
        ctx.has_dirty(),
        "create_signal_string should not subscribe scope",
    )
    ctx.destroy()


def test_ctx_signal_string_set_and_get():
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String("before"))
    ctx.end_setup()
    name.set(String("after"))
    assert_equal(name.get(), String("after"), "set/get round-trip")
    ctx.destroy()


def test_ctx_signal_string_version_lifecycle():
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String("v0"))
    ctx.end_setup()
    var v0 = name.version()
    name.set(String("v1"))
    var v1 = name.version()
    assert_true(v1 > v0, "version increments on write")
    name.set(String("v2"))
    var v2 = name.version()
    assert_true(v2 > v1, "version increments again")
    ctx.destroy()


def test_ctx_signal_string_str_interpolation():
    """Test the pattern of using __str__ for display text building."""
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String("Mojo"))
    ctx.end_setup()
    var display = String("Hello, ") + String(name) + String("!")
    assert_equal(display, String("Hello, Mojo!"), "str interpolation")
    ctx.destroy()


def test_ctx_signal_string_with_render_builder():
    """Test using a SignalString with RenderBuilder.add_dyn_text_signal()."""
    var ctx = ComponentContext.create()
    var name = ctx.use_signal_string(String("dynamic"))
    ctx.setup_view(
        el_div(el_h1(dyn_text())),
        String("str-test"),
    )
    var vb = ctx.render_builder()
    vb.add_dyn_text_signal(name)
    var idx = vb.build()
    assert_true(Int(idx) >= 0, "VNode should be valid")
    ctx.destroy()


def test_ctx_multiple_signal_strings():
    """Test creating multiple string signals in one component."""
    var ctx = ComponentContext.create()
    var first = ctx.use_signal_string(String("Alice"))
    var last = ctx.use_signal_string(String("Smith"))
    ctx.end_setup()
    assert_equal(first.get(), String("Alice"), "first signal")
    assert_equal(last.get(), String("Smith"), "second signal")
    first.set(String("Bob"))
    last.set(String("Jones"))
    assert_equal(first.get(), String("Bob"), "first after set")
    assert_equal(last.get(), String("Jones"), "second after set")
    ctx.destroy()


def test_ctx_signal_string_with_signal_i32():
    """Test mixing SignalString with SignalI32 in one component."""
    var ctx = ComponentContext.create()
    var count = ctx.use_signal(0)
    var label = ctx.use_signal_string(String("Count: 0"))
    ctx.end_setup()
    count += 1
    label.set(String("Count: ") + String(count.peek()))
    assert_equal(count.peek(), Int32(1), "i32 signal")
    assert_equal(label.get(), String("Count: 1"), "string signal")
    ctx.destroy()


# ══════════════════════════════════════════════════════════════════════════════
# Main — run all tests
# ══════════════════════════════════════════════════════════════════════════════


fn main() raises:
    var pass_count = 0
    var fail_count = 0

    # -- SignalI32 --
    try:
        test_signal_i32_peek()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_peek:", e)
        fail_count += 1

    try:
        test_signal_i32_read()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_read:", e)
        fail_count += 1

    try:
        test_signal_i32_set()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_set:", e)
        fail_count += 1

    try:
        test_signal_i32_iadd()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_iadd:", e)
        fail_count += 1

    try:
        test_signal_i32_isub()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_isub:", e)
        fail_count += 1

    try:
        test_signal_i32_imul()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_imul:", e)
        fail_count += 1

    try:
        test_signal_i32_ifloordiv()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_ifloordiv:", e)
        fail_count += 1

    try:
        test_signal_i32_imod()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_imod:", e)
        fail_count += 1

    try:
        test_signal_i32_toggle_from_zero()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_toggle_from_zero:", e)
        fail_count += 1

    try:
        test_signal_i32_toggle_from_one()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_toggle_from_one:", e)
        fail_count += 1

    try:
        test_signal_i32_toggle_round_trip()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_toggle_round_trip:", e)
        fail_count += 1

    try:
        test_signal_i32_version_increments()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_version_increments:", e)
        fail_count += 1

    try:
        test_signal_i32_str()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_str:", e)
        fail_count += 1

    try:
        test_signal_i32_copy()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_copy:", e)
        fail_count += 1

    try:
        test_signal_i32_chained_ops()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_chained_ops:", e)
        fail_count += 1

    try:
        test_signal_i32_marks_scope_dirty()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_marks_scope_dirty:", e)
        fail_count += 1

    try:
        test_signal_i32_negative_values()
        pass_count += 1
    except e:
        print("FAIL test_signal_i32_negative_values:", e)
        fail_count += 1

    # -- MemoI32 --
    try:
        test_memo_i32_is_dirty_initially()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_is_dirty_initially:", e)
        fail_count += 1

    try:
        test_memo_i32_compute_clears_dirty()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_compute_clears_dirty:", e)
        fail_count += 1

    try:
        test_memo_i32_read_after_compute()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_read_after_compute:", e)
        fail_count += 1

    try:
        test_memo_i32_peek_after_compute()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_peek_after_compute:", e)
        fail_count += 1

    try:
        test_memo_i32_recompute_from()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_recompute_from:", e)
        fail_count += 1

    try:
        test_memo_i32_str()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_str:", e)
        fail_count += 1

    try:
        test_memo_i32_tracks_signal_deps()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_tracks_signal_deps:", e)
        fail_count += 1

    try:
        test_memo_i32_copy()
        pass_count += 1
    except e:
        print("FAIL test_memo_i32_copy:", e)
        fail_count += 1

    # -- EffectHandle --
    try:
        test_effect_handle_is_pending_initially()
        pass_count += 1
    except e:
        print("FAIL test_effect_handle_is_pending_initially:", e)
        fail_count += 1

    try:
        test_effect_handle_begin_end_run_clears_pending()
        pass_count += 1
    except e:
        print("FAIL test_effect_handle_begin_end_run_clears_pending:", e)
        fail_count += 1

    try:
        test_effect_handle_tracks_signal_deps()
        pass_count += 1
    except e:
        print("FAIL test_effect_handle_tracks_signal_deps:", e)
        fail_count += 1

    try:
        test_effect_handle_copy()
        pass_count += 1
    except e:
        print("FAIL test_effect_handle_copy:", e)
        fail_count += 1

    # -- ComponentContext --
    try:
        test_ctx_create_destroy()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_destroy:", e)
        fail_count += 1

    try:
        test_ctx_use_signal()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal:", e)
        fail_count += 1

    try:
        test_ctx_use_signal_subscribes_scope()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_subscribes_scope:", e)
        fail_count += 1

    try:
        test_ctx_use_memo()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_memo:", e)
        fail_count += 1

    try:
        test_ctx_use_memo_subscribes_scope()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_memo_subscribes_scope:", e)
        fail_count += 1

    try:
        test_ctx_use_effect()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_effect:", e)
        fail_count += 1

    try:
        test_ctx_create_signal_no_hook()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_signal_no_hook:", e)
        fail_count += 1

    try:
        test_ctx_create_memo_no_hook()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_memo_no_hook:", e)
        fail_count += 1

    try:
        test_ctx_create_effect_no_hook()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_effect_no_hook:", e)
        fail_count += 1

    try:
        test_ctx_register_template()
        pass_count += 1
    except e:
        print("FAIL test_ctx_register_template:", e)
        fail_count += 1

    try:
        test_ctx_on_click_add()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_click_add:", e)
        fail_count += 1

    try:
        test_ctx_on_click_sub()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_click_sub:", e)
        fail_count += 1

    try:
        test_ctx_on_click_set()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_click_set:", e)
        fail_count += 1

    try:
        test_ctx_on_click_toggle()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_click_toggle:", e)
        fail_count += 1

    try:
        test_ctx_on_input_set()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_input_set:", e)
        fail_count += 1

    try:
        test_ctx_vnode_builder()
        pass_count += 1
    except e:
        print("FAIL test_ctx_vnode_builder:", e)
        fail_count += 1

    try:
        test_ctx_mount_produces_mutations()
        pass_count += 1
    except e:
        print("FAIL test_ctx_mount_produces_mutations:", e)
        fail_count += 1

    try:
        test_ctx_consume_dirty_after_signal_write()
        pass_count += 1
    except e:
        print("FAIL test_ctx_consume_dirty_after_signal_write:", e)
        fail_count += 1

    try:
        test_ctx_consume_dirty_when_clean()
        pass_count += 1
    except e:
        print("FAIL test_ctx_consume_dirty_when_clean:", e)
        fail_count += 1

    try:
        test_ctx_full_counter_lifecycle()
        pass_count += 1
    except e:
        print("FAIL test_ctx_full_counter_lifecycle:", e)
        fail_count += 1

    try:
        test_ctx_multiple_signals()
        pass_count += 1
    except e:
        print("FAIL test_ctx_multiple_signals:", e)
        fail_count += 1

    try:
        test_ctx_signal_and_memo_integration()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_and_memo_integration:", e)
        fail_count += 1

    try:
        test_ctx_signal_memo_effect_integration()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_memo_effect_integration:", e)
        fail_count += 1

    try:
        test_ctx_on_event_add_generic()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_event_add_generic:", e)
        fail_count += 1

    try:
        test_ctx_on_event_sub_generic()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_event_sub_generic:", e)
        fail_count += 1

    try:
        test_ctx_on_event_set_generic()
        pass_count += 1
    except e:
        print("FAIL test_ctx_on_event_set_generic:", e)
        fail_count += 1

    try:
        test_ctx_runtime_ptr()
        pass_count += 1
    except e:
        print("FAIL test_ctx_runtime_ptr:", e)
        fail_count += 1

    try:
        test_ctx_store_ptr()
        pass_count += 1
    except e:
        print("FAIL test_ctx_store_ptr:", e)
        fail_count += 1

    try:
        test_ctx_double_destroy()
        pass_count += 1
    except e:
        print("FAIL test_ctx_double_destroy:", e)
        fail_count += 1

    try:
        test_ctx_setup_view_basic()
        pass_count += 1
    except e:
        print("FAIL test_ctx_setup_view_basic:", e)
        fail_count += 1

    try:
        test_ctx_setup_view_auto_dyn_text()
        pass_count += 1
    except e:
        print("FAIL test_ctx_setup_view_auto_dyn_text:", e)
        fail_count += 1

    try:
        test_ctx_setup_view_with_render_builder()
        pass_count += 1
    except e:
        print("FAIL test_ctx_setup_view_with_render_builder:", e)
        fail_count += 1

    try:
        test_ctx_flush_convenience()
        pass_count += 1
    except e:
        print("FAIL test_ctx_flush_convenience:", e)
        fail_count += 1

    try:
        test_ctx_dioxus_style_counter_lifecycle()
        pass_count += 1
    except e:
        print("FAIL test_ctx_dioxus_style_counter_lifecycle:", e)
        fail_count += 1

    try:
        test_ctx_vnode_builder_keyed()
        pass_count += 1
    except e:
        print("FAIL test_ctx_vnode_builder_keyed:", e)
        fail_count += 1

    try:
        test_ctx_vnode_builder_for()
        pass_count += 1
    except e:
        print("FAIL test_ctx_vnode_builder_for:", e)
        fail_count += 1

    # -- SignalBool --
    try:
        test_signal_bool_get_false()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_get_false:", e)
        fail_count += 1

    try:
        test_signal_bool_get_true()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_get_true:", e)
        fail_count += 1

    try:
        test_signal_bool_set_true()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_set_true:", e)
        fail_count += 1

    try:
        test_signal_bool_set_false()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_set_false:", e)
        fail_count += 1

    try:
        test_signal_bool_toggle_from_false()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_toggle_from_false:", e)
        fail_count += 1

    try:
        test_signal_bool_toggle_from_true()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_toggle_from_true:", e)
        fail_count += 1

    try:
        test_signal_bool_toggle_round_trip()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_toggle_round_trip:", e)
        fail_count += 1

    try:
        test_signal_bool_read_subscribes()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_read_subscribes:", e)
        fail_count += 1

    try:
        test_signal_bool_peek_i32()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_peek_i32:", e)
        fail_count += 1

    try:
        test_signal_bool_version_increments()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_version_increments:", e)
        fail_count += 1

    try:
        test_signal_bool_str_true()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_str_true:", e)
        fail_count += 1

    try:
        test_signal_bool_str_false()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_str_false:", e)
        fail_count += 1

    try:
        test_signal_bool_copy()
        pass_count += 1
    except e:
        print("FAIL test_signal_bool_copy:", e)
        fail_count += 1

    # -- Conditional helpers --
    try:
        test_class_if_true()
        pass_count += 1
    except e:
        print("FAIL test_class_if_true:", e)
        fail_count += 1

    try:
        test_class_if_false()
        pass_count += 1
    except e:
        print("FAIL test_class_if_false:", e)
        fail_count += 1

    try:
        test_class_when_true()
        pass_count += 1
    except e:
        print("FAIL test_class_when_true:", e)
        fail_count += 1

    try:
        test_class_when_false()
        pass_count += 1
    except e:
        print("FAIL test_class_when_false:", e)
        fail_count += 1

    try:
        test_text_when_true()
        pass_count += 1
    except e:
        print("FAIL test_text_when_true:", e)
        fail_count += 1

    try:
        test_text_when_false()
        pass_count += 1
    except e:
        print("FAIL test_text_when_false:", e)
        fail_count += 1

    try:
        test_class_if_with_empty_name()
        pass_count += 1
    except e:
        print("FAIL test_class_if_with_empty_name:", e)
        fail_count += 1

    try:
        test_text_when_with_expression()
        pass_count += 1
    except e:
        print("FAIL test_text_when_with_expression:", e)
        fail_count += 1

    # -- ComponentContext SignalBool --
    try:
        test_ctx_use_signal_bool_true()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_bool_true:", e)
        fail_count += 1

    try:
        test_ctx_use_signal_bool_false()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_bool_false:", e)
        fail_count += 1

    try:
        test_ctx_use_signal_bool_subscribes_scope()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_bool_subscribes_scope:", e)
        fail_count += 1

    try:
        test_ctx_create_signal_bool_true()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_signal_bool_true:", e)
        fail_count += 1

    try:
        test_ctx_create_signal_bool_false()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_signal_bool_false:", e)
        fail_count += 1

    try:
        test_ctx_signal_bool_toggle_lifecycle()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_bool_toggle_lifecycle:", e)
        fail_count += 1

    # -- StringStore --
    try:
        test_string_store_create_and_read()
        pass_count += 1
    except e:
        print("FAIL test_string_store_create_and_read:", e)
        fail_count += 1

    try:
        test_string_store_write()
        pass_count += 1
    except e:
        print("FAIL test_string_store_write:", e)
        fail_count += 1

    try:
        test_string_store_count()
        pass_count += 1
    except e:
        print("FAIL test_string_store_count:", e)
        fail_count += 1

    try:
        test_string_store_contains()
        pass_count += 1
    except e:
        print("FAIL test_string_store_contains:", e)
        fail_count += 1

    try:
        test_string_store_destroy()
        pass_count += 1
    except e:
        print("FAIL test_string_store_destroy:", e)
        fail_count += 1

    try:
        test_string_store_reuse_slot()
        pass_count += 1
    except e:
        print("FAIL test_string_store_reuse_slot:", e)
        fail_count += 1

    try:
        test_string_store_multiple_entries()
        pass_count += 1
    except e:
        print("FAIL test_string_store_multiple_entries:", e)
        fail_count += 1

    try:
        test_string_store_empty_string()
        pass_count += 1
    except e:
        print("FAIL test_string_store_empty_string:", e)
        fail_count += 1

    try:
        test_string_store_overwrite_multiple_times()
        pass_count += 1
    except e:
        print("FAIL test_string_store_overwrite_multiple_times:", e)
        fail_count += 1

    # -- SignalString --
    try:
        test_signal_string_get()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_get:", e)
        fail_count += 1

    try:
        test_signal_string_peek()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_peek:", e)
        fail_count += 1

    try:
        test_signal_string_set()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_set:", e)
        fail_count += 1

    try:
        test_signal_string_set_empty()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_set_empty:", e)
        fail_count += 1

    try:
        test_signal_string_read_subscribes()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_read_subscribes:", e)
        fail_count += 1

    try:
        test_signal_string_read_returns_value()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_read_returns_value:", e)
        fail_count += 1

    try:
        test_signal_string_version_increments()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_version_increments:", e)
        fail_count += 1

    try:
        test_signal_string_is_empty_true()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_is_empty_true:", e)
        fail_count += 1

    try:
        test_signal_string_is_empty_false()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_is_empty_false:", e)
        fail_count += 1

    try:
        test_signal_string_is_empty_after_set()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_is_empty_after_set:", e)
        fail_count += 1

    try:
        test_signal_string_str()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_str:", e)
        fail_count += 1

    try:
        test_signal_string_str_empty()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_str_empty:", e)
        fail_count += 1

    try:
        test_signal_string_copy()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_copy:", e)
        fail_count += 1

    try:
        test_signal_string_multiple_writes()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_multiple_writes:", e)
        fail_count += 1

    try:
        test_signal_string_concatenation_pattern()
        pass_count += 1
    except e:
        print("FAIL test_signal_string_concatenation_pattern:", e)
        fail_count += 1

    # -- Runtime string signal --
    try:
        test_runtime_string_signal_count()
        pass_count += 1
    except e:
        print("FAIL test_runtime_string_signal_count:", e)
        fail_count += 1

    try:
        test_runtime_destroy_signal_string()
        pass_count += 1
    except e:
        print("FAIL test_runtime_destroy_signal_string:", e)
        fail_count += 1

    try:
        test_runtime_use_signal_string()
        pass_count += 1
    except e:
        print("FAIL test_runtime_use_signal_string:", e)
        fail_count += 1

    # -- ComponentContext SignalString --
    try:
        test_ctx_use_signal_string()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_string:", e)
        fail_count += 1

    try:
        test_ctx_use_signal_string_empty()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_string_empty:", e)
        fail_count += 1

    try:
        test_ctx_use_signal_string_subscribes_scope()
        pass_count += 1
    except e:
        print("FAIL test_ctx_use_signal_string_subscribes_scope:", e)
        fail_count += 1

    try:
        test_ctx_create_signal_string()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_signal_string:", e)
        fail_count += 1

    try:
        test_ctx_create_signal_string_no_subscribe()
        pass_count += 1
    except e:
        print("FAIL test_ctx_create_signal_string_no_subscribe:", e)
        fail_count += 1

    try:
        test_ctx_signal_string_set_and_get()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_string_set_and_get:", e)
        fail_count += 1

    try:
        test_ctx_signal_string_version_lifecycle()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_string_version_lifecycle:", e)
        fail_count += 1

    try:
        test_ctx_signal_string_str_interpolation()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_string_str_interpolation:", e)
        fail_count += 1

    try:
        test_ctx_signal_string_with_render_builder()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_string_with_render_builder:", e)
        fail_count += 1

    try:
        test_ctx_multiple_signal_strings()
        pass_count += 1
    except e:
        print("FAIL test_ctx_multiple_signal_strings:", e)
        fail_count += 1

    try:
        test_ctx_signal_string_with_signal_i32()
        pass_count += 1
    except e:
        print("FAIL test_ctx_signal_string_with_signal_i32:", e)
        fail_count += 1

    var total = pass_count + fail_count
    print(
        "handles:",
        String(pass_count) + "/" + String(total),
        "passed",
    )
    if fail_count > 0:
        raise Error(String(fail_count) + " tests failed")
