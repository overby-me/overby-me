from bridge import MutationWriter
from arena import ElementId, ElementIdAllocator
from signals import Runtime, create_runtime, destroy_runtime
from mutations import CreateEngine, DiffEngine
from events import HandlerEntry
from vdom import (
    TemplateBuilder,
    create_builder,
    destroy_builder,
    VNode,
    VNodeStore,
    DynamicNode,
    DynamicAttr,
    AttributeValue,
)
from html import (
    # DSL — Ergonomic builder API (M10.5)
    Node,
    text,
    dyn_text,
    dyn_node,
    attr,
    dyn_attr,
    el_empty,
    to_template,
    VNodeBuilder,
    count_nodes,
    count_all_items,
    count_dynamic_text_slots,
    count_dynamic_node_slots,
    count_dynamic_attr_slots,
    count_static_attr_nodes,
)
from html.dsl_tests import (
    test_text_node as _dsl_test_text_node,
    test_dyn_text_node as _dsl_test_dyn_text_node,
    test_dyn_node_slot as _dsl_test_dyn_node_slot,
    test_static_attr as _dsl_test_static_attr,
    test_dyn_attr as _dsl_test_dyn_attr,
    test_empty_element as _dsl_test_empty_element,
    test_element_with_children as _dsl_test_element_with_children,
    test_element_with_attrs as _dsl_test_element_with_attrs,
    test_element_mixed as _dsl_test_element_mixed,
    test_nested_elements as _dsl_test_nested_elements,
    test_counter_template as _dsl_test_counter_template,
    test_to_template_simple as _dsl_test_to_template_simple,
    test_to_template_attrs as _dsl_test_to_template_attrs,
    test_to_template_multi_root as _dsl_test_to_template_multi_root,
    test_vnode_builder as _dsl_test_vnode_builder,
    test_vnode_builder_keyed as _dsl_test_vnode_builder_keyed,
    test_all_tag_helpers as _dsl_test_all_tag_helpers,
    test_count_utilities as _dsl_test_count_utilities,
    test_template_equivalence as _dsl_test_template_equivalence,
    # Phase 20 — M20.3: oninput_set_string / onchange_set_string tests
    test_oninput_set_string_node as _dsl_test_oninput_set_string_node,
    test_onchange_set_string_node as _dsl_test_onchange_set_string_node,
    test_oninput_in_element as _dsl_test_oninput_in_element,
    # Phase 20 — M20.4: bind_value / bind_attr tests
    test_bind_value_node as _dsl_test_bind_value_node,
    test_bind_attr_node as _dsl_test_bind_attr_node,
    test_bind_value_in_element as _dsl_test_bind_value_in_element,
    test_two_way_binding_element as _dsl_test_two_way_binding_element,
    test_bind_value_to_template as _dsl_test_bind_value_to_template,
    test_two_way_to_template as _dsl_test_two_way_to_template,
    # Phase 20 — M20.5: onclick_custom tests
    test_onclick_custom_node as _dsl_test_onclick_custom_node,
    test_onclick_custom_in_element as _dsl_test_onclick_custom_in_element,
    test_onclick_custom_with_binding as _dsl_test_onclick_custom_with_binding,
    # Phase 22: onkeydown_enter_custom tests
    test_onkeydown_enter_custom_node as _dsl_test_onkeydown_enter_custom_node,
    test_onkeydown_enter_custom_in_element as _dsl_test_onkeydown_enter_custom_in_element,
    test_onkeydown_enter_custom_with_binding as _dsl_test_onkeydown_enter_custom_with_binding,
)
from scheduler import Scheduler
from component import (
    AppShell,
    app_shell_create,
    ComponentContext,
)
from signals.handle import SignalI32 as _SignalI32

from gui_app_exports import (
    gui_app_init,
    gui_app_destroy,
    gui_app_mount,
    gui_app_handle_event,
    gui_app_handle_event_string,
    gui_app_flush,
    gui_app_has_dirty,
)
from counter import CounterApp
from todo import TodoApp
from bench import BenchmarkApp
from app import MultiViewApp
from apps.child_counter import (
    ChildCounterApp,
    _cc_init,
    _cc_destroy,
    _cc_rebuild,
    _cc_handle_event,
    _cc_flush,
)
from apps.context_test import (
    ContextTestApp,
    _cta_init,
    _cta_destroy,
)
from apps.effect_demo import (
    EffectDemoApp,
    _ed_init,
    _ed_destroy,
    _ed_rebuild,
    _ed_handle_event,
    _ed_flush,
)
from apps.effect_memo import (
    EffectMemoApp,
    _em_init,
    _em_destroy,
    _em_rebuild,
    _em_handle_event,
    _em_flush,
)
from apps.memo_form import (
    MemoFormApp,
    _mf_init,
    _mf_destroy,
    _mf_rebuild,
    _mf_handle_event,
    _mf_handle_event_string,
    _mf_flush,
)
from apps.memo_chain import (
    MemoChainApp,
    _mc_init,
    _mc_destroy,
    _mc_rebuild,
    _mc_handle_event,
    _mc_flush,
)
from apps.equality_demo import (
    EqualityDemoApp,
    _eq_init,
    _eq_destroy,
    _eq_rebuild,
    _eq_handle_event,
    _eq_flush,
)
from apps.batch_demo import (
    BatchDemoApp,
    _bd_init,
    _bd_destroy,
    _bd_rebuild,
    _bd_handle_event,
    _bd_flush,
)
from apps.child_context_test import (
    ChildContextTestApp,
    _cct_init,
    _cct_destroy,
    _cct_rebuild,
    _cct_handle_event,
    _cct_flush,
)
from apps.props_counter import (
    PropsCounterApp,
    _pc_init,
    _pc_destroy,
    _pc_rebuild,
    _pc_handle_event,
    _pc_flush,
)
from apps.theme_counter import (
    ThemeCounterApp,
    _tc_init,
    _tc_destroy,
    _tc_rebuild,
    _tc_handle_event,
    _tc_flush,
)
from apps.safe_counter import (
    SafeCounterApp,
    _sc_init,
    _sc_destroy,
    _sc_rebuild,
    _sc_handle_event,
    _sc_flush,
)
from apps.error_nest import (
    ErrorNestApp,
    _en_init,
    _en_destroy,
    _en_rebuild,
    _en_handle_event,
    _en_flush,
)
from apps.data_loader import (
    DataLoaderApp,
    _dl_init,
    _dl_destroy,
    _dl_rebuild,
    _dl_handle_event,
    _dl_resolve,
    _dl_flush,
)
from apps.suspense_nest import (
    SuspenseNestApp,
    _sn_init,
    _sn_destroy,
    _sn_rebuild,
    _sn_handle_event,
    _sn_outer_resolve,
    _sn_inner_resolve,
    _sn_flush,
)
from memory import UnsafePointer, memset_zero, alloc


# ██████████████████████████████████████████████████████████████████████████████
#
#   SECTION 1 — Shared Utilities
#
#   Pointer ↔ Int helpers, Bool conversion, MutationWriter allocation.
#   Used by all @export wrappers below.
#
# ██████████████████████████████████████████████████████████████████████████████


# ══════════════════════════════════════════════════════════════════════════════
# Pointer ↔ Int helpers
# ══════════════════════════════════════════════════════════════════════════════
#
# Mojo 0.25 does not support UnsafePointer construction from an integer
# address directly.  We reinterpret the bits via a temporary heap slot.
#
# Generic helpers — one function replaces all type-specific variants.


@always_inline
fn _as_ptr[T: AnyType](addr: Int) -> UnsafePointer[T, MutExternalOrigin]:
    """Reinterpret an integer address as an UnsafePointer[T, MutExternalOrigin].
    """
    var slot = alloc[Int](1)
    slot[0] = addr
    var result = slot.bitcast[UnsafePointer[T, MutExternalOrigin]]()[0]
    slot.free()
    return result


@always_inline
fn _to_i64[T: AnyType](ptr: UnsafePointer[T, MutExternalOrigin]) -> Int64:
    """Return the raw address of a typed pointer as Int64."""
    return Int64(Int(ptr))


# ── Helper: generic heap alloc/free ─────────────────────────────────────────


@always_inline
fn _heap_new[T: Movable](var val: T) -> UnsafePointer[T, MutExternalOrigin]:
    """Allocate a single T on the heap and move val into it."""
    var ptr = alloc[T](1)
    ptr.init_pointee_move(val^)
    return ptr


@always_inline
fn _heap_del[
    T: Movable & ImplicitlyDestructible
](ptr: UnsafePointer[T, MutExternalOrigin]):
    """Destroy and free a single heap-allocated T."""
    ptr.destroy_pointee()
    ptr.free()


# ── Helper: generic get-pointer wrapper ──────────────────────────────────────


@always_inline
fn _get[T: AnyType](ptr: Int64) -> UnsafePointer[T, MutExternalOrigin]:
    """Reinterpret an Int64 WASM handle as an UnsafePointer[T, MutExternalOrigin].
    """
    return _as_ptr[T](Int(ptr))


# ── Helper: Bool → Int32 for WASM ABI ───────────────────────────────────────


@always_inline
fn _b2i(val: Bool) -> Int32:
    """Convert a Bool to Int32 (1 or 0) for WASM export returns."""
    if val:
        return 1
    return 0


# ── Helper: writer at (buf, off) ────────────────────────────────────────────


@always_inline
fn _writer(buf: Int64, off: Int32) -> MutationWriter:
    return MutationWriter(_get[UInt8](buf), Int(off), 0)


# ── Helper: heap-allocated MutationWriter for app rebuild/flush ──────────────


@always_inline
fn _alloc_writer(
    buf_ptr: Int64, capacity: Int32
) -> UnsafePointer[MutationWriter, MutExternalOrigin]:
    """Allocate a MutationWriter on the heap with the given buffer and capacity.
    """
    return _heap_new(MutationWriter(_get[UInt8](buf_ptr), Int(capacity)))


@always_inline
fn _free_writer(ptr: UnsafePointer[MutationWriter, MutExternalOrigin]):
    """Destroy and free a heap-allocated MutationWriter."""
    _heap_del(ptr)


# ██████████████████████████████████████████████████████████████████████████████
#
#   SECTION 2 — Framework Test & Runtime Exports
#
#   Low-level exports for the Mojo and JS test suites exercising
#   individual framework subsystems: ElementId, Runtime/Signals, Scopes,
#   Templates, VNode, Create/Diff, Mutations, Events, Context, Error
#   Boundaries, Suspense, Memo (I32/Bool/String), Equality, Batch,
#   Effects, Scheduler, AppShell, DSL, and PoC arithmetic/string exports.
#
# ██████████████████████████████████████████████████████████████████████████████


# ══════════════════════════════════════════════════════════════════════════════
# ElementId Allocator Test Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn eid_alloc_create() -> Int64:
    """Allocate an ElementIdAllocator on the heap."""
    return _to_i64(_heap_new(ElementIdAllocator()))


@export
fn eid_alloc_destroy(alloc_ptr: Int64):
    """Destroy and free a heap-allocated ElementIdAllocator."""
    _heap_del(_get[ElementIdAllocator](alloc_ptr))


@export
fn eid_alloc(alloc_ptr: Int64) -> Int32:
    """Allocate a new ElementId.  Returns the raw u32 as i32."""
    return Int32(_get[ElementIdAllocator](alloc_ptr)[0].alloc().as_u32())


@export
fn eid_free(alloc_ptr: Int64, id: Int32):
    """Free an ElementId."""
    _get[ElementIdAllocator](alloc_ptr)[0].free(ElementId(UInt32(id)))


@export
fn eid_is_alive(alloc_ptr: Int64, id: Int32) -> Int32:
    """Check whether an ElementId is currently allocated.  Returns 1 or 0."""
    return _b2i(
        _get[ElementIdAllocator](alloc_ptr)[0].is_alive(ElementId(UInt32(id)))
    )


@export
fn eid_count(alloc_ptr: Int64) -> Int32:
    """Number of allocated IDs (including root)."""
    return Int32(_get[ElementIdAllocator](alloc_ptr)[0].count())


@export
fn eid_user_count(alloc_ptr: Int64) -> Int32:
    """Number of user-allocated IDs (excluding root)."""
    return Int32(_get[ElementIdAllocator](alloc_ptr)[0].user_count())


# ══════════════════════════════════════════════════════════════════════════════
# Runtime / Signals Test Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn runtime_create() -> Int64:
    """Allocate a reactive Runtime on the heap."""
    return _to_i64(create_runtime())


@export
fn runtime_destroy(rt_ptr: Int64):
    """Destroy and free a heap-allocated Runtime."""
    destroy_runtime(_get[Runtime](rt_ptr))


@export
fn signal_create_i32(rt_ptr: Int64, initial: Int32) -> Int32:
    """Create an Int32 signal.  Returns its key."""
    return Int32(_get[Runtime](rt_ptr)[0].create_signal[Int32](initial))


@export
fn signal_read_i32(rt_ptr: Int64, key: Int32) -> Int32:
    """Read an Int32 signal (with context tracking)."""
    return _get[Runtime](rt_ptr)[0].read_signal[Int32](UInt32(key))


@export
fn signal_write_i32(rt_ptr: Int64, key: Int32, value: Int32):
    """Write a new value to an Int32 signal."""
    _get[Runtime](rt_ptr)[0].write_signal[Int32](UInt32(key), value)


@export
fn signal_peek_i32(rt_ptr: Int64, key: Int32) -> Int32:
    """Read an Int32 signal without subscribing."""
    return _get[Runtime](rt_ptr)[0].peek_signal[Int32](UInt32(key))


@export
fn signal_destroy(rt_ptr: Int64, key: Int32):
    """Destroy a signal."""
    _get[Runtime](rt_ptr)[0].destroy_signal(UInt32(key))


@export
fn signal_subscriber_count(rt_ptr: Int64, key: Int32) -> Int32:
    """Return the number of subscribers for a signal."""
    return Int32(_get[Runtime](rt_ptr)[0].signals.subscriber_count(UInt32(key)))


@export
fn signal_version(rt_ptr: Int64, key: Int32) -> Int32:
    """Return the write-version counter for a signal."""
    return Int32(_get[Runtime](rt_ptr)[0].signals.version(UInt32(key)))


@export
fn signal_count(rt_ptr: Int64) -> Int32:
    """Return the number of live signals."""
    return Int32(_get[Runtime](rt_ptr)[0].signals.signal_count())


@export
fn signal_contains(rt_ptr: Int64, key: Int32) -> Int32:
    """Check whether a signal key is live.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].signals.contains(UInt32(key)))


# ── String signal exports (Phase 20) ────────────────────────────────────────


@export
fn signal_create_string(rt_ptr: Int64, initial: String) -> Int64:
    """Create a string signal.  Returns (string_key, version_key) packed as i64.

    Phase 20: The two UInt32 keys are packed into a single Int64:
      - low 32 bits  = string_key  (StringStore index)
      - high 32 bits = version_key (SignalStore index)

    Use signal_string_key() and signal_version_key() to unpack.
    """
    var keys = _get[Runtime](rt_ptr)[0].create_signal_string(initial)
    # Pack two UInt32 keys into one Int64: low = string_key, high = version_key
    return Int64(Int(keys[0])) | (Int64(Int(keys[1])) << 32)


@export
fn signal_string_key(packed: Int64) -> Int32:
    """Extract the string_key (low 32 bits) from a packed string signal pair."""
    return Int32(packed & 0xFFFFFFFF)


@export
fn signal_version_key(packed: Int64) -> Int32:
    """Extract the version_key (high 32 bits) from a packed string signal pair.
    """
    return Int32((packed >> 32) & 0xFFFFFFFF)


@export
fn signal_peek_string(rt_ptr: Int64, string_key: Int32) -> String:
    """Read a string signal without subscribing.  Returns the current value."""
    return _get[Runtime](rt_ptr)[0].peek_signal_string(UInt32(string_key))


@export
fn signal_write_string(
    rt_ptr: Int64, string_key: Int32, version_key: Int32, value: String
):
    """Write a new value to a string signal (bumps version, notifies subscribers).
    """
    _get[Runtime](rt_ptr)[0].write_signal_string(
        UInt32(string_key), UInt32(version_key), value
    )


@export
fn signal_string_count(rt_ptr: Int64) -> Int32:
    """Return the number of live string signals."""
    return Int32(_get[Runtime](rt_ptr)[0].string_signal_count())


# ── Context management exports ───────────────────────────────────────────────


@export
fn runtime_set_context(rt_ptr: Int64, context_id: Int32):
    """Set the current reactive context."""
    _get[Runtime](rt_ptr)[0].set_context(UInt32(context_id))


@export
fn runtime_clear_context(rt_ptr: Int64):
    """Clear the current reactive context."""
    _get[Runtime](rt_ptr)[0].clear_context()


@export
fn runtime_has_context(rt_ptr: Int64) -> Int32:
    """Check if a reactive context is active.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].has_context())


@export
fn runtime_dirty_count(rt_ptr: Int64) -> Int32:
    """Return the number of dirty scopes."""
    return Int32(_get[Runtime](rt_ptr)[0].dirty_count())


@export
fn runtime_has_dirty(rt_ptr: Int64) -> Int32:
    """Check if any scopes are dirty.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].has_dirty())


# ══════════════════════════════════════════════════════════════════════════════
# Scope Management Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn scope_create(rt_ptr: Int64, height: Int32, parent_id: Int32) -> Int32:
    """Create a new scope.  Returns its ID."""
    return Int32(
        _get[Runtime](rt_ptr)[0].create_scope(UInt32(height), Int(parent_id))
    )


@export
fn scope_create_child(rt_ptr: Int64, parent_id: Int32) -> Int32:
    """Create a child scope.  Height is parent.height + 1."""
    return Int32(_get[Runtime](rt_ptr)[0].create_child_scope(UInt32(parent_id)))


@export
fn scope_destroy(rt_ptr: Int64, id: Int32):
    """Destroy a scope."""
    _get[Runtime](rt_ptr)[0].destroy_scope(UInt32(id))


@export
fn scope_count(rt_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(_get[Runtime](rt_ptr)[0].scope_count())


@export
fn scope_contains(rt_ptr: Int64, id: Int32) -> Int32:
    """Check whether a scope ID is live.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].scope_contains(UInt32(id)))


@export
fn scope_height(rt_ptr: Int64, id: Int32) -> Int32:
    """Return the height (depth) of a scope."""
    return Int32(_get[Runtime](rt_ptr)[0].scopes.height(UInt32(id)))


@export
fn scope_parent(rt_ptr: Int64, id: Int32) -> Int32:
    """Return the parent ID of a scope, or -1 if root."""
    return Int32(_get[Runtime](rt_ptr)[0].scopes.parent_id(UInt32(id)))


@export
fn scope_is_dirty(rt_ptr: Int64, id: Int32) -> Int32:
    """Check whether a scope is dirty.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].scopes.is_dirty(UInt32(id)))


@export
fn scope_set_dirty(rt_ptr: Int64, id: Int32, dirty: Int32):
    """Set the dirty flag on a scope."""
    _get[Runtime](rt_ptr)[0].scopes.set_dirty(UInt32(id), dirty != 0)


@export
fn scope_render_count(rt_ptr: Int64, id: Int32) -> Int32:
    """Return how many times a scope has been rendered."""
    return Int32(_get[Runtime](rt_ptr)[0].scopes.render_count(UInt32(id)))


@export
fn scope_hook_count(rt_ptr: Int64, id: Int32) -> Int32:
    """Return the number of hooks in a scope."""
    return Int32(_get[Runtime](rt_ptr)[0].scopes.hook_count(UInt32(id)))


@export
fn scope_hook_value_at(rt_ptr: Int64, id: Int32, index: Int32) -> Int32:
    """Return the hook value (signal key) at position `index` in scope `id`."""
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.hook_value_at(UInt32(id), Int(index))
    )


@export
fn scope_hook_tag_at(rt_ptr: Int64, id: Int32, index: Int32) -> Int32:
    """Return the hook tag at position `index` in scope `id`."""
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.hook_tag_at(UInt32(id), Int(index))
    )


# ── Scope rendering lifecycle exports ────────────────────────────────────────


@export
fn scope_begin_render(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Begin rendering a scope.  Returns the previous scope ID (or -1)."""
    return Int32(_get[Runtime](rt_ptr)[0].begin_scope_render(UInt32(scope_id)))


@export
fn scope_end_render(rt_ptr: Int64, prev_scope: Int32):
    """End rendering the current scope and restore the previous scope."""
    _get[Runtime](rt_ptr)[0].end_scope_render(Int(prev_scope))


@export
fn scope_has_scope(rt_ptr: Int64) -> Int32:
    """Check if a scope is currently active.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].has_scope())


@export
fn scope_get_current(rt_ptr: Int64) -> Int32:
    """Return the current scope ID, or -1 if none."""
    var rt = _get[Runtime](rt_ptr)
    if rt[0].has_scope():
        return Int32(rt[0].get_scope())
    return -1


# ── Hook exports ─────────────────────────────────────────────────────────────


@export
fn hook_use_signal_i32(rt_ptr: Int64, initial: Int32) -> Int32:
    """Hook: create or retrieve an Int32 signal for the current scope.

    On first render: creates signal, stores in hooks, returns key.
    On re-render: returns existing signal key (initial ignored).
    """
    return Int32(_get[Runtime](rt_ptr)[0].use_signal_i32(initial))


@export
fn hook_use_memo_i32(rt_ptr: Int64, initial: Int32) -> Int32:
    """Hook: create or retrieve an Int32 memo for the current scope.

    On first render: creates memo, stores in hooks (HOOK_MEMO tag), returns ID.
    On re-render: returns existing memo ID (initial ignored).
    """
    return Int32(_get[Runtime](rt_ptr)[0].use_memo_i32(initial))


@export
fn hook_use_effect(rt_ptr: Int64) -> Int32:
    """Hook: create or retrieve an effect for the current scope.

    On first render: creates effect, stores in hooks (HOOK_EFFECT tag), returns ID.
    On re-render: returns existing effect ID.
    """
    return Int32(_get[Runtime](rt_ptr)[0].use_effect())


@export
fn scope_is_first_render(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Check if a scope is on its first render.  Returns 1 or 0."""
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.is_first_render(UInt32(scope_id))
    )


# ══════════════════════════════════════════════════════════════════════════════
# Template Builder Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn tmpl_builder_create(name: String) -> Int64:
    """Create a heap-allocated TemplateBuilder.  Returns its pointer."""
    return _to_i64(create_builder(name))


@export
fn tmpl_builder_destroy(ptr: Int64):
    """Destroy and free a heap-allocated TemplateBuilder."""
    destroy_builder(_get[TemplateBuilder](ptr))


@export
fn tmpl_builder_push_element(
    ptr: Int64, html_tag: Int32, parent: Int32
) -> Int32:
    """Add an Element node.  parent=-1 means root.  Returns node index."""
    return Int32(
        _get[TemplateBuilder](ptr)[0].push_element(UInt8(html_tag), Int(parent))
    )


@export
fn tmpl_builder_push_text(ptr: Int64, text: String, parent: Int32) -> Int32:
    """Add a static Text node.  Returns node index."""
    return Int32(_get[TemplateBuilder](ptr)[0].push_text(text, Int(parent)))


@export
fn tmpl_builder_push_dynamic(
    ptr: Int64, dynamic_index: Int32, parent: Int32
) -> Int32:
    """Add a Dynamic node placeholder.  Returns node index."""
    return Int32(
        _get[TemplateBuilder](ptr)[0].push_dynamic(
            UInt32(dynamic_index), Int(parent)
        )
    )


@export
fn tmpl_builder_push_dynamic_text(
    ptr: Int64, dynamic_index: Int32, parent: Int32
) -> Int32:
    """Add a DynamicText node placeholder.  Returns node index."""
    return Int32(
        _get[TemplateBuilder](ptr)[0].push_dynamic_text(
            UInt32(dynamic_index), Int(parent)
        )
    )


@export
fn tmpl_builder_push_static_attr(
    ptr: Int64, node_index: Int32, name: String, value: String
):
    """Add a static attribute to the specified node."""
    _get[TemplateBuilder](ptr)[0].push_static_attr(Int(node_index), name, value)


@export
fn tmpl_builder_push_dynamic_attr(
    ptr: Int64, node_index: Int32, dynamic_index: Int32
):
    """Add a dynamic attribute placeholder to the specified node."""
    _get[TemplateBuilder](ptr)[0].push_dynamic_attr(
        Int(node_index), UInt32(dynamic_index)
    )


@export
fn tmpl_builder_node_count(ptr: Int64) -> Int32:
    """Return the number of nodes in the builder."""
    return Int32(_get[TemplateBuilder](ptr)[0].node_count())


@export
fn tmpl_builder_root_count(ptr: Int64) -> Int32:
    """Return the number of root nodes in the builder."""
    return Int32(_get[TemplateBuilder](ptr)[0].root_count())


@export
fn tmpl_builder_attr_count(ptr: Int64) -> Int32:
    """Return the total number of attributes in the builder."""
    return Int32(_get[TemplateBuilder](ptr)[0].attr_count())


@export
fn tmpl_builder_register(rt_ptr: Int64, builder_ptr: Int64) -> Int32:
    """Build the template and register it in the runtime.  Returns template ID.
    """
    var template = _get[TemplateBuilder](builder_ptr)[0].build()
    return Int32(_get[Runtime](rt_ptr)[0].templates.register(template^))


# ── Template Registry Query Exports ──────────────────────────────────────────


@export
fn tmpl_count(rt_ptr: Int64) -> Int32:
    """Return the number of registered templates."""
    return Int32(_get[Runtime](rt_ptr)[0].templates.count())


@export
fn tmpl_root_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the number of root nodes in the template."""
    return Int32(_get[Runtime](rt_ptr)[0].templates.root_count(UInt32(tmpl_id)))


@export
fn tmpl_node_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the total number of nodes in the template."""
    return Int32(_get[Runtime](rt_ptr)[0].templates.node_count(UInt32(tmpl_id)))


@export
fn tmpl_node_kind(rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32) -> Int32:
    """Return the kind tag (TNODE_*) of the node at node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_kind(
            UInt32(tmpl_id), Int(node_idx)
        )
    )


@export
fn tmpl_node_tag(rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32) -> Int32:
    """Return the HTML tag constant of the Element node at node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_html_tag(
            UInt32(tmpl_id), Int(node_idx)
        )
    )


@export
fn tmpl_node_child_count(
    rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32
) -> Int32:
    """Return the number of children of the node at node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_child_count(
            UInt32(tmpl_id), Int(node_idx)
        )
    )


@export
fn tmpl_node_child_at(
    rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32, child_pos: Int32
) -> Int32:
    """Return the node index of the child at child_pos within node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_child_at(
            UInt32(tmpl_id), Int(node_idx), Int(child_pos)
        )
    )


@export
fn tmpl_node_dynamic_index(
    rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32
) -> Int32:
    """Return the dynamic slot index of the node at node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_dynamic_index(
            UInt32(tmpl_id), Int(node_idx)
        )
    )


@export
fn tmpl_node_attr_count(
    rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32
) -> Int32:
    """Return the number of attributes on the node at node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_attr_count(
            UInt32(tmpl_id), Int(node_idx)
        )
    )


@export
fn tmpl_attr_total_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the total number of attributes in the template."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.attr_total_count(UInt32(tmpl_id))
    )


@export
fn tmpl_get_root_index(rt_ptr: Int64, tmpl_id: Int32, root_pos: Int32) -> Int32:
    """Return the node index of the root at position root_pos."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.get_root_index(
            UInt32(tmpl_id), Int(root_pos)
        )
    )


@export
fn tmpl_attr_kind(rt_ptr: Int64, tmpl_id: Int32, attr_idx: Int32) -> Int32:
    """Return the kind (TATTR_*) of the attribute at attr_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.get_attr_kind(
            UInt32(tmpl_id), Int(attr_idx)
        )
    )


@export
fn tmpl_attr_dynamic_index(
    rt_ptr: Int64, tmpl_id: Int32, attr_idx: Int32
) -> Int32:
    """Return the dynamic index of the attribute at attr_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.get_attr_dynamic_index(
            UInt32(tmpl_id), Int(attr_idx)
        )
    )


@export
fn tmpl_dynamic_node_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the number of Dynamic node slots in the template."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.dynamic_node_count(UInt32(tmpl_id))
    )


@export
fn tmpl_dynamic_text_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the number of DynamicText slots in the template."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.dynamic_text_count(UInt32(tmpl_id))
    )


@export
fn tmpl_dynamic_attr_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the number of dynamic attribute slots in the template."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.dynamic_attr_count(UInt32(tmpl_id))
    )


@export
fn tmpl_static_attr_count(rt_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Return the number of static attributes in the template."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.static_attr_count(UInt32(tmpl_id))
    )


@export
fn tmpl_contains_name(rt_ptr: Int64, name: String) -> Int32:
    """Check if a template with the given name is registered.  Returns 1 or 0.
    """
    return _b2i(_get[Runtime](rt_ptr)[0].templates.contains_name(name))


@export
fn tmpl_find_by_name(rt_ptr: Int64, name: String) -> Int32:
    """Find a template by name.  Returns ID or -1 if not found."""
    return Int32(_get[Runtime](rt_ptr)[0].templates.find_by_name(name))


@export
fn tmpl_node_first_attr(
    rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32
) -> Int32:
    """Return the first attribute index of the node at node_idx."""
    return Int32(
        _get[Runtime](rt_ptr)[0].templates.node_first_attr(
            UInt32(tmpl_id), Int(node_idx)
        )
    )


@export
fn tmpl_node_text(rt_ptr: Int64, tmpl_id: Int32, node_idx: Int32) -> String:
    """Return the static text content of a Text node in the template."""
    return (
        _get[Runtime](rt_ptr)[0]
        .templates.get_ptr(UInt32(tmpl_id))[0]
        .get_node_ptr(Int(node_idx))[0]
        .text
    )


@export
fn tmpl_attr_name(rt_ptr: Int64, tmpl_id: Int32, attr_idx: Int32) -> String:
    """Return the name of the attribute at attr_idx in the template."""
    return (
        _get[Runtime](rt_ptr)[0]
        .templates.get_ptr(UInt32(tmpl_id))[0]
        .attrs[Int(attr_idx)]
        .name
    )


@export
fn tmpl_attr_value(rt_ptr: Int64, tmpl_id: Int32, attr_idx: Int32) -> String:
    """Return the value of the attribute at attr_idx in the template."""
    return (
        _get[Runtime](rt_ptr)[0]
        .templates.get_ptr(UInt32(tmpl_id))[0]
        .attrs[Int(attr_idx)]
        .value
    )


# ══════════════════════════════════════════════════════════════════════════════
# VNode Store Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn vnode_store_create() -> Int64:
    """Allocate a standalone VNodeStore on the heap.  Returns its pointer."""
    return _to_i64(_heap_new(VNodeStore()))


@export
fn vnode_store_destroy(store_ptr: Int64):
    """Destroy and free a heap-allocated VNodeStore."""
    _heap_del(_get[VNodeStore](store_ptr))


@export
fn vnode_push_template_ref(store_ptr: Int64, tmpl_id: Int32) -> Int32:
    """Create a TemplateRef VNode and push it into the store.  Returns index."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].push(VNode.template_ref(UInt32(tmpl_id)))
    )


@export
fn vnode_push_template_ref_keyed(
    store_ptr: Int64, tmpl_id: Int32, key: String
) -> Int32:
    """Create a keyed TemplateRef VNode.  Returns index."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].push(
            VNode.template_ref_keyed(UInt32(tmpl_id), key)
        )
    )


@export
fn vnode_push_text(store_ptr: Int64, text: String) -> Int32:
    """Create a Text VNode and push it into the store.  Returns index."""
    return Int32(_get[VNodeStore](store_ptr)[0].push(VNode.text_node(text)))


@export
fn vnode_push_placeholder(store_ptr: Int64, element_id: Int32) -> Int32:
    """Create a Placeholder VNode.  Returns index."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].push(
            VNode.placeholder(UInt32(element_id))
        )
    )


@export
fn vnode_push_fragment(store_ptr: Int64) -> Int32:
    """Create an empty Fragment VNode.  Returns index."""
    return Int32(_get[VNodeStore](store_ptr)[0].push(VNode.fragment()))


@export
fn vnode_count(store_ptr: Int64) -> Int32:
    """Return the number of VNodes in the store."""
    return Int32(_get[VNodeStore](store_ptr)[0].count())


@export
fn vnode_kind(store_ptr: Int64, index: Int32) -> Int32:
    """Return the kind tag (VNODE_*) of the VNode at index."""
    return Int32(_get[VNodeStore](store_ptr)[0].kind(UInt32(index)))


@export
fn vnode_template_id(store_ptr: Int64, index: Int32) -> Int32:
    """Return the template_id of the TemplateRef VNode at index."""
    return Int32(_get[VNodeStore](store_ptr)[0].template_id(UInt32(index)))


@export
fn vnode_element_id(store_ptr: Int64, index: Int32) -> Int32:
    """Return the element_id of the Placeholder VNode at index."""
    return Int32(_get[VNodeStore](store_ptr)[0].element_id(UInt32(index)))


@export
fn vnode_has_key(store_ptr: Int64, index: Int32) -> Int32:
    """Check if the VNode has a key.  Returns 1 or 0."""
    return _b2i(_get[VNodeStore](store_ptr)[0].has_key(UInt32(index)))


@export
fn vnode_dynamic_node_count(store_ptr: Int64, index: Int32) -> Int32:
    """Return the number of dynamic nodes on the VNode."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].dynamic_node_count(UInt32(index))
    )


@export
fn vnode_dynamic_attr_count(store_ptr: Int64, index: Int32) -> Int32:
    """Return the number of dynamic attributes on the VNode."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].dynamic_attr_count(UInt32(index))
    )


@export
fn vnode_fragment_child_count(store_ptr: Int64, index: Int32) -> Int32:
    """Return the number of fragment children on the VNode."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].fragment_child_count(UInt32(index))
    )


@export
fn vnode_fragment_child_at(
    store_ptr: Int64, index: Int32, child_pos: Int32
) -> Int32:
    """Return the VNode index of the fragment child at child_pos."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].get_fragment_child(
            UInt32(index), Int(child_pos)
        )
    )


@export
fn vnode_push_dynamic_text_node(
    store_ptr: Int64, vnode_index: Int32, text: String
):
    """Append a dynamic text node to the VNode at vnode_index."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_node(
        UInt32(vnode_index), DynamicNode.text_node(text)
    )


@export
fn vnode_push_dynamic_placeholder(store_ptr: Int64, vnode_index: Int32):
    """Append a dynamic placeholder node to the VNode at vnode_index."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_node(
        UInt32(vnode_index), DynamicNode.placeholder()
    )


@export
fn vnode_push_dynamic_attr_text(
    store_ptr: Int64,
    vnode_index: Int32,
    name: String,
    value: String,
    elem_id: Int32,
):
    """Append a dynamic text attribute to the VNode."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_attr(
        UInt32(vnode_index),
        DynamicAttr(name, AttributeValue.text(value), UInt32(elem_id)),
    )


@export
fn vnode_push_dynamic_attr_int(
    store_ptr: Int64,
    vnode_index: Int32,
    name: String,
    value: Int32,
    elem_id: Int32,
):
    """Append a dynamic integer attribute to the VNode."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_attr(
        UInt32(vnode_index),
        DynamicAttr(
            name, AttributeValue.integer(Int64(value)), UInt32(elem_id)
        ),
    )


@export
fn vnode_push_dynamic_attr_bool(
    store_ptr: Int64,
    vnode_index: Int32,
    name: String,
    value: Int32,
    elem_id: Int32,
):
    """Append a dynamic boolean attribute to the VNode."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_attr(
        UInt32(vnode_index),
        DynamicAttr(name, AttributeValue.boolean(value != 0), UInt32(elem_id)),
    )


@export
fn vnode_push_dynamic_attr_event(
    store_ptr: Int64,
    vnode_index: Int32,
    name: String,
    handler_id: Int32,
    elem_id: Int32,
):
    """Append a dynamic event handler attribute to the VNode."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_attr(
        UInt32(vnode_index),
        DynamicAttr(
            name, AttributeValue.event(UInt32(handler_id)), UInt32(elem_id)
        ),
    )


@export
fn vnode_push_dynamic_attr_none(
    store_ptr: Int64, vnode_index: Int32, name: String, elem_id: Int32
):
    """Append a dynamic none attribute (removal) to the VNode."""
    _get[VNodeStore](store_ptr)[0].push_dynamic_attr(
        UInt32(vnode_index),
        DynamicAttr(name, AttributeValue.none(), UInt32(elem_id)),
    )


@export
fn vnode_push_fragment_child(
    store_ptr: Int64, vnode_index: Int32, child_index: Int32
):
    """Append a child VNode index to the Fragment at vnode_index."""
    _get[VNodeStore](store_ptr)[0].push_fragment_child(
        UInt32(vnode_index), UInt32(child_index)
    )


@export
fn vnode_get_dynamic_node_kind(
    store_ptr: Int64, vnode_index: Int32, dyn_index: Int32
) -> Int32:
    """Return the kind of the dynamic node at dyn_index."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].get_dynamic_node_kind(
            UInt32(vnode_index), Int(dyn_index)
        )
    )


@export
fn vnode_get_dynamic_attr_kind(
    store_ptr: Int64, vnode_index: Int32, attr_index: Int32
) -> Int32:
    """Return the attribute value kind of the dynamic attr at attr_index."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].get_dynamic_attr_kind(
            UInt32(vnode_index), Int(attr_index)
        )
    )


@export
fn vnode_get_dynamic_attr_element_id(
    store_ptr: Int64, vnode_index: Int32, attr_index: Int32
) -> Int32:
    """Return the element_id of the dynamic attr at attr_index."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].get_dynamic_attr_element_id(
            UInt32(vnode_index), Int(attr_index)
        )
    )


@export
fn vnode_store_clear(store_ptr: Int64):
    """Clear all VNodes from the store."""
    _get[VNodeStore](store_ptr)[0].clear()


# ── Signal arithmetic helpers ────────────────────────────────────────────────


@export
fn signal_iadd_i32(rt_ptr: Int64, key: Int32, rhs: Int32):
    """Increment a signal: signal += rhs."""
    var rt = _get[Runtime](rt_ptr)
    var current = rt[0].peek_signal[Int32](UInt32(key))
    rt[0].write_signal[Int32](UInt32(key), current + rhs)


@export
fn signal_isub_i32(rt_ptr: Int64, key: Int32, rhs: Int32):
    """Decrement a signal: signal -= rhs."""
    var rt = _get[Runtime](rt_ptr)
    var current = rt[0].peek_signal[Int32](UInt32(key))
    rt[0].write_signal[Int32](UInt32(key), current - rhs)


# ══════════════════════════════════════════════════════════════════════════════
# Create & Diff Engine Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn writer_create(buf_ptr: Int64, capacity: Int32) -> Int64:
    """Create a heap-allocated MutationWriter.  Returns its pointer."""
    return _to_i64(_alloc_writer(buf_ptr, capacity))


@export
fn writer_destroy(writer_ptr: Int64):
    """Destroy and free a heap-allocated MutationWriter."""
    _free_writer(_get[MutationWriter](writer_ptr))


@export
fn writer_offset(writer_ptr: Int64) -> Int32:
    """Return the current write offset of the MutationWriter."""
    return Int32(_get[MutationWriter](writer_ptr)[0].offset)


@export
fn writer_finalize(writer_ptr: Int64) -> Int32:
    """Write the End sentinel and return the final offset."""
    var ptr = _get[MutationWriter](writer_ptr)
    ptr[0].finalize()
    return Int32(ptr[0].offset)


@export
fn create_vnode(
    writer_ptr: Int64,
    eid_ptr: Int64,
    rt_ptr: Int64,
    store_ptr: Int64,
    vnode_index: Int32,
) -> Int32:
    """Create mutations for the VNode at vnode_index.  Returns root count."""
    var engine = CreateEngine(
        _get[MutationWriter](writer_ptr),
        _get[ElementIdAllocator](eid_ptr),
        _get[Runtime](rt_ptr),
        _get[VNodeStore](store_ptr),
    )
    return Int32(engine.create_node(UInt32(vnode_index)))


@export
fn diff_vnodes(
    writer_ptr: Int64,
    eid_ptr: Int64,
    rt_ptr: Int64,
    store_ptr: Int64,
    old_index: Int32,
    new_index: Int32,
) -> Int32:
    """Diff old and new VNodes and emit mutations.  Returns writer offset."""
    var w = _get[MutationWriter](writer_ptr)
    var engine = DiffEngine(
        w,
        _get[ElementIdAllocator](eid_ptr),
        _get[Runtime](rt_ptr),
        _get[VNodeStore](store_ptr),
    )
    engine.diff_node(UInt32(old_index), UInt32(new_index))
    return Int32(w[0].offset)


# ── VNode mount state query exports ──────────────────────────────────────────


@export
fn vnode_root_id_count(store_ptr: Int64, index: Int32) -> Int32:
    """Return the number of root ElementIds assigned to this VNode."""
    return Int32(
        _get[VNodeStore](store_ptr)[0].get_ptr(UInt32(index))[0].root_id_count()
    )


@export
fn vnode_get_root_id(store_ptr: Int64, index: Int32, pos: Int32) -> Int32:
    """Return the root ElementId at position `pos`."""
    return Int32(
        _get[VNodeStore](store_ptr)[0]
        .get_ptr(UInt32(index))[0]
        .get_root_id(Int(pos))
    )


@export
fn vnode_dyn_node_id_count(store_ptr: Int64, index: Int32) -> Int32:
    """Return the number of dynamic node ElementIds."""
    return Int32(
        _get[VNodeStore](store_ptr)[0]
        .get_ptr(UInt32(index))[0]
        .dyn_node_id_count()
    )


@export
fn vnode_get_dyn_node_id(store_ptr: Int64, index: Int32, pos: Int32) -> Int32:
    """Return the dynamic node ElementId at position `pos`."""
    return Int32(
        _get[VNodeStore](store_ptr)[0]
        .get_ptr(UInt32(index))[0]
        .get_dyn_node_id(Int(pos))
    )


@export
fn vnode_dyn_attr_id_count(store_ptr: Int64, index: Int32) -> Int32:
    """Return the number of dynamic attribute target ElementIds."""
    return Int32(
        _get[VNodeStore](store_ptr)[0]
        .get_ptr(UInt32(index))[0]
        .dyn_attr_id_count()
    )


@export
fn vnode_get_dyn_attr_id(store_ptr: Int64, index: Int32, pos: Int32) -> Int32:
    """Return the dynamic attribute target ElementId at position `pos`."""
    return Int32(
        _get[VNodeStore](store_ptr)[0]
        .get_ptr(UInt32(index))[0]
        .get_dyn_attr_id(Int(pos))
    )


@export
fn vnode_is_mounted(store_ptr: Int64, index: Int32) -> Int32:
    """Check whether the VNode has been mounted.  Returns 1 or 0."""
    return _b2i(
        _get[VNodeStore](store_ptr)[0].get_ptr(UInt32(index))[0].is_mounted()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Mutation Protocol Test Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn mutation_buf_alloc(capacity: Int32) -> Int64:
    """Allocate a mutation buffer. Returns a pointer into WASM linear memory.

    The buffer is zero-initialized so that unwritten positions read as
    OP_END (0x00).  This is necessary because the allocator may reuse
    previously freed blocks that contain stale data.
    """
    var cap = Int(capacity)
    var ptr = alloc[UInt8](cap)
    memset_zero(ptr, cap)
    return _to_i64(ptr)


@export
fn mutation_buf_free(ptr: Int64):
    """Free a previously allocated mutation buffer."""
    _get[UInt8](ptr).free()


# ── Debug exports ────────────────────────────────────────────────────────────


@export
fn debug_ptr_roundtrip(ptr: Int64) -> Int64:
    """Check that _as_ptr round-trips correctly."""
    return _to_i64(_get[UInt8](ptr))


@export
fn debug_write_byte(ptr: Int64, off: Int32, val: Int32) -> Int32:
    """Write a single byte to ptr+off and return off+1."""
    _get[UInt8](ptr)[Int(off)] = UInt8(val)
    return off + 1


@export
fn debug_read_byte(ptr: Int64, off: Int32) -> Int32:
    """Read a single byte from ptr+off."""
    return Int32(_get[UInt8](ptr)[Int(off)])


# ── Simple opcodes ───────────────────────────────────────────────────────────


@export
fn write_op_end(buf: Int64, off: Int32) -> Int32:
    var w = _writer(buf, off)
    w.end()
    return Int32(w.offset)


@export
fn write_op_append_children(
    buf: Int64, off: Int32, id: Int32, m: Int32
) -> Int32:
    var w = _writer(buf, off)
    w.append_children(UInt32(id), UInt32(m))
    return Int32(w.offset)


@export
fn write_op_create_placeholder(buf: Int64, off: Int32, id: Int32) -> Int32:
    var w = _writer(buf, off)
    w.create_placeholder(UInt32(id))
    return Int32(w.offset)


@export
fn write_op_load_template(
    buf: Int64, off: Int32, tmpl_id: Int32, index: Int32, id: Int32
) -> Int32:
    var w = _writer(buf, off)
    w.load_template(UInt32(tmpl_id), UInt32(index), UInt32(id))
    return Int32(w.offset)


@export
fn write_op_replace_with(buf: Int64, off: Int32, id: Int32, m: Int32) -> Int32:
    var w = _writer(buf, off)
    w.replace_with(UInt32(id), UInt32(m))
    return Int32(w.offset)


@export
fn write_op_insert_after(buf: Int64, off: Int32, id: Int32, m: Int32) -> Int32:
    var w = _writer(buf, off)
    w.insert_after(UInt32(id), UInt32(m))
    return Int32(w.offset)


@export
fn write_op_insert_before(buf: Int64, off: Int32, id: Int32, m: Int32) -> Int32:
    var w = _writer(buf, off)
    w.insert_before(UInt32(id), UInt32(m))
    return Int32(w.offset)


@export
fn write_op_remove(buf: Int64, off: Int32, id: Int32) -> Int32:
    var w = _writer(buf, off)
    w.remove(UInt32(id))
    return Int32(w.offset)


@export
fn write_op_push_root(buf: Int64, off: Int32, id: Int32) -> Int32:
    var w = _writer(buf, off)
    w.push_root(UInt32(id))
    return Int32(w.offset)


# ── String-carrying opcodes ──────────────────────────────────────────────────


@export
fn write_op_create_text_node(
    buf: Int64, off: Int32, id: Int32, text: String
) -> Int32:
    var w = _writer(buf, off)
    w.create_text_node(UInt32(id), text)
    return Int32(w.offset)


@export
fn write_op_set_text(buf: Int64, off: Int32, id: Int32, text: String) -> Int32:
    var w = _writer(buf, off)
    w.set_text(UInt32(id), text)
    return Int32(w.offset)


@export
fn write_op_set_attribute(
    buf: Int64, off: Int32, id: Int32, ns: Int32, name: String, value: String
) -> Int32:
    var w = _writer(buf, off)
    w.set_attribute(UInt32(id), UInt8(ns), name, value)
    return Int32(w.offset)


@export
fn write_op_remove_attribute(
    buf: Int64, off: Int32, id: Int32, ns: Int32, name: String
) -> Int32:
    var w = _writer(buf, off)
    w.remove_attribute(UInt32(id), UInt8(ns), name)
    return Int32(w.offset)


@export
fn write_op_new_event_listener(
    buf: Int64, off: Int32, id: Int32, handler_id: Int32, name: String
) -> Int32:
    var w = _writer(buf, off)
    w.new_event_listener(UInt32(id), UInt32(handler_id), name)
    return Int32(w.offset)


@export
fn write_op_remove_event_listener(
    buf: Int64, off: Int32, id: Int32, name: String
) -> Int32:
    var w = _writer(buf, off)
    w.remove_event_listener(UInt32(id), name)
    return Int32(w.offset)


# ── Path-carrying opcodes ────────────────────────────────────────────────────


@export
fn write_op_assign_id(
    buf: Int64, off: Int32, path_ptr: Int64, path_len: Int32, id: Int32
) -> Int32:
    var w = _writer(buf, off)
    w.assign_id(_get[UInt8](path_ptr), Int(path_len), UInt32(id))
    return Int32(w.offset)


@export
fn write_op_replace_placeholder(
    buf: Int64, off: Int32, path_ptr: Int64, path_len: Int32, m: Int32
) -> Int32:
    var w = _writer(buf, off)
    w.replace_placeholder(_get[UInt8](path_ptr), Int(path_len), UInt32(m))
    return Int32(w.offset)


@export
fn write_op_register_template(
    buf: Int64, off: Int32, rt_ptr: Int64, tmpl_id: Int32
) -> Int32:
    """Serialize a registered template into the mutation buffer.

    Looks up the template by ID in the Runtime's template registry and
    writes the full OP_REGISTER_TEMPLATE record.

    Args:
        buf: Pointer to the mutation buffer.
        off: Current write offset in the buffer.
        rt_ptr: Pointer to the Runtime (owns the template registry).
        tmpl_id: Template ID to serialize.

    Returns:
        New write offset after the template record.
    """
    var w = _writer(buf, off)
    var tmpl_ptr = _get[Runtime](rt_ptr)[0].templates.get_ptr(UInt32(tmpl_id))
    w.register_template(tmpl_ptr[0].copy())
    return Int32(w.offset)


# ── Composite test helper ────────────────────────────────────────────────────


@export
fn write_test_sequence(buf: Int64) -> Int32:
    """Write a known 5-mutation sequence for integration testing."""
    var w = MutationWriter(_get[UInt8](buf), 0)
    w.load_template(1, 0, 10)
    w.create_text_node(11, String("hello"))
    w.append_children(10, 1)
    w.push_root(10)
    w.end()
    return Int32(w.offset)


# ══════════════════════════════════════════════════════════════════════════════
# Event Handler Registry Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn handler_register_signal_add(
    rt_ptr: Int64,
    scope_id: Int32,
    signal_key: Int32,
    delta: Int32,
    event_name: String,
) -> Int32:
    """Register a handler that adds `delta` to `signal_key`.  Returns handler ID.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.signal_add(
                UInt32(scope_id), UInt32(signal_key), delta, event_name
            )
        )
    )


@export
fn handler_register_signal_sub(
    rt_ptr: Int64,
    scope_id: Int32,
    signal_key: Int32,
    delta: Int32,
    event_name: String,
) -> Int32:
    """Register a handler that subtracts `delta` from `signal_key`.  Returns handler ID.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.signal_sub(
                UInt32(scope_id), UInt32(signal_key), delta, event_name
            )
        )
    )


@export
fn handler_register_signal_set(
    rt_ptr: Int64,
    scope_id: Int32,
    signal_key: Int32,
    value: Int32,
    event_name: String,
) -> Int32:
    """Register a handler that sets `signal_key` to `value`.  Returns handler ID.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.signal_set(
                UInt32(scope_id), UInt32(signal_key), value, event_name
            )
        )
    )


@export
fn handler_register_signal_toggle(
    rt_ptr: Int64,
    scope_id: Int32,
    signal_key: Int32,
    event_name: String,
) -> Int32:
    """Register a handler that toggles `signal_key` (0↔1).  Returns handler ID.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.signal_toggle(
                UInt32(scope_id), UInt32(signal_key), event_name
            )
        )
    )


@export
fn handler_register_signal_set_input(
    rt_ptr: Int64,
    scope_id: Int32,
    signal_key: Int32,
    event_name: String,
) -> Int32:
    """Register a handler that sets `signal_key` from event input.  Returns handler ID.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.signal_set_input(
                UInt32(scope_id), UInt32(signal_key), event_name
            )
        )
    )


@export
fn handler_register_signal_set_string(
    rt_ptr: Int64,
    scope_id: Int32,
    string_key: Int32,
    version_key: Int32,
    event_name: String,
) -> Int32:
    """Register a handler that sets a SignalString from a string event value.

    Phase 20: The handler stores string_key and version_key so that
    dispatch_event_with_string() can call write_signal_string().
    Returns the handler ID.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.signal_set_string(
                UInt32(scope_id),
                UInt32(string_key),
                UInt32(version_key),
                event_name,
            )
        )
    )


@export
fn handler_register_custom(
    rt_ptr: Int64, scope_id: Int32, event_name: String
) -> Int32:
    """Register a custom handler.  Returns handler ID."""
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.custom(UInt32(scope_id), event_name)
        )
    )


@export
fn handler_register_noop(
    rt_ptr: Int64, scope_id: Int32, event_name: String
) -> Int32:
    """Register a no-op handler.  Returns handler ID."""
    return Int32(
        _get[Runtime](rt_ptr)[0].register_handler(
            HandlerEntry.noop(UInt32(scope_id), event_name)
        )
    )


@export
fn handler_remove(rt_ptr: Int64, handler_id: Int32):
    """Remove an event handler by ID."""
    _get[Runtime](rt_ptr)[0].remove_handler(UInt32(handler_id))


@export
fn handler_count(rt_ptr: Int64) -> Int32:
    """Return the number of live event handlers."""
    return Int32(_get[Runtime](rt_ptr)[0].handler_count())


@export
fn handler_contains(rt_ptr: Int64, handler_id: Int32) -> Int32:
    """Check whether a handler ID is live.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].handlers.contains(UInt32(handler_id)))


@export
fn handler_scope_id(rt_ptr: Int64, handler_id: Int32) -> Int32:
    """Return the scope_id of the handler."""
    return Int32(_get[Runtime](rt_ptr)[0].handlers.scope_id(UInt32(handler_id)))


@export
fn handler_action(rt_ptr: Int64, handler_id: Int32) -> Int32:
    """Return the action tag of the handler."""
    return Int32(_get[Runtime](rt_ptr)[0].handlers.action(UInt32(handler_id)))


@export
fn handler_signal_key(rt_ptr: Int64, handler_id: Int32) -> Int32:
    """Return the signal_key of the handler."""
    return Int32(
        _get[Runtime](rt_ptr)[0].handlers.signal_key(UInt32(handler_id))
    )


@export
fn handler_operand(rt_ptr: Int64, handler_id: Int32) -> Int32:
    """Return the operand of the handler."""
    return Int32(_get[Runtime](rt_ptr)[0].handlers.operand(UInt32(handler_id)))


@export
fn dispatch_event(rt_ptr: Int64, handler_id: Int32, event_type: Int32) -> Int32:
    """Dispatch an event to a handler.  Returns 1 if action executed, 0 otherwise.
    """
    return _b2i(
        _get[Runtime](rt_ptr)[0].dispatch_event(
            UInt32(handler_id), UInt8(event_type)
        )
    )


@export
fn dispatch_event_with_i32(
    rt_ptr: Int64, handler_id: Int32, event_type: Int32, value: Int32
) -> Int32:
    """Dispatch an event with an Int32 payload.  Returns 1 if action executed.
    """
    return _b2i(
        _get[Runtime](rt_ptr)[0].dispatch_event_with_i32(
            UInt32(handler_id), UInt8(event_type), value
        )
    )


@export
fn dispatch_event_with_string(
    rt_ptr: Int64, handler_id: Int32, event_type: Int32, value: String
) -> Int32:
    """Dispatch an event with a String payload (Phase 20).

    For ACTION_SIGNAL_SET_STRING handlers, writes the string value to
    the target SignalString.  Falls back to normal dispatch otherwise.
    Returns 1 if action executed, 0 otherwise.
    """
    return _b2i(
        _get[Runtime](rt_ptr)[0].dispatch_event_with_string(
            UInt32(handler_id), UInt8(event_type), value
        )
    )


@export
fn runtime_drain_dirty(rt_ptr: Int64) -> Int32:
    """Drain the dirty scope queue.  Returns the number of dirty scopes."""
    return Int32(len(_get[Runtime](rt_ptr)[0].drain_dirty()))


# ══════════════════════════════════════════════════════════════════════════════
# Phase 8.3 — Context (Dependency Injection) Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn ctx_provide(rt_ptr: Int64, scope_id: Int32, key: Int32, value: Int32):
    """Provide a context value at the given scope."""
    _get[Runtime](rt_ptr)[0].scopes.provide_context(
        UInt32(scope_id), UInt32(key), value
    )


@export
fn ctx_consume(rt_ptr: Int64, scope_id: Int32, key: Int32) -> Int32:
    """Look up a context value by walking up the scope tree.  Returns 0 if not found.
    """
    return _get[Runtime](rt_ptr)[0].scopes.consume_context(
        UInt32(scope_id), UInt32(key)
    )[1]


@export
fn ctx_consume_found(rt_ptr: Int64, scope_id: Int32, key: Int32) -> Int32:
    """Check whether a context value exists.  Returns 1 if found, 0 if not."""
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.consume_context(
            UInt32(scope_id), UInt32(key)
        )[0]
    )


@export
fn ctx_has_local(rt_ptr: Int64, scope_id: Int32, key: Int32) -> Int32:
    """Check whether the scope itself provides a context for `key`.  Returns 1 or 0.
    """
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.has_context_local(
            UInt32(scope_id), UInt32(key)
        )
    )


@export
fn ctx_count(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Return the number of context entries provided by this scope."""
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.context_count(UInt32(scope_id))
    )


@export
fn ctx_remove(rt_ptr: Int64, scope_id: Int32, key: Int32) -> Int32:
    """Remove a context entry.  Returns 1 if removed, 0 if not found."""
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.remove_context(
            UInt32(scope_id), UInt32(key)
        )
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 8.4 — Error Boundaries Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn err_set_boundary(rt_ptr: Int64, scope_id: Int32, enabled: Int32):
    """Mark or unmark a scope as an error boundary."""
    _get[Runtime](rt_ptr)[0].scopes.set_error_boundary(
        UInt32(scope_id), enabled != 0
    )


@export
fn err_is_boundary(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Check whether the scope is an error boundary.  Returns 1 or 0."""
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.is_error_boundary(UInt32(scope_id))
    )


@export
fn err_set_error(rt_ptr: Int64, scope_id: Int32, message: String):
    """Set an error directly on the scope."""
    _get[Runtime](rt_ptr)[0].scopes.set_error(UInt32(scope_id), message)


@export
fn err_clear(rt_ptr: Int64, scope_id: Int32):
    """Clear the error state on the scope."""
    _get[Runtime](rt_ptr)[0].scopes.clear_error(UInt32(scope_id))


@export
fn err_has_error(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Check whether the scope has a captured error.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].scopes.has_error(UInt32(scope_id)))


@export
fn err_find_boundary(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Find the nearest error boundary ancestor.  Returns scope ID or -1."""
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.find_error_boundary(UInt32(scope_id))
    )


@export
fn err_propagate(rt_ptr: Int64, scope_id: Int32, message: String) -> Int32:
    """Propagate an error to its nearest error boundary.  Returns boundary ID or -1.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.propagate_error(
            UInt32(scope_id), message
        )
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 8.5 — Suspense Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn suspense_set_boundary(rt_ptr: Int64, scope_id: Int32, enabled: Int32):
    """Mark or unmark a scope as a suspense boundary."""
    _get[Runtime](rt_ptr)[0].scopes.set_suspense_boundary(
        UInt32(scope_id), enabled != 0
    )


@export
fn suspense_is_boundary(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Check whether the scope is a suspense boundary.  Returns 1 or 0."""
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.is_suspense_boundary(UInt32(scope_id))
    )


@export
fn suspense_set_pending(rt_ptr: Int64, scope_id: Int32, pending: Int32):
    """Set the pending (async loading) state on a scope."""
    _get[Runtime](rt_ptr)[0].scopes.set_pending(UInt32(scope_id), pending != 0)


@export
fn suspense_is_pending(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Check whether the scope is in a pending state.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].scopes.is_pending(UInt32(scope_id)))


@export
fn suspense_find_boundary(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Find the nearest suspense boundary ancestor.  Returns scope ID or -1."""
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.find_suspense_boundary(UInt32(scope_id))
    )


@export
fn suspense_has_pending(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Check if any descendant of `scope_id` is pending.  Returns 1 or 0."""
    return _b2i(
        _get[Runtime](rt_ptr)[0].scopes.has_pending_descendant(UInt32(scope_id))
    )


@export
fn suspense_resolve(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Mark a scope as no longer pending.  Returns suspense boundary ID or -1.
    """
    return Int32(
        _get[Runtime](rt_ptr)[0].scopes.resolve_pending(UInt32(scope_id))
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 13.2 — Memo (Computed/Derived Signal) Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn memo_create_i32(rt_ptr: Int64, scope_id: Int32, initial: Int32) -> Int32:
    """Create a memo with an initial cached value.  Returns memo ID."""
    return Int32(
        _get[Runtime](rt_ptr)[0].create_memo_i32(UInt32(scope_id), initial)
    )


@export
fn memo_begin_compute(rt_ptr: Int64, memo_id: Int32):
    """Begin memo computation — sets memo's context as current."""
    _get[Runtime](rt_ptr)[0].memo_begin_compute(UInt32(memo_id))


@export
fn memo_end_compute_i32(rt_ptr: Int64, memo_id: Int32, value: Int32):
    """End memo computation and store the result."""
    _get[Runtime](rt_ptr)[0].memo_end_compute_i32(UInt32(memo_id), value)


@export
fn memo_read_i32(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Read the memo's cached value."""
    return _get[Runtime](rt_ptr)[0].memo_read_i32(UInt32(memo_id))


@export
fn memo_is_dirty(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Check whether the memo needs recomputation.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].memo_is_dirty(UInt32(memo_id)))


@export
fn memo_destroy(rt_ptr: Int64, memo_id: Int32):
    """Destroy a memo, cleaning up its context and output signal."""
    _get[Runtime](rt_ptr)[0].destroy_memo(UInt32(memo_id))


@export
fn memo_count(rt_ptr: Int64) -> Int32:
    """Return the number of live memos."""
    return Int32(_get[Runtime](rt_ptr)[0].memo_count())


@export
fn memo_output_key(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Return the output signal key of the memo (for testing)."""
    return Int32(_get[Runtime](rt_ptr)[0].memo_output_key(UInt32(memo_id)))


@export
fn memo_context_id(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Return the reactive context ID of the memo (for testing)."""
    return Int32(_get[Runtime](rt_ptr)[0].memo_context_id(UInt32(memo_id)))


@export
fn memo_string_key(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Return the StringStore key of a string memo (for testing)."""
    return Int32(_get[Runtime](rt_ptr)[0].memo_string_key(UInt32(memo_id)))


# ══════════════════════════════════════════════════════════════════════════════
# Phase 35.1 — MemoBool Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn memo_bool_create(rt_ptr: Int64, scope_id: Int32, initial: Int32) -> Int32:
    """Create a Bool memo.  initial is 0 or 1.  Returns memo ID."""
    return Int32(
        _get[Runtime](rt_ptr)[0].create_memo_bool(
            UInt32(scope_id), initial != 0
        )
    )


@export
fn memo_bool_begin_compute(rt_ptr: Int64, memo_id: Int32):
    """Begin Bool memo computation — sets memo's context as current."""
    _get[Runtime](rt_ptr)[0].memo_begin_compute(UInt32(memo_id))


@export
fn memo_bool_end_compute(rt_ptr: Int64, memo_id: Int32, value: Int32):
    """End Bool memo computation and cache the result (0 or 1)."""
    _get[Runtime](rt_ptr)[0].memo_end_compute_bool(UInt32(memo_id), value != 0)


@export
fn memo_bool_read(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Read the Bool memo's cached value.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].memo_read_bool(UInt32(memo_id)))


@export
fn memo_bool_is_dirty(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Check whether the Bool memo needs recomputation.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].memo_is_dirty(UInt32(memo_id)))


# ══════════════════════════════════════════════════════════════════════════════
# Phase 35.1 — MemoString Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn memo_string_create(rt_ptr: Int64, scope_id: Int32, initial: String) -> Int32:
    """Create a String memo with an initial value.  Returns memo ID."""
    return Int32(
        _get[Runtime](rt_ptr)[0].create_memo_string(UInt32(scope_id), initial)
    )


@export
fn memo_string_begin_compute(rt_ptr: Int64, memo_id: Int32):
    """Begin String memo computation — sets memo's context as current."""
    _get[Runtime](rt_ptr)[0].memo_begin_compute(UInt32(memo_id))


@export
fn memo_string_end_compute(rt_ptr: Int64, memo_id: Int32, value: String):
    """End String memo computation and cache the result."""
    _get[Runtime](rt_ptr)[0].memo_end_compute_string(UInt32(memo_id), value)


@export
fn memo_string_read(rt_ptr: Int64, memo_id: Int32) -> String:
    """Read the String memo's cached value (with context tracking)."""
    return _get[Runtime](rt_ptr)[0].memo_read_string(UInt32(memo_id))


@export
fn memo_string_peek(rt_ptr: Int64, memo_id: Int32) -> String:
    """Read the String memo's cached value without subscribing."""
    return _get[Runtime](rt_ptr)[0].memo_peek_string(UInt32(memo_id))


@export
fn memo_string_is_dirty(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Check whether the String memo needs recomputation.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].memo_is_dirty(UInt32(memo_id)))


# ══════════════════════════════════════════════════════════════════════════════
# Phase 37 — Equality-Gated Memo Propagation Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn memo_did_value_change(rt_ptr: Int64, memo_id: Int32) -> Int32:
    """Check whether the last end_compute changed the memo's value.

    Returns 1 if the value changed, 0 if it was value-stable (new == old).
    """
    return _b2i(_get[Runtime](rt_ptr)[0].memo_did_value_change(UInt32(memo_id)))


@export
fn runtime_settle_scopes(rt_ptr: Int64):
    """Remove dirty scopes whose subscribed signals didn't actually change.

    Call after run_memos() and before render() to skip unnecessary
    re-renders when memo equality gates cancel downstream dirtiness.
    """
    _get[Runtime](rt_ptr)[0].settle_scopes()


@export
fn runtime_clear_changed_signals(rt_ptr: Int64):
    """Reset the changed-signals tracking set.

    Normally called automatically by settle_scopes().  Exposed for
    testing scenarios that need to reset between operations.
    """
    _get[Runtime](rt_ptr)[0].clear_changed_signals()


@export
fn runtime_signal_changed(rt_ptr: Int64, key: Int32) -> Int32:
    """Check whether a signal was written with a changed value this cycle.

    Returns 1 if the signal key appears in _changed_signals, 0 otherwise.
    """
    return _b2i(_get[Runtime](rt_ptr)[0].signal_changed_this_cycle(UInt32(key)))


# ══════════════════════════════════════════════════════════════════════════════
# Phase 38 — Batch Signal Writes Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn runtime_begin_batch(rt_ptr: Int64):
    """Enter batch mode for signal writes.

    While batching, signal writes store values immediately (reads see
    the new value) but defer subscriber scanning and worklist propagation
    until the outermost end_batch().  Can be nested.
    """
    _get[Runtime](rt_ptr)[0].begin_batch()


@export
fn runtime_end_batch(rt_ptr: Int64):
    """Exit batch mode and propagate all deferred writes.

    On the outermost call, runs a single combined propagation pass
    over all signals written during the batch.  Nested calls just
    decrement the depth counter.
    """
    _get[Runtime](rt_ptr)[0].end_batch()


@export
fn runtime_is_batching(rt_ptr: Int64) -> Int32:
    """Return 1 if currently inside a begin_batch/end_batch bracket."""
    if _get[Runtime](rt_ptr)[0].is_batching():
        return 1
    return 0


# ══════════════════════════════════════════════════════════════════════════════
# Phase 14 — Effect (Reactive Side Effect) Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn effect_create(rt_ptr: Int64, scope_id: Int32) -> Int32:
    """Create an effect with a reactive context.  Returns effect ID."""
    return Int32(_get[Runtime](rt_ptr)[0].create_effect(UInt32(scope_id)))


@export
fn effect_begin_run(rt_ptr: Int64, effect_id: Int32):
    """Begin effect execution — sets effect's context as current."""
    _get[Runtime](rt_ptr)[0].effect_begin_run(UInt32(effect_id))


@export
fn effect_end_run(rt_ptr: Int64, effect_id: Int32):
    """End effect execution — clears pending, restores context."""
    _get[Runtime](rt_ptr)[0].effect_end_run(UInt32(effect_id))


@export
fn effect_is_pending(rt_ptr: Int64, effect_id: Int32) -> Int32:
    """Check whether the effect needs re-execution.  Returns 1 or 0."""
    return _b2i(_get[Runtime](rt_ptr)[0].effect_is_pending(UInt32(effect_id)))


@export
fn effect_destroy(rt_ptr: Int64, effect_id: Int32):
    """Destroy an effect, cleaning up its context signal."""
    _get[Runtime](rt_ptr)[0].destroy_effect(UInt32(effect_id))


@export
fn effect_count(rt_ptr: Int64) -> Int32:
    """Return the number of live effects."""
    return Int32(_get[Runtime](rt_ptr)[0].effect_count())


@export
fn effect_context_id(rt_ptr: Int64, effect_id: Int32) -> Int32:
    """Return the reactive context ID of the effect (for testing)."""
    return Int32(_get[Runtime](rt_ptr)[0].effect_context_id(UInt32(effect_id)))


@export
fn effect_drain_pending(rt_ptr: Int64) -> Int32:
    """Return the number of currently pending effects."""
    return Int32(_get[Runtime](rt_ptr)[0].pending_effect_count())


@export
fn effect_pending_at(rt_ptr: Int64, index: Int32) -> Int32:
    """Return the effect ID at the given index in the pending list."""
    return Int32(_get[Runtime](rt_ptr)[0].pending_effect_at(Int(index)))


# ██████████████████████████████████████████████████████████████████████████████
#
#   SECTION 3 — App WASM Export Wrappers
#
#   Thin @export wrappers forwarding to app modules.  App struct and
#   lifecycle code lives in dedicated modules under src/apps/ (or
#   examples/ for Counter, Todo, Bench).  Only the @export annotations
#   live here because Mojo requires exports in the main compilation unit.
#
#   Apps (in order):
#     Counter        — examples/counter/counter.mojo
#     Todo           — examples/todo/todo.mojo
#     Benchmark      — examples/bench/bench.mojo
#     MultiView      — src/app.mojo (client-side routing)
#     ChildCounter   — src/apps/child_counter.mojo
#     ContextTest    — src/apps/context_test.mojo
#     ChildContextTest — src/apps/child_context_test.mojo
#     PropsCounter   — src/apps/props_counter.mojo
#     ThemeCounter   — src/apps/theme_counter.mojo
#     SafeCounter    — src/apps/safe_counter.mojo
#     ErrorNest      — src/apps/error_nest.mojo
#     DataLoader     — src/apps/data_loader.mojo
#     SuspenseNest   — src/apps/suspense_nest.mojo
#     EffectDemo     — src/apps/effect_demo.mojo
#     EffectMemo     — src/apps/effect_memo.mojo
#     MemoForm       — src/apps/memo_form.mojo
#     MemoChain      — src/apps/memo_chain.mojo
#     EqualityDemo   — src/apps/equality_demo.mojo
#     BatchDemo      — src/apps/batch_demo.mojo
#
# ██████████████████████████████████████████████████████████████████████████████


# ══════════════════════════════════════════════════════════════════════════════
# Phase 7 — Counter App (End-to-End)
# ══════════════════════════════════════════════════════════════════════════════
#
# Thin @export wrappers calling into examples/counter/counter.mojo.


@export
fn counter_init() -> Int64:
    """Initialize the counter app.  Returns a pointer to the app state."""
    return gui_app_init[CounterApp]()


@export
fn counter_destroy(app_ptr: Int64):
    """Destroy the counter app and free all resources."""
    gui_app_destroy[CounterApp](app_ptr)


@export
fn counter_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount) of the counter app.  Returns mutation byte length.
    """
    return gui_app_mount[CounterApp](app_ptr, buf_ptr, capacity)


@export
fn counter_handle_event(
    app_ptr: Int64, handler_id: Int32, event_type: Int32
) -> Int32:
    """Dispatch an event to the counter app.  Returns 1 if action executed."""
    return gui_app_handle_event[CounterApp](app_ptr, handler_id, event_type)


@export
fn counter_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0 if nothing dirty.
    """
    return gui_app_flush[CounterApp](app_ptr, buf_ptr, capacity)


# ── Counter App Query Exports ────────────────────────────────────────────────


@export
fn counter_rt_ptr(app_ptr: Int64) -> Int64:
    """Return the runtime pointer for JS template registration."""
    return _to_i64(_get[CounterApp](app_ptr)[0].ctx.shell.runtime)


@export
fn counter_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the counter template ID."""
    return Int32(_get[CounterApp](app_ptr)[0].ctx.template_id)


@export
fn counter_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment handler ID (from view_events[0])."""
    var events = _get[CounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 0:
        return Int32(events[0].handler_id)
    return -1


@export
fn counter_decr_handler(app_ptr: Int64) -> Int32:
    """Return the decrement handler ID (from view_events[1])."""
    var events = _get[CounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 1:
        return Int32(events[1].handler_id)
    return -1


@export
fn counter_count_value(app_ptr: Int64) -> Int32:
    """Peek the current count signal value (without subscribing)."""
    return _get[CounterApp](app_ptr)[0].count.peek()


@export
fn counter_has_dirty(app_ptr: Int64) -> Int32:
    """Check if the counter app has dirty scopes.  Returns 1 or 0."""
    return gui_app_has_dirty[CounterApp](app_ptr)


@export
fn counter_scope_id(app_ptr: Int64) -> Int32:
    """Return the counter app's root scope ID."""
    return Int32(_get[CounterApp](app_ptr)[0].ctx.scope_id)


@export
fn counter_count_signal(app_ptr: Int64) -> Int32:
    """Return the counter app's count signal key."""
    return Int32(_get[CounterApp](app_ptr)[0].count.key)


@export
fn counter_doubled_value(app_ptr: Int64) -> Int32:
    """Return doubled count value (computed inline, no memo)."""
    return _get[CounterApp](app_ptr)[0].count.peek() * 2


@export
fn counter_doubled_memo(app_ptr: Int64) -> Int32:
    """Return -1 (doubled memo removed in ergonomic counter rewrite)."""
    return -1


@export
fn counter_toggle_handler(app_ptr: Int64) -> Int32:
    """Return the toggle detail handler ID (from view_events[2])."""
    var events = _get[CounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 2:
        return Int32(events[2].handler_id)
    return -1


@export
fn counter_show_detail(app_ptr: Int64) -> Int32:
    """Peek the show_detail signal value (0 = hidden, 1 = shown)."""
    return _get[CounterApp](app_ptr)[0].show_detail.peek_i32()


@export
fn counter_detail_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the detail template ID."""
    return Int32(_get[CounterApp](app_ptr)[0].detail_tmpl)


@export
fn counter_cond_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the conditional detail slot is mounted, 0 otherwise."""
    if _get[CounterApp](app_ptr)[0].cond_slot.mounted:
        return 1
    return 0


# ══════════════════════════════════════════════════════════════════════════════
# Phase 8 — Todo App
# ══════════════════════════════════════════════════════════════════════════════
#
# Thin @export wrappers calling into apps.todo module.


@export
fn todo_init() -> Int64:
    """Initialize the todo app.  Returns a pointer to the app state."""
    return gui_app_init[TodoApp]()


@export
fn todo_destroy(app_ptr: Int64):
    """Destroy the todo app and free all resources."""
    gui_app_destroy[TodoApp](app_ptr)


@export
fn todo_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount) of the todo app.  Returns mutation byte length."""
    return gui_app_mount[TodoApp](app_ptr, buf_ptr, capacity)


@export
fn todo_handle_event(
    app_ptr: Int64, handler_id: Int32, event_type: Int32
) -> Int32:
    """Dispatch a click event by handler ID.

    Returns 1 if the handler was found and action executed (toggle/remove),
    0 if the handler is the add handler (JS must read input) or unknown.
    """
    return gui_app_handle_event[TodoApp](app_ptr, handler_id, event_type)


@export
fn todo_add_item(app_ptr: Int64, text: String):
    """Add a new item to the todo list."""
    _get[TodoApp](app_ptr)[0].add_item(text)


@export
fn todo_dispatch_string(
    app_ptr: Int64, handler_id: Int32, event_type: Int32, value: String
) -> Int32:
    """Dispatch a string event (input/change) to the todo app.

    Phase 20 (M20.5): Used by JS EventBridge for input events — extracts
    event.target.value as a string and dispatches via the string dispatch
    path.  For ACTION_SIGNAL_SET_STRING handlers, this writes the string
    to the SignalString and bumps the version signal.

    Returns 1 if the handler was found and action executed, 0 otherwise.
    """
    return gui_app_handle_event_string[TodoApp](
        app_ptr, handler_id, event_type, value
    )


@export
fn todo_add_handler_id(app_ptr: Int64) -> Int32:
    """Return the Add button handler ID.

    The Add handler is auto-registered by register_view() via
    onclick_custom().  This export lets JS and tests retrieve it.
    """
    return Int32(_get[TodoApp](app_ptr)[0].add_handler)


@export
fn todo_enter_handler_id(app_ptr: Int64) -> Int32:
    """Return the Enter key handler ID (Phase 22).

    The Enter handler is auto-registered by register_view() via
    onkeydown_enter_custom().  Dispatched via dispatch_event_with_string
    with the key name — only "Enter" triggers the action.
    """
    return Int32(_get[TodoApp](app_ptr)[0].enter_handler)


@export
fn todo_remove_item(app_ptr: Int64, item_id: Int32):
    """Remove an item by its ID."""
    _get[TodoApp](app_ptr)[0].remove_item(item_id)


@export
fn todo_toggle_item(app_ptr: Int64, item_id: Int32):
    """Toggle an item's completed status."""
    _get[TodoApp](app_ptr)[0].toggle_item(item_id)


@export
fn todo_set_input(app_ptr: Int64, text: String):
    """Update the input text via SignalString.set() (no re-render).

    Phase 19: input_text is now a SignalString created via
    create_signal_string() — no scope is subscribed, so set() bumps
    the companion version signal but does NOT dirty any scope.
    """
    _get[TodoApp](app_ptr)[0].input_text.set(text)


@export
fn todo_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0 if nothing dirty.
    """
    return gui_app_flush[TodoApp](app_ptr, buf_ptr, capacity)


# ── Todo App Query Exports ───────────────────────────────────────────────────


@export
fn todo_app_template_id(app_ptr: Int64) -> Int32:
    """Return the app template ID."""
    return Int32(_get[TodoApp](app_ptr)[0].ctx.template_id)


@export
fn todo_item_template_id(app_ptr: Int64) -> Int32:
    """Return the item template ID."""
    return Int32(_get[TodoApp](app_ptr)[0].items.template_id)


@export
fn todo_add_handler(app_ptr: Int64) -> Int32:
    """Return the Add button handler ID."""
    return Int32(_get[TodoApp](app_ptr)[0].add_handler)


@export
fn todo_item_count(app_ptr: Int64) -> Int32:
    """Return the number of items in the list."""
    return Int32(len(_get[TodoApp](app_ptr)[0].data))


@export
fn todo_item_id_at(app_ptr: Int64, index: Int32) -> Int32:
    """Return the ID of the item at the given index."""
    return _get[TodoApp](app_ptr)[0].data[Int(index)].id


@export
fn todo_item_completed_at(app_ptr: Int64, index: Int32) -> Int32:
    """Return 1 if the item at index is completed, 0 otherwise."""
    return _b2i(_get[TodoApp](app_ptr)[0].data[Int(index)].completed)


@export
fn todo_has_dirty(app_ptr: Int64) -> Int32:
    """Check if the todo app has dirty scopes.  Returns 1 or 0."""
    return gui_app_has_dirty[TodoApp](app_ptr)


@export
fn todo_list_version(app_ptr: Int64) -> Int32:
    """Return the current list version signal value."""
    return _get[TodoApp](app_ptr)[0].list_version.peek()


@export
fn todo_empty_msg_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the empty state message is mounted, 0 otherwise (Phase 28).
    """
    if _get[TodoApp](app_ptr)[0].empty_msg_slot.mounted:
        return 1
    return 0


@export
fn todo_scope_id(app_ptr: Int64) -> Int32:
    """Return the root scope ID."""
    return Int32(_get[TodoApp](app_ptr)[0].ctx.scope_id)


@export
fn todo_handler_count(app_ptr: Int64) -> Int32:
    """Return the number of live event handlers in the todo app's runtime."""
    return Int32(_get[TodoApp](app_ptr)[0].ctx.handler_count())


@export
fn todo_handler_map_count(app_ptr: Int64) -> Int32:
    """Return the number of handler→action mappings in the todo KeyedList.

    Phase 17: This reflects the number of custom events registered via
    ItemBuilder.add_custom_event() since the last begin_rebuild().
    Each item registers 2 handlers (toggle + remove), so this should
    equal 2 * item_count after a rebuild.
    """
    return Int32(_get[TodoApp](app_ptr)[0].items.handler_count())


@export
fn todo_input_version(app_ptr: Int64) -> Int32:
    """Return the input text SignalString's version (Phase 19).

    The version increments on every set() call — useful for staleness
    checks and verifying that SignalString write tracking works.
    """
    return Int32(_get[TodoApp](app_ptr)[0].input_text.version())


@export
fn todo_input_is_empty(app_ptr: Int64) -> Int32:
    """Return 1 if the input text is empty, 0 otherwise (Phase 19).

    Demonstrates SignalString.is_empty() — reads via peek (no subscription).
    """
    return _b2i(_get[TodoApp](app_ptr)[0].input_text.is_empty())


@export
fn todo_handler_action(app_ptr: Int64, handler_id: Int32) -> Int32:
    """Look up a handler ID in the todo KeyedList's handler map.

    Phase 17: Returns the action tag (1=toggle, 2=remove) if found,
    or -1 if the handler ID is not in the map.
    """
    var action = _get[TodoApp](app_ptr)[0].items.get_action(UInt32(handler_id))
    if action.found:
        return Int32(action.tag)
    return -1


@export
fn todo_handler_action_data(app_ptr: Int64, handler_id: Int32) -> Int32:
    """Return the data (item ID) for a handler in the todo KeyedList's map.

    Phase 17: Returns the item ID if found, or -1 if not found.
    """
    var action = _get[TodoApp](app_ptr)[0].items.get_action(UInt32(handler_id))
    if action.found:
        return action.data
    return -1


# ══════════════════════════════════════════════════════════════════════════════
# Phase 9 — Benchmark App
# ══════════════════════════════════════════════════════════════════════════════
#
# Thin @export wrappers calling into apps.bench module.


@export
fn bench_init() -> Int64:
    """Initialize the benchmark app.  Returns a pointer to the app state."""
    return gui_app_init[BenchmarkApp]()


@export
fn bench_destroy(app_ptr: Int64):
    """Destroy the benchmark app and free all resources."""
    gui_app_destroy[BenchmarkApp](app_ptr)


@export
fn bench_create(app_ptr: Int64, count: Int32):
    """Replace all rows with `count` new rows (benchmark: create)."""
    _get[BenchmarkApp](app_ptr)[0].create_rows(Int(count))


@export
fn bench_append(app_ptr: Int64, count: Int32):
    """Append `count` new rows (benchmark: append)."""
    _get[BenchmarkApp](app_ptr)[0].append_rows(Int(count))


@export
fn bench_update(app_ptr: Int64):
    """Update every 10th row label (benchmark: update)."""
    _get[BenchmarkApp](app_ptr)[0].update_every_10th()


@export
fn bench_select(app_ptr: Int64, id: Int32):
    """Select a row by id (benchmark: select)."""
    _get[BenchmarkApp](app_ptr)[0].select_row(id)


@export
fn bench_swap(app_ptr: Int64):
    """Swap rows at indices 1 and 998 (benchmark: swap)."""
    _get[BenchmarkApp](app_ptr)[0].swap_rows(1, 998)


@export
fn bench_remove(app_ptr: Int64, id: Int32):
    """Remove a row by id (benchmark: remove)."""
    _get[BenchmarkApp](app_ptr)[0].remove_row(id)


@export
fn bench_clear(app_ptr: Int64):
    """Clear all rows (benchmark: clear)."""
    _get[BenchmarkApp](app_ptr)[0].clear_rows()


@export
fn bench_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render of the benchmark table body.  Returns mutation byte length.
    """
    return gui_app_mount[BenchmarkApp](app_ptr, buf_ptr, capacity)


@export
fn bench_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0 if nothing dirty.
    """
    return gui_app_flush[BenchmarkApp](app_ptr, buf_ptr, capacity)


@export
fn bench_handle_event(
    app_ptr: Int64, handler_id: Int32, event_type: Int32
) -> Int32:
    """Dispatch a click event by handler ID.

    Phase 24.1: Routes row click events via the KeyedList handler_map.
    EventBridge calls this automatically when bench_handle_event exists
    as a WASM export.  Eliminates the need for JS-side tbody event
    delegation.

    Returns 1 if the handler was found and action executed (select/remove),
    0 otherwise.
    """
    return gui_app_handle_event[BenchmarkApp](app_ptr, handler_id, event_type)


# ── Benchmark App Query Exports ──────────────────────────────────────────────


@export
fn bench_row_count(app_ptr: Int64) -> Int32:
    """Return the number of rows."""
    return Int32(len(_get[BenchmarkApp](app_ptr)[0].rows))


@export
fn bench_row_id_at(app_ptr: Int64, index: Int32) -> Int32:
    """Return the id of the row at the given index."""
    return _get[BenchmarkApp](app_ptr)[0].rows[Int(index)].id


@export
fn bench_selected(app_ptr: Int64) -> Int32:
    """Return the currently selected row id (0 = none)."""
    return _get[BenchmarkApp](app_ptr)[0].selected.peek()


@export
fn bench_has_dirty(app_ptr: Int64) -> Int32:
    """Check if the benchmark app has dirty scopes.  Returns 1 or 0."""
    return gui_app_has_dirty[BenchmarkApp](app_ptr)


@export
fn bench_version(app_ptr: Int64) -> Int32:
    """Return the current version signal value."""
    return _get[BenchmarkApp](app_ptr)[0].version.peek()


@export
fn bench_row_template_id(app_ptr: Int64) -> Int32:
    """Return the row template ID."""
    return Int32(_get[BenchmarkApp](app_ptr)[0].rows_list.template_id)


@export
fn bench_scope_id(app_ptr: Int64) -> Int32:
    """Return the root scope ID."""
    return Int32(_get[BenchmarkApp](app_ptr)[0].ctx.scope_id)


@export
fn bench_handler_count(app_ptr: Int64) -> Int32:
    """Return the number of live event handlers in the bench app's runtime."""
    return Int32(_get[BenchmarkApp](app_ptr)[0].ctx.handler_count())


@export
fn bench_handler_map_count(app_ptr: Int64) -> Int32:
    """Return the number of handler→action mappings in the bench KeyedList.

    Phase 17: This reflects the number of custom events registered via
    ItemBuilder.add_custom_event() since the last begin_rebuild().
    Each row registers 2 handlers (select + remove), so this should
    equal 2 * row_count after a rebuild.
    """
    return Int32(_get[BenchmarkApp](app_ptr)[0].rows_list.handler_count())


@export
fn bench_handler_id_at(app_ptr: Int64, index: Int32) -> Int32:
    """Return the toolbar handler ID at the given index (0–5).

    Phase 24.3: Enables JS tests to look up toolbar handler IDs by
    tree-walk order index for programmatic dispatch:
      0 = create1k, 1 = create10k, 2 = append,
      3 = update, 4 = swap, 5 = clear.

    Returns 0 for out-of-range indices.
    """
    var app = _get[BenchmarkApp](app_ptr)
    if index == 0:
        return Int32(app[0].create1k_handler)
    elif index == 1:
        return Int32(app[0].create10k_handler)
    elif index == 2:
        return Int32(app[0].append_handler)
    elif index == 3:
        return Int32(app[0].update_handler)
    elif index == 4:
        return Int32(app[0].swap_handler)
    elif index == 5:
        return Int32(app[0].clear_handler)
    return 0


@export
fn bench_status_text(app_ptr: Int64) -> String:
    """Return the full status bar text (concatenation of all 3 parts).

    Phase 24.4: Returns op_name + timing_text + row_count_text.
    Before any operation: "Ready" (timing_text and row_count_text are "").
    After an operation: e.g. "Create 1,000 rows — 12.3ms · 1,000 rows".

    Backward-compatible with P24.3 tests that check startsWith/includes.
    """
    var app = _get[BenchmarkApp](app_ptr)
    return app[0].op_name + app[0].timing_text + app[0].row_count_text


@export
fn bench_op_name(app_ptr: Int64) -> String:
    """Return the operation name (dyn_text[0]) from the status bar.

    Phase 24.4: "Ready" before any operation, or the operation name
    (e.g. "Create 1,000 rows") after a toolbar action.
    """
    return _get[BenchmarkApp](app_ptr)[0].op_name


@export
fn bench_timing_text(app_ptr: Int64) -> String:
    """Return the timing text (dyn_text[1]) from the status bar.

    Phase 24.4: "" before any operation, or " — X.Yms" after a
    toolbar action (includes leading em-dash separator).
    """
    return _get[BenchmarkApp](app_ptr)[0].timing_text


@export
fn bench_row_count_text(app_ptr: Int64) -> String:
    """Return the row count text (dyn_text[2]) from the status bar.

    Phase 24.4: "" before any operation, or " · N rows" after a
    toolbar action (includes leading middle-dot separator).
    """
    return _get[BenchmarkApp](app_ptr)[0].row_count_text


# ══════════════════════════════════════════════════════════════════════════════
# Phase 9.4 — Signal Write Batching (replaced by Phase 38 exports above)
# ══════════════════════════════════════════════════════════════════════════════


# ══════════════════════════════════════════════════════════════════════════════
# Phase 9.5 — Debug Mode
# ══════════════════════════════════════════════════════════════════════════════


@export
fn debug_signal_store_capacity(rt_ptr: Int64) -> Int32:
    """Return the total number of signal slots (occupied + free)."""
    return Int32(len(_get[Runtime](rt_ptr)[0].signals._entries))


@export
fn debug_scope_store_capacity(rt_ptr: Int64) -> Int32:
    """Return the total number of scope slots (occupied + free)."""
    return Int32(len(_get[Runtime](rt_ptr)[0].scopes._scopes))


@export
fn debug_vnode_store_count(store_ptr: Int64) -> Int32:
    """Return the number of VNodes in a standalone store."""
    return Int32(_get[VNodeStore](store_ptr)[0].count())


@export
fn debug_handler_store_capacity(rt_ptr: Int64) -> Int32:
    """Return the total number of handler slots."""
    return Int32(len(_get[Runtime](rt_ptr)[0].handlers._entries))


@export
fn debug_eid_alloc_capacity(alloc_ptr: Int64) -> Int32:
    """Return the total number of ElementId slots."""
    return Int32(len(_get[ElementIdAllocator](alloc_ptr)[0]._slots))


# ── Memory Management Test Exports ───────────────────────────────────────────


@export
fn mem_test_signal_cycle(rt_ptr: Int64, count: Int32) -> Int32:
    """Create and destroy `count` signals.  Returns final signal_count (should be 0).
    """
    var rt = _get[Runtime](rt_ptr)
    var keys = List[UInt32]()
    for i in range(Int(count)):
        keys.append(rt[0].create_signal[Int32](Int32(i)))
    for i in range(Int(count)):
        rt[0].destroy_signal(keys[i])
    return Int32(rt[0].signals.signal_count())


@export
fn mem_test_scope_cycle(rt_ptr: Int64, count: Int32) -> Int32:
    """Create and destroy `count` scopes.  Returns final scope_count (should be 0).
    """
    var rt = _get[Runtime](rt_ptr)
    var ids = List[UInt32]()
    for i in range(Int(count)):
        ids.append(rt[0].create_scope(0, -1))
    for i in range(Int(count)):
        rt[0].destroy_scope(ids[i])
    return Int32(rt[0].scope_count())


@export
fn mem_test_rapid_writes(rt_ptr: Int64, key: Int32, count: Int32) -> Int32:
    """Write to a signal `count` times.  Returns final dirty_count."""
    var rt = _get[Runtime](rt_ptr)
    for i in range(Int(count)):
        rt[0].write_signal[Int32](UInt32(key), Int32(i))
    return Int32(rt[0].dirty_count())


# ══════════════════════════════════════════════════════════════════════════════
# Phase 10.4 — Scheduler Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn scheduler_create() -> Int64:
    """Allocate a Scheduler on the heap.  Returns its pointer."""
    return _to_i64(_heap_new(Scheduler()))


@export
fn scheduler_destroy(sched_ptr: Int64):
    """Destroy and free a heap-allocated Scheduler."""
    _heap_del(_get[Scheduler](sched_ptr))


@export
fn scheduler_collect(sched_ptr: Int64, rt_ptr: Int64):
    """Drain the runtime's dirty queue into the scheduler."""
    _get[Scheduler](sched_ptr)[0].collect(_get[Runtime](rt_ptr))


@export
fn scheduler_collect_one(sched_ptr: Int64, rt_ptr: Int64, scope_id: Int32):
    """Add a single scope to the scheduler queue."""
    _get[Scheduler](sched_ptr)[0].collect_one(
        _get[Runtime](rt_ptr), UInt32(scope_id)
    )


@export
fn scheduler_next(sched_ptr: Int64) -> Int32:
    """Return and remove the next scope to render (lowest height first)."""
    return Int32(_get[Scheduler](sched_ptr)[0].next())


@export
fn scheduler_is_empty(sched_ptr: Int64) -> Int32:
    """Check if the scheduler has no pending dirty scopes.  Returns 1 or 0."""
    return _b2i(_get[Scheduler](sched_ptr)[0].is_empty())


@export
fn scheduler_count(sched_ptr: Int64) -> Int32:
    """Return the number of pending dirty scopes."""
    return Int32(_get[Scheduler](sched_ptr)[0].count())


@export
fn scheduler_has_scope(sched_ptr: Int64, scope_id: Int32) -> Int32:
    """Check if a scope is already in the scheduler queue.  Returns 1 or 0."""
    return _b2i(_get[Scheduler](sched_ptr)[0].has_scope(UInt32(scope_id)))


@export
fn scheduler_clear(sched_ptr: Int64):
    """Discard all pending dirty scopes."""
    _get[Scheduler](sched_ptr)[0].clear()


# ══════════════════════════════════════════════════════════════════════════════
# Phase 10.4 — AppShell Exports
# ══════════════════════════════════════════════════════════════════════════════


@export
fn shell_create() -> Int64:
    """Create an AppShell with all subsystems allocated.  Returns its pointer.
    """
    return _to_i64(_heap_new(app_shell_create()))


@export
fn shell_destroy(shell_ptr: Int64):
    """Destroy an AppShell and free all resources."""
    var ptr = _get[AppShell](shell_ptr)
    ptr[0].destroy()
    _heap_del(ptr)


@export
fn shell_is_alive(shell_ptr: Int64) -> Int32:
    """Check if the shell is alive.  Returns 1 or 0."""
    return _b2i(_get[AppShell](shell_ptr)[0].is_alive())


@export
fn shell_create_root_scope(shell_ptr: Int64) -> Int32:
    """Create a root scope via the AppShell.  Returns scope ID."""
    return Int32(_get[AppShell](shell_ptr)[0].create_root_scope())


@export
fn shell_create_child_scope(shell_ptr: Int64, parent_id: Int32) -> Int32:
    """Create a child scope via the AppShell.  Returns scope ID."""
    return Int32(
        _get[AppShell](shell_ptr)[0].create_child_scope(UInt32(parent_id))
    )


@export
fn shell_create_signal_i32(shell_ptr: Int64, initial: Int32) -> Int32:
    """Create an Int32 signal via the AppShell.  Returns signal key."""
    return Int32(_get[AppShell](shell_ptr)[0].create_signal_i32(initial))


@export
fn shell_read_signal_i32(shell_ptr: Int64, key: Int32) -> Int32:
    """Read an Int32 signal via the AppShell (with context tracking)."""
    return _get[AppShell](shell_ptr)[0].read_signal_i32(UInt32(key))


@export
fn shell_peek_signal_i32(shell_ptr: Int64, key: Int32) -> Int32:
    """Peek an Int32 signal via the AppShell (without subscribing)."""
    return _get[AppShell](shell_ptr)[0].peek_signal_i32(UInt32(key))


@export
fn shell_write_signal_i32(shell_ptr: Int64, key: Int32, value: Int32):
    """Write to an Int32 signal via the AppShell."""
    _get[AppShell](shell_ptr)[0].write_signal_i32(UInt32(key), value)


@export
fn shell_begin_render(shell_ptr: Int64, scope_id: Int32) -> Int32:
    """Begin rendering a scope.  Returns previous scope ID (or -1)."""
    return Int32(_get[AppShell](shell_ptr)[0].begin_render(UInt32(scope_id)))


@export
fn shell_end_render(shell_ptr: Int64, prev_scope: Int32):
    """End rendering and restore the previous scope."""
    _get[AppShell](shell_ptr)[0].end_render(Int(prev_scope))


@export
fn shell_has_dirty(shell_ptr: Int64) -> Int32:
    """Check if the shell has dirty scopes.  Returns 1 or 0."""
    return _b2i(_get[AppShell](shell_ptr)[0].has_dirty())


@export
fn shell_collect_dirty(shell_ptr: Int64):
    """Drain dirty scopes into the shell's scheduler."""
    _get[AppShell](shell_ptr)[0].collect_dirty()


@export
fn shell_next_dirty(shell_ptr: Int64) -> Int32:
    """Return next dirty scope from the shell's scheduler."""
    return Int32(_get[AppShell](shell_ptr)[0].next_dirty())


@export
fn shell_scheduler_empty(shell_ptr: Int64) -> Int32:
    """Check if the shell's scheduler is empty.  Returns 1 or 0."""
    return _b2i(_get[AppShell](shell_ptr)[0].scheduler_empty())


@export
fn shell_dispatch_event(
    shell_ptr: Int64, handler_id: Int32, event_type: Int32
) -> Int32:
    """Dispatch an event via the AppShell.  Returns 1 if executed."""
    return _b2i(
        _get[AppShell](shell_ptr)[0].dispatch_event(
            UInt32(handler_id), UInt8(event_type)
        )
    )


@export
fn shell_dispatch_event_with_string(
    shell_ptr: Int64, handler_id: Int32, event_type: Int32, value: String
) -> Int32:
    """Dispatch an event with a String payload via the AppShell (Phase 20).

    For ACTION_SIGNAL_SET_STRING handlers, writes the string value to
    the target SignalString.  Falls back to normal dispatch otherwise.
    Returns 1 if executed.
    """
    return _b2i(
        _get[AppShell](shell_ptr)[0].dispatch_event_with_string(
            UInt32(handler_id), UInt8(event_type), value
        )
    )


# ── Shell memo helpers (M13.5) ──────────────────────────────────────────────


@export
fn shell_memo_create_i32(
    shell_ptr: Int64, scope_id: Int32, initial: Int32
) -> Int32:
    """Create an Int32 memo via the AppShell.  Returns memo ID."""
    return Int32(
        _get[AppShell](shell_ptr)[0].create_memo_i32(UInt32(scope_id), initial)
    )


@export
fn shell_memo_begin_compute(shell_ptr: Int64, memo_id: Int32):
    """Begin memo computation via the AppShell."""
    _get[AppShell](shell_ptr)[0].memo_begin_compute(UInt32(memo_id))


@export
fn shell_memo_end_compute_i32(shell_ptr: Int64, memo_id: Int32, value: Int32):
    """End memo computation and cache the result via the AppShell."""
    _get[AppShell](shell_ptr)[0].memo_end_compute_i32(UInt32(memo_id), value)


@export
fn shell_memo_read_i32(shell_ptr: Int64, memo_id: Int32) -> Int32:
    """Read a memo's cached value via the AppShell."""
    return _get[AppShell](shell_ptr)[0].memo_read_i32(UInt32(memo_id))


@export
fn shell_memo_is_dirty(shell_ptr: Int64, memo_id: Int32) -> Int32:
    """Check whether the memo needs recomputation.  Returns 1 or 0."""
    return _b2i(_get[AppShell](shell_ptr)[0].memo_is_dirty(UInt32(memo_id)))


@export
fn shell_use_memo_i32(shell_ptr: Int64, initial: Int32) -> Int32:
    """Hook: create or retrieve an Int32 memo via the AppShell."""
    return Int32(_get[AppShell](shell_ptr)[0].use_memo_i32(initial))


# ══════════════════════════════════════════════════════════════════════════════
# Phase 14.4 — AppShell Effect Helpers
# ══════════════════════════════════════════════════════════════════════════════


@export
fn shell_effect_create(shell_ptr: Int64, scope_id: Int32) -> Int32:
    """Create an effect via the AppShell.  Returns effect ID."""
    return Int32(_get[AppShell](shell_ptr)[0].create_effect(UInt32(scope_id)))


@export
fn shell_effect_begin_run(shell_ptr: Int64, effect_id: Int32):
    """Begin effect execution via the AppShell."""
    _get[AppShell](shell_ptr)[0].effect_begin_run(UInt32(effect_id))


@export
fn shell_effect_end_run(shell_ptr: Int64, effect_id: Int32):
    """End effect execution via the AppShell."""
    _get[AppShell](shell_ptr)[0].effect_end_run(UInt32(effect_id))


@export
fn shell_effect_is_pending(shell_ptr: Int64, effect_id: Int32) -> Int32:
    """Check whether effect needs re-execution via the AppShell.  Returns 1 or 0.
    """
    return _b2i(
        _get[AppShell](shell_ptr)[0].effect_is_pending(UInt32(effect_id))
    )


@export
fn shell_use_effect(shell_ptr: Int64) -> Int32:
    """Hook: create or retrieve an effect via the AppShell."""
    return Int32(_get[AppShell](shell_ptr)[0].use_effect())


@export
fn shell_effect_drain_pending(shell_ptr: Int64) -> Int32:
    """Return the number of pending effects via the AppShell."""
    return Int32(_get[AppShell](shell_ptr)[0].pending_effect_count())


@export
fn shell_effect_pending_at(shell_ptr: Int64, index: Int32) -> Int32:
    """Return the pending effect ID at the given index via the AppShell."""
    return Int32(_get[AppShell](shell_ptr)[0].pending_effect_at(Int(index)))


@export
fn shell_rt_ptr(shell_ptr: Int64) -> Int64:
    """Return the runtime pointer from an AppShell (for template registration etc.).
    """
    return _to_i64(_get[AppShell](shell_ptr)[0].runtime)


@export
fn shell_store_ptr(shell_ptr: Int64) -> Int64:
    """Return the VNodeStore pointer from an AppShell."""
    return _to_i64(_get[AppShell](shell_ptr)[0].store)


@export
fn shell_eid_ptr(shell_ptr: Int64) -> Int64:
    """Return the ElementIdAllocator pointer from an AppShell."""
    return _to_i64(_get[AppShell](shell_ptr)[0].eid_alloc)


@export
fn shell_mount(
    shell_ptr: Int64, buf_ptr: Int64, capacity: Int32, vnode_index: Int32
) -> Int32:
    """Mount a VNode via the AppShell.  Returns mutation byte length."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var result = _get[AppShell](shell_ptr)[0].mount(
        writer_ptr, UInt32(vnode_index)
    )
    _free_writer(writer_ptr)
    return result


@export
fn shell_diff(
    shell_ptr: Int64,
    buf_ptr: Int64,
    capacity: Int32,
    old_index: Int32,
    new_index: Int32,
) -> Int32:
    """Diff two VNodes via the AppShell.  Returns mutation byte length."""
    var ptr = _get[AppShell](shell_ptr)
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    ptr[0].diff(writer_ptr, UInt32(old_index), UInt32(new_index))
    var result = ptr[0].finalize(writer_ptr)
    _free_writer(writer_ptr)
    return result


# ══════════════════════════════════════════════════════════════════════════════
# DSL Ergonomic Builder Exports (M10.5)
# ══════════════════════════════════════════════════════════════════════════════
#
# WASM-exported test functions for the declarative builder DSL.
# Each function exercises a specific aspect of the DSL and returns 1 for
# pass, 0 for fail.  The JS test harness calls these and asserts the result.
#
# Additionally, "dsl_node_*" and "dsl_vb_*" exports provide low-level
# building blocks that the Mojo test harness can orchestrate directly.


# ── Heap-allocated Node handle ───────────────────────────────────────────────


@always_inline
fn _alloc_node(var val: Node) -> Int64:
    """Heap-allocate a Node and return its address as Int64."""
    return _to_i64(_heap_new(val^))


@always_inline
fn _free_node(addr: Int64):
    """Destroy and free a heap-allocated Node from its Int64 address."""
    _heap_del(_get[Node](addr))


@export
fn dsl_node_text(s: String) -> Int64:
    """Create a text Node on the heap.  Returns a pointer handle."""
    return _alloc_node(text(s))


@export
fn dsl_node_dyn_text(index: Int32) -> Int64:
    """Create a dynamic text Node on the heap."""
    return _alloc_node(dyn_text(Int(index)))


@export
fn dsl_node_dyn_node(index: Int32) -> Int64:
    """Create a dynamic node placeholder on the heap."""
    return _alloc_node(dyn_node(Int(index)))


@export
fn dsl_node_attr(name: String, value: String) -> Int64:
    """Create a static attribute Node on the heap."""
    return _alloc_node(attr(name, value))


@export
fn dsl_node_dyn_attr(index: Int32) -> Int64:
    """Create a dynamic attribute Node on the heap."""
    return _alloc_node(dyn_attr(Int(index)))


@export
fn dsl_node_element(html_tag: Int32) -> Int64:
    """Create an empty element Node on the heap."""
    return _alloc_node(el_empty(UInt8(html_tag)))


@export
fn dsl_node_add_item(parent_ptr: Int64, child_ptr: Int64):
    """Add a child/attr Node to an element Node.

    The child Node is moved out of its heap slot (the child pointer
    becomes invalid after this call).
    """
    var child_val = _get[Node](child_ptr)[0].copy()
    _get[Node](parent_ptr)[0].add_item(child_val^)
    _free_node(child_ptr)


@export
fn dsl_node_destroy(ptr: Int64):
    """Destroy and free a heap-allocated Node."""
    _free_node(ptr)


@export
fn dsl_node_kind(ptr: Int64) -> Int32:
    """Return the kind tag of a Node."""
    return Int32(_get[Node](ptr)[0].kind)


@export
fn dsl_node_tag(ptr: Int64) -> Int32:
    """Return the HTML tag of an element Node."""
    return Int32(_get[Node](ptr)[0].tag)


@export
fn dsl_node_item_count(ptr: Int64) -> Int32:
    """Return the total item count (children + attrs) of an element Node."""
    return Int32(_get[Node](ptr)[0].item_count())


@export
fn dsl_node_child_count(ptr: Int64) -> Int32:
    """Return the child count (excluding attrs) of an element Node."""
    return Int32(_get[Node](ptr)[0].child_count())


@export
fn dsl_node_attr_count(ptr: Int64) -> Int32:
    """Return the attribute count (excluding children) of an element Node."""
    return Int32(_get[Node](ptr)[0].attr_count())


@export
fn dsl_node_dynamic_index(ptr: Int64) -> Int32:
    """Return the dynamic_index of a DYN_TEXT/DYN_NODE/DYN_ATTR Node."""
    return Int32(_get[Node](ptr)[0].dynamic_index)


@export
fn dsl_node_count_nodes(ptr: Int64) -> Int32:
    """Recursively count tree nodes (excluding attrs)."""
    return Int32(count_nodes(_get[Node](ptr)[0]))


@export
fn dsl_node_count_all(ptr: Int64) -> Int32:
    """Recursively count all items (including attrs)."""
    return Int32(count_all_items(_get[Node](ptr)[0]))


@export
fn dsl_node_count_dyn_text(ptr: Int64) -> Int32:
    """Count DYN_TEXT slots in the tree."""
    return Int32(count_dynamic_text_slots(_get[Node](ptr)[0]))


@export
fn dsl_node_count_dyn_node(ptr: Int64) -> Int32:
    """Count DYN_NODE slots in the tree."""
    return Int32(count_dynamic_node_slots(_get[Node](ptr)[0]))


@export
fn dsl_node_count_dyn_attr(ptr: Int64) -> Int32:
    """Count DYN_ATTR slots in the tree."""
    return Int32(count_dynamic_attr_slots(_get[Node](ptr)[0]))


@export
fn dsl_node_count_static_attr(ptr: Int64) -> Int32:
    """Count STATIC_ATTR nodes in the tree."""
    return Int32(count_static_attr_nodes(_get[Node](ptr)[0]))


# ── to_template via DSL ──────────────────────────────────────────────────────


@export
fn dsl_to_template(node_ptr: Int64, name: String, rt_ptr: Int64) -> Int32:
    """Convert a Node tree to a Template and register it.

    Args:
        node_ptr: Pointer to the root Node (consumed — freed after use).
        name: Template name for registration.
        rt_ptr: Pointer to the Runtime (owns TemplateRegistry).

    Returns:
        The template ID (UInt32 as Int32).
    """
    var template = to_template(_get[Node](node_ptr)[0], name)
    var tmpl_id = _get[Runtime](rt_ptr)[0].templates.register(template^)
    _free_node(node_ptr)
    return Int32(tmpl_id)


# ── VNodeBuilder WASM exports ────────────────────────────────────────────────


@export
fn dsl_vb_create(tmpl_id: Int32, store_ptr: Int64) -> Int64:
    """Create a VNodeBuilder on the heap.

    Returns a pointer handle to the VNodeBuilder.
    """
    return _to_i64(
        _heap_new(VNodeBuilder(UInt32(tmpl_id), _get[VNodeStore](store_ptr)))
    )


@export
fn dsl_vb_create_keyed(tmpl_id: Int32, key: String, store_ptr: Int64) -> Int64:
    """Create a keyed VNodeBuilder on the heap."""
    return _to_i64(
        _heap_new(
            VNodeBuilder(UInt32(tmpl_id), key, _get[VNodeStore](store_ptr))
        )
    )


@export
fn dsl_vb_destroy(ptr: Int64):
    """Destroy and free a heap-allocated VNodeBuilder."""
    _heap_del(_get[VNodeBuilder](ptr))


@export
fn dsl_vb_add_dyn_text(ptr: Int64, value: String):
    """Add a dynamic text node via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_text(value)


@export
fn dsl_vb_add_dyn_placeholder(ptr: Int64):
    """Add a dynamic placeholder via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_placeholder()


@export
fn dsl_vb_add_dyn_event(ptr: Int64, event_name: String, handler_id: Int32):
    """Add a dynamic event handler via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_event(event_name, UInt32(handler_id))


@export
fn dsl_vb_add_dyn_text_attr(ptr: Int64, name: String, value: String):
    """Add a dynamic text attribute via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_text_attr(name, value)


@export
fn dsl_vb_add_dyn_int_attr(ptr: Int64, name: String, value: Int64):
    """Add a dynamic integer attribute via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_int_attr(name, value)


@export
fn dsl_vb_add_dyn_bool_attr(ptr: Int64, name: String, value: Int32):
    """Add a dynamic boolean attribute via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_bool_attr(name, value != 0)


@export
fn dsl_vb_add_dyn_none_attr(ptr: Int64, name: String):
    """Add a dynamic none/removal attribute via the VNodeBuilder."""
    _get[VNodeBuilder](ptr)[0].add_dyn_none_attr(name)


@export
fn dsl_vb_index(ptr: Int64) -> Int32:
    """Return the VNode index from the VNodeBuilder."""
    return Int32(_get[VNodeBuilder](ptr)[0].index())


# ── Self-contained DSL tests ─────────────────────────────────────────────────
#
# Thin @export wrappers delegating to vdom.dsl_tests (M10.13).
# Each returns 1 (pass) or 0 (fail).


@export
fn dsl_test_text_node() -> Int32:
    return _dsl_test_text_node()


@export
fn dsl_test_dyn_text_node() -> Int32:
    return _dsl_test_dyn_text_node()


@export
fn dsl_test_dyn_node_slot() -> Int32:
    return _dsl_test_dyn_node_slot()


@export
fn dsl_test_static_attr() -> Int32:
    return _dsl_test_static_attr()


@export
fn dsl_test_dyn_attr() -> Int32:
    return _dsl_test_dyn_attr()


@export
fn dsl_test_empty_element() -> Int32:
    return _dsl_test_empty_element()


@export
fn dsl_test_element_with_children() -> Int32:
    return _dsl_test_element_with_children()


@export
fn dsl_test_element_with_attrs() -> Int32:
    return _dsl_test_element_with_attrs()


@export
fn dsl_test_element_mixed() -> Int32:
    return _dsl_test_element_mixed()


@export
fn dsl_test_nested_elements() -> Int32:
    return _dsl_test_nested_elements()


@export
fn dsl_test_counter_template() -> Int32:
    return _dsl_test_counter_template()


@export
fn dsl_test_to_template_simple() -> Int32:
    return _dsl_test_to_template_simple()


@export
fn dsl_test_to_template_attrs() -> Int32:
    return _dsl_test_to_template_attrs()


@export
fn dsl_test_to_template_multi_root() -> Int32:
    return _dsl_test_to_template_multi_root()


@export
fn dsl_test_vnode_builder() -> Int32:
    return _dsl_test_vnode_builder()


@export
fn dsl_test_vnode_builder_keyed() -> Int32:
    return _dsl_test_vnode_builder_keyed()


@export
fn dsl_test_all_tag_helpers() -> Int32:
    return _dsl_test_all_tag_helpers()


@export
fn dsl_test_count_utilities() -> Int32:
    return _dsl_test_count_utilities()


@export
fn dsl_test_template_equivalence() -> Int32:
    return _dsl_test_template_equivalence()


# ── Phase 20 — M20.3: oninput_set_string / onchange_set_string ───────────────


@export
fn dsl_test_oninput_set_string_node() -> Int32:
    return _dsl_test_oninput_set_string_node()


@export
fn dsl_test_onchange_set_string_node() -> Int32:
    return _dsl_test_onchange_set_string_node()


@export
fn dsl_test_oninput_in_element() -> Int32:
    return _dsl_test_oninput_in_element()


# ── Phase 20 — M20.4: bind_value / bind_attr ─────────────────────────────────


@export
fn dsl_test_bind_value_node() -> Int32:
    return _dsl_test_bind_value_node()


@export
fn dsl_test_bind_attr_node() -> Int32:
    return _dsl_test_bind_attr_node()


@export
fn dsl_test_bind_value_in_element() -> Int32:
    return _dsl_test_bind_value_in_element()


@export
fn dsl_test_two_way_binding_element() -> Int32:
    return _dsl_test_two_way_binding_element()


@export
fn dsl_test_bind_value_to_template() -> Int32:
    return _dsl_test_bind_value_to_template()


@export
fn dsl_test_two_way_to_template() -> Int32:
    return _dsl_test_two_way_to_template()


# ── Phase 20 — M20.5: onclick_custom tests ──────────────────────────────────


@export
fn dsl_test_onclick_custom_node() -> Int32:
    return _dsl_test_onclick_custom_node()


@export
fn dsl_test_onclick_custom_in_element() -> Int32:
    return _dsl_test_onclick_custom_in_element()


@export
fn dsl_test_onclick_custom_with_binding() -> Int32:
    return _dsl_test_onclick_custom_with_binding()


@export
fn dsl_test_onkeydown_enter_custom_node() -> Int32:
    return _dsl_test_onkeydown_enter_custom_node()


@export
fn dsl_test_onkeydown_enter_custom_in_element() -> Int32:
    return _dsl_test_onkeydown_enter_custom_in_element()


@export
fn dsl_test_onkeydown_enter_custom_with_binding() -> Int32:
    return _dsl_test_onkeydown_enter_custom_with_binding()


# ══════════════════════════════════════════════════════════════════════════════
# Original mojo-wasm PoC Exports — Arithmetic, String, Algorithm Functions
# ══════════════════════════════════════════════════════════════════════════════
#
# Thin @export wrappers calling into poc/ package modules.


# ── Add ──────────────────────────────────────────────────────────────────────


@export
fn add_int32(x: Int32, y: Int32) -> Int32:
    return x + y


@export
fn add_int64(x: Int64, y: Int64) -> Int64:
    return x + y


@export
fn add_float32(x: Float32, y: Float32) -> Float32:
    return x + y


@export
fn add_float64(x: Float64, y: Float64) -> Float64:
    return x + y


# ── Subtract ─────────────────────────────────────────────────────────────────


@export
fn sub_int32(x: Int32, y: Int32) -> Int32:
    return x - y


@export
fn sub_int64(x: Int64, y: Int64) -> Int64:
    return x - y


@export
fn sub_float32(x: Float32, y: Float32) -> Float32:
    return x - y


@export
fn sub_float64(x: Float64, y: Float64) -> Float64:
    return x - y


# ── Multiply ─────────────────────────────────────────────────────────────────


@export
fn mul_int32(x: Int32, y: Int32) -> Int32:
    return x * y


@export
fn mul_int64(x: Int64, y: Int64) -> Int64:
    return x * y


@export
fn mul_float32(x: Float32, y: Float32) -> Float32:
    return x * y


@export
fn mul_float64(x: Float64, y: Float64) -> Float64:
    return x * y


# ── Division ─────────────────────────────────────────────────────────────────


@export
fn div_int32(x: Int32, y: Int32) -> Int32:
    return x // y


@export
fn div_int64(x: Int64, y: Int64) -> Int64:
    return x // y


@export
fn div_float32(x: Float32, y: Float32) -> Float32:
    return x / y


@export
fn div_float64(x: Float64, y: Float64) -> Float64:
    return x / y


# ── Modulo ───────────────────────────────────────────────────────────────────


@export
fn mod_int32(x: Int32, y: Int32) -> Int32:
    return x % y


@export
fn mod_int64(x: Int64, y: Int64) -> Int64:
    return x % y


# ── Power ────────────────────────────────────────────────────────────────────


@export
fn pow_int32(x: Int32) -> Int32:
    return x**x


@export
fn pow_int64(x: Int64) -> Int64:
    return x**x


@export
fn pow_float32(x: Float32) -> Float32:
    return x**x


@export
fn pow_float64(x: Float64) -> Float64:
    return x**x


# ── Negate ───────────────────────────────────────────────────────────────────


@export
fn neg_int32(x: Int32) -> Int32:
    return -x


@export
fn neg_int64(x: Int64) -> Int64:
    return -x


@export
fn neg_float32(x: Float32) -> Float32:
    return -x


@export
fn neg_float64(x: Float64) -> Float64:
    return -x


# ── Absolute value ───────────────────────────────────────────────────────────


@export
fn abs_int32(x: Int32) -> Int32:
    if x < 0:
        return -x
    return x


@export
fn abs_int64(x: Int64) -> Int64:
    if x < 0:
        return -x
    return x


@export
fn abs_float32(x: Float32) -> Float32:
    if x < 0:
        return -x
    return x


@export
fn abs_float64(x: Float64) -> Float64:
    if x < 0:
        return -x
    return x


# ── Min / Max ────────────────────────────────────────────────────────────────


@export
fn min_int32(x: Int32, y: Int32) -> Int32:
    if x < y:
        return x
    return y


@export
fn max_int32(x: Int32, y: Int32) -> Int32:
    if x > y:
        return x
    return y


@export
fn min_int64(x: Int64, y: Int64) -> Int64:
    if x < y:
        return x
    return y


@export
fn max_int64(x: Int64, y: Int64) -> Int64:
    if x > y:
        return x
    return y


@export
fn min_float64(x: Float64, y: Float64) -> Float64:
    if x < y:
        return x
    return y


@export
fn max_float64(x: Float64, y: Float64) -> Float64:
    if x > y:
        return x
    return y


# ── Clamp ────────────────────────────────────────────────────────────────────


@export
fn clamp_int32(x: Int32, lo: Int32, hi: Int32) -> Int32:
    if x < lo:
        return lo
    if x > hi:
        return hi
    return x


@export
fn clamp_float64(x: Float64, lo: Float64, hi: Float64) -> Float64:
    if x < lo:
        return lo
    if x > hi:
        return hi
    return x


# ── Bitwise operations ──────────────────────────────────────────────────────


@export
fn bitand_int32(x: Int32, y: Int32) -> Int32:
    return x & y


@export
fn bitor_int32(x: Int32, y: Int32) -> Int32:
    return x | y


@export
fn bitxor_int32(x: Int32, y: Int32) -> Int32:
    return x ^ y


@export
fn bitnot_int32(x: Int32) -> Int32:
    return ~x


@export
fn shl_int32(x: Int32, y: Int32) -> Int32:
    return x << y


@export
fn shr_int32(x: Int32, y: Int32) -> Int32:
    return x >> y


# ── Boolean / comparison ─────────────────────────────────────────────────────


@export
fn eq_int32(x: Int32, y: Int32) -> Bool:
    return x == y


@export
fn ne_int32(x: Int32, y: Int32) -> Bool:
    return x != y


@export
fn lt_int32(x: Int32, y: Int32) -> Bool:
    return x < y


@export
fn le_int32(x: Int32, y: Int32) -> Bool:
    return x <= y


@export
fn gt_int32(x: Int32, y: Int32) -> Bool:
    return x > y


@export
fn ge_int32(x: Int32, y: Int32) -> Bool:
    return x >= y


@export
fn bool_and(x: Bool, y: Bool) -> Bool:
    return x and y


@export
fn bool_or(x: Bool, y: Bool) -> Bool:
    return x or y


@export
fn bool_not(x: Bool) -> Bool:
    return not x


# ── Fibonacci (iterative) ───────────────────────────────────────────────────


@export
fn fib_int32(n: Int32) -> Int32:
    if n <= 0:
        return 0
    if n == 1:
        return 1
    var a: Int32 = 0
    var b: Int32 = 1
    for _ in range(2, Int(n) + 1):
        var tmp = a + b
        a = b
        b = tmp
    return b


@export
fn fib_int64(n: Int64) -> Int64:
    if n <= 0:
        return 0
    if n == 1:
        return 1
    var a: Int64 = 0
    var b: Int64 = 1
    for _ in range(2, Int(n) + 1):
        var tmp = a + b
        a = b
        b = tmp
    return b


# ── Factorial (iterative) ───────────────────────────────────────────────────


@export
fn factorial_int32(n: Int32) -> Int32:
    if n <= 1:
        return 1
    var result: Int32 = 1
    for i in range(2, Int(n) + 1):
        result *= Int32(i)
    return result


@export
fn factorial_int64(n: Int64) -> Int64:
    if n <= 1:
        return 1
    var result: Int64 = 1
    for i in range(2, Int(n) + 1):
        result *= Int64(i)
    return result


# ── GCD (Euclidean algorithm) ────────────────────────────────────────────────


@export
fn gcd_int32(x: Int32, y: Int32) -> Int32:
    var a = x
    var b = y
    if a < 0:
        a = -a
    if b < 0:
        b = -b
    while b != 0:
        var tmp = b
        b = a % b
        a = tmp
    return a


# ── Identity / passthrough ──────────────────────────────────────────────────


@export
fn identity_int32(x: Int32) -> Int32:
    return x


@export
fn identity_int64(x: Int64) -> Int64:
    return x


@export
fn identity_float32(x: Float32) -> Float32:
    return x


@export
fn identity_float64(x: Float64) -> Float64:
    return x


# ── Print ────────────────────────────────────────────────────────────────────


@export
fn print_int32():
    comptime int32: Int32 = 3
    print(int32)


@export
fn print_int64():
    comptime int64: Int64 = 3
    print(2)


@export
fn print_float32():
    comptime float32: Float32 = 3.0
    print(float32)


@export
fn print_float64():
    comptime float64: Float64 = 3.0
    print(float64)


@export
fn print_static_string():
    print("print-static-string")


@export
fn print_input_string(input: String):
    print(input)


# ── String I/O ───────────────────────────────────────────────────────────────


@export
fn return_input_string(x: String) -> String:
    return x


@export
fn return_static_string() -> String:
    return "return-static-string"


@export
fn string_length(x: String) -> Int64:
    return Int64(len(x))


@export
fn string_concat(x: String, y: String) -> String:
    return x + y


@export
fn string_repeat(x: String, n: Int32) -> String:
    var result = String("")
    for _ in range(Int(n)):
        result += x
    return result


@export
fn string_eq(x: String, y: String) -> Bool:
    return x == y


# ══════════════════════════════════════════════════════════════════════════════
# Phase 29 — Component Composition (ChildCounterApp)
#   Struct + lifecycle: src/apps/child_counter.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── ChildCounter WASM exports ───────────────────────────────────────────────


@export
fn cc_init() -> Int64:
    """Initialize the child-counter app.  Returns app pointer."""
    return _to_i64(_cc_init())


@export
fn cc_destroy(app_ptr: Int64):
    """Destroy the child-counter app."""
    _cc_destroy(_get[ChildCounterApp](app_ptr))


@export
fn cc_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount).  Returns mutation byte length."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _cc_rebuild(_get[ChildCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn cc_handle_event(
    app_ptr: Int64, handler_id: Int32, event_type: Int32
) -> Int32:
    """Dispatch an event.  Returns 1 if action executed."""
    return _b2i(
        _cc_handle_event(
            _get[ChildCounterApp](app_ptr)[0],
            UInt32(handler_id),
            UInt8(event_type),
        )
    )


@export
fn cc_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _cc_flush(_get[ChildCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn cc_count_value(app_ptr: Int64) -> Int32:
    """Peek the current count signal value."""
    return _get[ChildCounterApp](app_ptr)[0].count.peek()


@export
fn cc_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment handler ID (parent view_events[0])."""
    var events = _get[ChildCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 0:
        return Int32(events[0].handler_id)
    return -1


@export
fn cc_decr_handler(app_ptr: Int64) -> Int32:
    """Return the decrement handler ID (parent view_events[1])."""
    var events = _get[ChildCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 1:
        return Int32(events[1].handler_id)
    return -1


@export
fn cc_child_scope_id(app_ptr: Int64) -> Int32:
    """Return the child component's scope ID."""
    return Int32(_get[ChildCounterApp](app_ptr)[0].child.scope_id)


@export
fn cc_child_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the child component's template ID."""
    return Int32(_get[ChildCounterApp](app_ptr)[0].child.template_id)


@export
fn cc_child_event_count(app_ptr: Int64) -> Int32:
    """Return the number of event bindings on the child."""
    return Int32(_get[ChildCounterApp](app_ptr)[0].child.event_count())


@export
fn cc_child_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if the child has been rendered, 0 otherwise."""
    return _b2i(_get[ChildCounterApp](app_ptr)[0].child.has_rendered())


@export
fn cc_child_is_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the child is mounted in the DOM, 0 otherwise."""
    return _b2i(_get[ChildCounterApp](app_ptr)[0].child.is_mounted())


@export
fn cc_parent_scope_id(app_ptr: Int64) -> Int32:
    """Return the parent root scope ID."""
    return Int32(_get[ChildCounterApp](app_ptr)[0].ctx.scope_id)


@export
fn cc_parent_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the parent template ID."""
    return Int32(_get[ChildCounterApp](app_ptr)[0].ctx.template_id)


@export
fn cc_handler_count(app_ptr: Int64) -> Int32:
    """Return the total number of registered handlers."""
    return Int32(_get[ChildCounterApp](app_ptr)[0].ctx.handler_count())


# ══════════════════════════════════════════════════════════════════════════════
# Phase 30 — Client-Side Routing (MultiViewApp)
# ══════════════════════════════════════════════════════════════════════════════


@export
fn mv_init() -> Int64:
    """Initialize the multi-view app.  Returns app pointer."""
    return gui_app_init[MultiViewApp]()


@export
fn mv_destroy(app_ptr: Int64):
    """Destroy the multi-view app."""
    gui_app_destroy[MultiViewApp](app_ptr)


@export
fn mv_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount).  Returns mutation byte length."""
    return gui_app_mount[MultiViewApp](app_ptr, buf_ptr, capacity)


@export
fn mv_handle_event(
    app_ptr: Int64, handler_id: Int32, event_type: Int32
) -> Int32:
    """Dispatch an event.  Returns 1 if action executed."""
    return gui_app_handle_event[MultiViewApp](app_ptr, handler_id, event_type)


@export
fn mv_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0."""
    return gui_app_flush[MultiViewApp](app_ptr, buf_ptr, capacity)


@export
fn mv_navigate(app_ptr: Int64, path: String) -> Int32:
    """Navigate to a URL path.  Returns 1 if route matched, 0 otherwise.

    Call mv_flush() after this to apply DOM mutations.
    """
    return _b2i(_get[MultiViewApp](app_ptr)[0].navigate(path))


@export
fn mv_current_path(app_ptr: Int64) -> String:
    """Return the currently active URL path."""
    return _get[MultiViewApp](app_ptr)[0].router.current_path


@export
fn mv_current_branch(app_ptr: Int64) -> Int32:
    """Return the currently active branch tag (0=counter, 1=todo, 255=none)."""
    return Int32(_get[MultiViewApp](app_ptr)[0].router.current)


@export
fn mv_route_count(app_ptr: Int64) -> Int32:
    """Return the number of registered routes."""
    return Int32(_get[MultiViewApp](app_ptr)[0].router.route_count())


@export
fn mv_count_value(app_ptr: Int64) -> Int32:
    """Peek the counter view's count signal value."""
    return _get[MultiViewApp](app_ptr)[0].count.peek()


@export
fn mv_todo_count(app_ptr: Int64) -> Int32:
    """Peek the todo view's item count signal value."""
    return _get[MultiViewApp](app_ptr)[0].todo_count.peek()


@export
fn mv_nav_counter_handler(app_ptr: Int64) -> Int32:
    """Return the Counter nav button handler ID."""
    return Int32(_get[MultiViewApp](app_ptr)[0].nav_counter_handler)


@export
fn mv_nav_todo_handler(app_ptr: Int64) -> Int32:
    """Return the Todo nav button handler ID."""
    return Int32(_get[MultiViewApp](app_ptr)[0].nav_todo_handler)


@export
fn mv_todo_add_handler(app_ptr: Int64) -> Int32:
    """Return the Todo Add button handler ID."""
    return Int32(_get[MultiViewApp](app_ptr)[0].todo_add_handler)


@export
fn mv_counter_incr_handler(app_ptr: Int64) -> Int32:
    """Return the counter +1 handler ID (from view_events on counter tmpl)."""
    _ = _get[MultiViewApp](app_ptr)
    # Counter view events are registered after nav events (0=nav_counter, 1=nav_todo)
    # The counter template uses onclick_add/onclick_sub which are signal handlers,
    # not custom handlers — they're registered by the template infrastructure.
    # We expose the nav handlers instead; counter +/- go through dispatch_event.
    return -1


@export
fn mv_has_dirty(app_ptr: Int64) -> Int32:
    """Check if the multi-view app has dirty scopes.  Returns 1 or 0."""
    return _b2i(_get[MultiViewApp](app_ptr)[0].ctx.has_dirty())


@export
fn mv_router_dirty(app_ptr: Int64) -> Int32:
    """Check if the router has a pending route change.  Returns 1 or 0."""
    if _get[MultiViewApp](app_ptr)[0].router.dirty:
        return 1
    return 0


@export
fn mv_cond_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the router's conditional slot is mounted, 0 otherwise."""
    if _get[MultiViewApp](app_ptr)[0].router.slot.mounted:
        return 1
    return 0


# ══════════════════════════════════════════════════════════════════════════════
# Phase 31.1 — Context Test App (ComponentContext provide/consume surface)
#   Struct + lifecycle: src/apps/context_test.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── ContextTestApp WASM exports ─────────────────────────────────────────────


@export
fn cta_init() -> Int64:
    """Initialize the context test app.  Returns app pointer."""
    return _to_i64(_cta_init())


@export
fn cta_destroy(app_ptr: Int64):
    """Destroy the context test app."""
    _cta_destroy(_get[ContextTestApp](app_ptr))


@export
fn cta_root_scope_id(app_ptr: Int64) -> Int32:
    """Return the root scope ID."""
    return Int32(_get[ContextTestApp](app_ptr)[0].ctx.scope_id)


@export
fn cta_child_scope_id(app_ptr: Int64) -> Int32:
    """Return the child scope ID."""
    return Int32(_get[ContextTestApp](app_ptr)[0].child_scope_id)


@export
fn cta_count_value(app_ptr: Int64) -> Int32:
    """Peek the current count signal value."""
    return _get[ContextTestApp](app_ptr)[0].count.peek()


@export
fn cta_count_signal_key(app_ptr: Int64) -> Int32:
    """Return the count signal's internal key."""
    return Int32(_get[ContextTestApp](app_ptr)[0].count.key)


@export
fn cta_provide_context(app_ptr: Int64, key: Int32, value: Int32):
    """Provide a context value at the root scope via ComponentContext."""
    _get[ContextTestApp](app_ptr)[0].ctx.provide_context(UInt32(key), value)


@export
fn cta_consume_context(app_ptr: Int64, key: Int32) -> Int32:
    """Consume a context value from the root scope.  Returns 0 if not found."""
    return _get[ContextTestApp](app_ptr)[0].ctx.consume_context(UInt32(key))[1]


@export
fn cta_has_context(app_ptr: Int64, key: Int32) -> Int32:
    """Check whether a context value is reachable.  Returns 1 or 0."""
    return _b2i(_get[ContextTestApp](app_ptr)[0].ctx.has_context(UInt32(key)))


@export
fn cta_consume_from_child(app_ptr: Int64, key: Int32) -> Int32:
    """Consume a context value starting from the child scope (walks up to parent).

    Returns 0 if not found.
    """
    var app = _get[ContextTestApp](app_ptr)
    return (
        app[0]
        .ctx.shell.runtime[0]
        .scopes.consume_context(app[0].child_scope_id, UInt32(key))[1]
    )


@export
fn cta_consume_found_from_child(app_ptr: Int64, key: Int32) -> Int32:
    """Check whether a context value is reachable from the child scope.

    Returns 1 if found, 0 if not.
    """
    var app = _get[ContextTestApp](app_ptr)
    return _b2i(
        app[0]
        .ctx.shell.runtime[0]
        .scopes.consume_context(app[0].child_scope_id, UInt32(key))[0]
    )


@export
fn cta_provide_signal_i32(app_ptr: Int64, ctx_key: Int32):
    """Provide the count signal at the root scope via provide_signal_i32."""
    var app = _get[ContextTestApp](app_ptr)
    app[0].ctx.provide_signal_i32(UInt32(ctx_key), app[0].count)


@export
fn cta_consume_signal_i32_from_child(app_ptr: Int64, ctx_key: Int32) -> Int32:
    """Consume a SignalI32 from the child scope via context, return its peek value.

    Reconstructs the signal handle from context and reads the value.
    Returns the signal value (not the signal key).
    """
    var app = _get[ContextTestApp](app_ptr)
    # Walk up from child scope to find the signal key
    var result = (
        app[0]
        .ctx.shell.runtime[0]
        .scopes.consume_context(app[0].child_scope_id, UInt32(ctx_key))
    )
    if not result[0]:
        return -9999  # sentinel: context key not found
    var signal = _SignalI32(UInt32(result[1]), app[0].ctx.shell.runtime)
    return signal.peek()


@export
fn cta_write_signal_via_child(app_ptr: Int64, ctx_key: Int32, value: Int32):
    """Consume a SignalI32 from child scope context, then write to it.

    This tests that writing a consumed signal marks the parent scope dirty.
    """
    var app = _get[ContextTestApp](app_ptr)
    var result = (
        app[0]
        .ctx.shell.runtime[0]
        .scopes.consume_context(app[0].child_scope_id, UInt32(ctx_key))
    )
    if result[0]:
        var signal = _SignalI32(UInt32(result[1]), app[0].ctx.shell.runtime)
        signal.set(value)


@export
fn cta_has_dirty(app_ptr: Int64) -> Int32:
    """Check if the app has dirty scopes.  Returns 1 or 0."""
    return _b2i(_get[ContextTestApp](app_ptr)[0].ctx.has_dirty())


@export
fn cta_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[ContextTestApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 31.2 — ChildContextTestApp (ChildComponentContext test harness)
#   Struct + lifecycle: src/apps/child_context_test.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── ChildContextTestApp WASM exports ────────────────────────────────────────


@export
fn cct_init() -> Int64:
    """Initialize the child-context test app.  Returns app pointer."""
    return _to_i64(_cct_init())


@export
fn cct_destroy(app_ptr: Int64):
    """Destroy the child-context test app."""
    _cct_destroy(_get[ChildContextTestApp](app_ptr))


@export
fn cct_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount).  Returns mutation byte length."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _cct_rebuild(_get[ChildContextTestApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn cct_handle_event(
    app_ptr: Int64,
    handler_id: Int32,
    event_type: Int32,
) -> Int32:
    """Dispatch an event.  Returns 1 if action executed."""
    return _b2i(
        _cct_handle_event(
            _get[ChildContextTestApp](app_ptr)[0],
            UInt32(handler_id),
            UInt8(event_type),
        )
    )


@export
fn cct_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _cct_flush(_get[ChildContextTestApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn cct_count_value(app_ptr: Int64) -> Int32:
    """Peek the current count signal value."""
    return _get[ChildContextTestApp](app_ptr)[0].count.peek()


@export
fn cct_show_hex(app_ptr: Int64) -> Int32:
    """Return 1 if show_hex is True, 0 otherwise."""
    return _b2i(_get[ChildContextTestApp](app_ptr)[0].child_show_hex.get())


@export
fn cct_parent_scope_id(app_ptr: Int64) -> Int32:
    """Return the parent scope ID."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].ctx.scope_id)


@export
fn cct_child_scope_id(app_ptr: Int64) -> Int32:
    """Return the child scope ID."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].child_ctx.scope_id)


@export
fn cct_child_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the child template ID."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].child_ctx.template_id())


@export
fn cct_parent_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the parent template ID."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].ctx.template_id)


@export
fn cct_child_is_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if child is mounted, 0 otherwise."""
    return _b2i(_get[ChildContextTestApp](app_ptr)[0].child_ctx.is_mounted())


@export
fn cct_child_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if child scope is dirty, 0 otherwise."""
    return _b2i(_get[ChildContextTestApp](app_ptr)[0].child_ctx.is_dirty())


@export
fn cct_parent_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if parent scope is in the dirty queue, 0 otherwise."""
    var app = _get[ChildContextTestApp](app_ptr)
    var scope_id = app[0].ctx.scope_id
    for i in range(len(app[0].ctx.shell.runtime[0].dirty_scopes)):
        if app[0].ctx.shell.runtime[0].dirty_scopes[i] == scope_id:
            return 1
    return 0


@export
fn cct_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty, 0 otherwise."""
    var app = _get[ChildContextTestApp](app_ptr)
    if app[0].ctx.has_dirty():
        return 1
    if app[0].child_ctx.is_dirty():
        return 1
    return 0


@export
fn cct_handler_count(app_ptr: Int64) -> Int32:
    """Return the total number of registered handlers."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].ctx.handler_count())


@export
fn cct_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment handler ID (parent view_events[0])."""
    var events = _get[ChildContextTestApp](app_ptr)[0].ctx.view_events()
    if len(events) > 0:
        return Int32(events[0].handler_id)
    return -1


@export
fn cct_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[ChildContextTestApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn cct_child_count_signal_key(app_ptr: Int64) -> Int32:
    """Return the child's consumed count signal key."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].child_count.key)


@export
fn cct_parent_count_signal_key(app_ptr: Int64) -> Int32:
    """Return the parent's count signal key."""
    return Int32(_get[ChildContextTestApp](app_ptr)[0].count.key)


@export
fn cct_toggle_hex(app_ptr: Int64):
    """Toggle the show_hex signal (child-owned)."""
    _get[ChildContextTestApp](app_ptr)[0].child_show_hex.toggle()


@export
fn cct_set_count(app_ptr: Int64, value: Int32):
    """Set the count signal directly (for testing)."""
    _get[ChildContextTestApp](app_ptr)[0].count.set(value)


@export
fn cct_child_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if child has rendered, 0 otherwise."""
    return _b2i(_get[ChildContextTestApp](app_ptr)[0].child_ctx.has_rendered())


# ══════════════════════════════════════════════════════════════════════════════
# Phase 31.3 — PropsCounterApp (self-rendering child with props)
#   Struct + lifecycle: src/apps/props_counter.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── PropsCounterApp WASM exports ────────────────────────────────────────────


@export
fn pc_init() -> Int64:
    """Initialize the props-counter app.  Returns app pointer."""
    return _to_i64(_pc_init())


@export
fn pc_destroy(app_ptr: Int64):
    """Destroy the props-counter app."""
    _pc_destroy(_get[PropsCounterApp](app_ptr))


@export
fn pc_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount).  Returns mutation byte length."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _pc_rebuild(_get[PropsCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn pc_handle_event(
    app_ptr: Int64,
    handler_id: Int32,
    event_type: Int32,
) -> Int32:
    """Dispatch an event.  Returns 1 if action executed."""
    return _b2i(
        _pc_handle_event(
            _get[PropsCounterApp](app_ptr)[0],
            UInt32(handler_id),
            UInt8(event_type),
        )
    )


@export
fn pc_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _pc_flush(_get[PropsCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn pc_count_value(app_ptr: Int64) -> Int32:
    """Peek the current count signal value."""
    return _get[PropsCounterApp](app_ptr)[0].count.peek()


@export
fn pc_show_hex(app_ptr: Int64) -> Int32:
    """Return 1 if show_hex is True, 0 otherwise."""
    return _b2i(_get[PropsCounterApp](app_ptr)[0].display.show_hex.get())


@export
fn pc_parent_scope_id(app_ptr: Int64) -> Int32:
    """Return the parent scope ID."""
    return Int32(_get[PropsCounterApp](app_ptr)[0].ctx.scope_id)


@export
fn pc_child_scope_id(app_ptr: Int64) -> Int32:
    """Return the child scope ID."""
    return Int32(_get[PropsCounterApp](app_ptr)[0].display.child_ctx.scope_id)


@export
fn pc_parent_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the parent template ID."""
    return Int32(_get[PropsCounterApp](app_ptr)[0].ctx.template_id)


@export
fn pc_child_tmpl_id(app_ptr: Int64) -> Int32:
    """Return the child template ID."""
    return Int32(
        _get[PropsCounterApp](app_ptr)[0].display.child_ctx.template_id()
    )


@export
fn pc_child_is_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if child is mounted, 0 otherwise."""
    return _b2i(
        _get[PropsCounterApp](app_ptr)[0].display.child_ctx.is_mounted()
    )


@export
fn pc_child_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if child scope is dirty, 0 otherwise."""
    return _b2i(_get[PropsCounterApp](app_ptr)[0].display.child_ctx.is_dirty())


@export
fn pc_parent_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if parent scope is in the dirty queue, 0 otherwise."""
    var app = _get[PropsCounterApp](app_ptr)
    var scope_id = app[0].ctx.scope_id
    for i in range(len(app[0].ctx.shell.runtime[0].dirty_scopes)):
        if app[0].ctx.shell.runtime[0].dirty_scopes[i] == scope_id:
            return 1
    return 0


@export
fn pc_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty, 0 otherwise."""
    var app = _get[PropsCounterApp](app_ptr)
    if app[0].ctx.has_dirty():
        return 1
    if app[0].display.child_ctx.is_dirty():
        return 1
    return 0


@export
fn pc_handler_count(app_ptr: Int64) -> Int32:
    """Return the total number of registered handlers."""
    return Int32(_get[PropsCounterApp](app_ptr)[0].ctx.handler_count())


@export
fn pc_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment handler ID (parent view_events[0])."""
    var events = _get[PropsCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 0:
        return Int32(events[0].handler_id)
    return -1


@export
fn pc_decr_handler(app_ptr: Int64) -> Int32:
    """Return the decrement handler ID (parent view_events[1])."""
    var events = _get[PropsCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 1:
        return Int32(events[1].handler_id)
    return -1


@export
fn pc_toggle_handler(app_ptr: Int64) -> Int32:
    """Return the toggle hex handler ID (child event_bindings[0])."""
    var app = _get[PropsCounterApp](app_ptr)
    if app[0].display.child_ctx.event_count() > 0:
        return Int32(app[0].display.child_ctx.event_handler_id(0))
    return -1


@export
fn pc_child_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if child has rendered, 0 otherwise."""
    return _b2i(
        _get[PropsCounterApp](app_ptr)[0].display.child_ctx.has_rendered()
    )


@export
fn pc_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[PropsCounterApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 31.4 — ThemeCounterApp (shared context + cross-component tests)
#   Struct + lifecycle: src/apps/theme_counter.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── ThemeCounterApp WASM exports ─────────────────────────────────────────────


@export
fn tc_init() -> Int64:
    """Initialize the theme-counter app.  Returns app pointer."""
    return _to_i64(_tc_init())


@export
fn tc_destroy(app_ptr: Int64):
    """Destroy the theme-counter app."""
    _tc_destroy(_get[ThemeCounterApp](app_ptr))


@export
fn tc_rebuild(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Initial render (mount).  Returns mutation byte length."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _tc_rebuild(_get[ThemeCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn tc_handle_event(
    app_ptr: Int64,
    handler_id: Int32,
    event_type: Int32,
) -> Int32:
    """Dispatch an event.  Returns 1 if action executed."""
    return _b2i(
        _tc_handle_event(
            _get[ThemeCounterApp](app_ptr)[0],
            UInt32(handler_id),
            UInt8(event_type),
        )
    )


@export
fn tc_flush(app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates.  Returns mutation byte length, or 0."""
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _tc_flush(_get[ThemeCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return offset


@export
fn tc_count_value(app_ptr: Int64) -> Int32:
    """Peek the current count signal value."""
    return _get[ThemeCounterApp](app_ptr)[0].count.peek()


@export
fn tc_theme_is_dark(app_ptr: Int64) -> Int32:
    """Return 1 if theme is dark, 0 if light."""
    return _b2i(_get[ThemeCounterApp](app_ptr)[0].theme.get())


@export
fn tc_on_reset_value(app_ptr: Int64) -> Int32:
    """Return the on_reset callback signal value."""
    return _get[ThemeCounterApp](app_ptr)[0].on_reset.peek()


@export
fn tc_counter_scope_id(app_ptr: Int64) -> Int32:
    """Return the counter child scope ID."""
    return Int32(
        _get[ThemeCounterApp](app_ptr)[0].counter_child.child_ctx.scope_id
    )


@export
fn tc_summary_scope_id(app_ptr: Int64) -> Int32:
    """Return the summary child scope ID."""
    return Int32(
        _get[ThemeCounterApp](app_ptr)[0].summary_child.child_ctx.scope_id
    )


@export
fn tc_parent_scope_id(app_ptr: Int64) -> Int32:
    """Return the parent scope ID."""
    return Int32(_get[ThemeCounterApp](app_ptr)[0].ctx.scope_id)


@export
fn tc_counter_is_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if counter child is mounted, 0 otherwise."""
    return _b2i(
        _get[ThemeCounterApp](app_ptr)[0].counter_child.child_ctx.is_mounted()
    )


@export
fn tc_summary_is_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if summary child is mounted, 0 otherwise."""
    return _b2i(
        _get[ThemeCounterApp](app_ptr)[0].summary_child.child_ctx.is_mounted()
    )


@export
fn tc_counter_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if counter child scope is dirty, 0 otherwise."""
    return _b2i(
        _get[ThemeCounterApp](app_ptr)[0].counter_child.child_ctx.is_dirty()
    )


@export
fn tc_summary_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if summary child scope is dirty, 0 otherwise."""
    return _b2i(
        _get[ThemeCounterApp](app_ptr)[0].summary_child.child_ctx.is_dirty()
    )


@export
fn tc_parent_is_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if parent scope is in the dirty queue, 0 otherwise."""
    var app = _get[ThemeCounterApp](app_ptr)
    var scope_id = app[0].ctx.scope_id
    for i in range(len(app[0].ctx.shell.runtime[0].dirty_scopes)):
        if app[0].ctx.shell.runtime[0].dirty_scopes[i] == scope_id:
            return 1
    return 0


@export
fn tc_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty, 0 otherwise."""
    var app = _get[ThemeCounterApp](app_ptr)
    if app[0].ctx.has_dirty():
        return 1
    if app[0].counter_child.child_ctx.is_dirty():
        return 1
    if app[0].summary_child.child_ctx.is_dirty():
        return 1
    return 0


@export
fn tc_handler_count(app_ptr: Int64) -> Int32:
    """Return the total number of registered handlers."""
    return Int32(_get[ThemeCounterApp](app_ptr)[0].ctx.handler_count())


@export
fn tc_toggle_theme_handler(app_ptr: Int64) -> Int32:
    """Return the toggle-theme handler ID (parent view_events[0])."""
    var events = _get[ThemeCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 0:
        return Int32(events[0].handler_id)
    return -1


@export
fn tc_increment_handler(app_ptr: Int64) -> Int32:
    """Return the increment handler ID (parent view_events[1])."""
    var events = _get[ThemeCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 1:
        return Int32(events[1].handler_id)
    return -1


@export
fn tc_reset_handler(app_ptr: Int64) -> Int32:
    """Return the reset handler ID (counter child event_bindings[0])."""
    var app = _get[ThemeCounterApp](app_ptr)
    if app[0].counter_child.child_ctx.event_count() > 0:
        return Int32(app[0].counter_child.child_ctx.event_handler_id(0))
    return -1


@export
fn tc_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[ThemeCounterApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn tc_counter_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if counter child has rendered, 0 otherwise."""
    return _b2i(
        _get[ThemeCounterApp](app_ptr)[0].counter_child.child_ctx.has_rendered()
    )


@export
fn tc_summary_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if summary child has rendered, 0 otherwise."""
    return _b2i(
        _get[ThemeCounterApp](app_ptr)[0].summary_child.child_ctx.has_rendered()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 32.2 — SafeCounterApp (error boundary demo)
#   Struct + lifecycle: src/apps/safe_counter.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── SafeCounterApp WASM exports ─────────────────────────────────────────────


@export
fn sc_init() -> Int64:
    """Initialize the safe-counter app.  Returns app pointer."""
    return _to_i64(_sc_init())


@export
fn sc_destroy(app_ptr: Int64):
    """Destroy the safe-counter app."""
    _sc_destroy(_get[SafeCounterApp](app_ptr))


@export
fn sc_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _sc_rebuild(_get[SafeCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn sc_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _sc_handle_event(
            _get[SafeCounterApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn sc_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _sc_flush(_get[SafeCounterApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn sc_count_value(app_ptr: Int64) -> Int32:
    """Return the current count value."""
    return _get[SafeCounterApp](app_ptr)[0].count.peek()


@export
fn sc_has_error(app_ptr: Int64) -> Int32:
    """Return 1 if the error boundary has captured an error."""
    return _b2i(_get[SafeCounterApp](app_ptr)[0].ctx.has_error())


@export
fn sc_error_message(app_ptr: Int64) -> String:
    """Return the captured error message."""
    return _get[SafeCounterApp](app_ptr)[0].ctx.error_message()


@export
fn sc_crash_handler(app_ptr: Int64) -> Int32:
    """Return the crash handler ID."""
    return Int32(_get[SafeCounterApp](app_ptr)[0].crash_handler)


@export
fn sc_retry_handler(app_ptr: Int64) -> Int32:
    """Return the retry handler ID."""
    return Int32(_get[SafeCounterApp](app_ptr)[0].retry_handler)


@export
fn sc_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment handler ID (view_events[0])."""
    var events = _get[SafeCounterApp](app_ptr)[0].ctx.view_events()
    if len(events) > 0:
        return Int32(events[0].handler_id)
    return -1


@export
fn sc_normal_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the normal child is currently mounted in the DOM."""
    return _b2i(_get[SafeCounterApp](app_ptr)[0].normal.child_ctx.is_mounted())


@export
fn sc_fallback_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the fallback child is currently mounted in the DOM."""
    return _b2i(
        _get[SafeCounterApp](app_ptr)[0].fallback.child_ctx.is_mounted()
    )


@export
fn sc_normal_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if the normal child has rendered at least once."""
    return _b2i(
        _get[SafeCounterApp](app_ptr)[0].normal.child_ctx.has_rendered()
    )


@export
fn sc_fallback_has_rendered(app_ptr: Int64) -> Int32:
    """Return 1 if the fallback child has rendered at least once."""
    return _b2i(
        _get[SafeCounterApp](app_ptr)[0].fallback.child_ctx.has_rendered()
    )


@export
fn sc_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[SafeCounterApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn sc_handler_count(app_ptr: Int64) -> Int32:
    """Return the total number of registered handlers."""
    return Int32(_get[SafeCounterApp](app_ptr)[0].ctx.handler_count())


@export
fn sc_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[SafeCounterApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn sc_parent_scope_id(app_ptr: Int64) -> Int32:
    """Return the parent (root) scope ID."""
    return Int32(_get[SafeCounterApp](app_ptr)[0].ctx.scope_id)


@export
fn sc_normal_scope_id(app_ptr: Int64) -> Int32:
    """Return the normal child's scope ID."""
    return Int32(_get[SafeCounterApp](app_ptr)[0].normal.child_ctx.scope_id)


@export
fn sc_fallback_scope_id(app_ptr: Int64) -> Int32:
    """Return the fallback child's scope ID."""
    return Int32(_get[SafeCounterApp](app_ptr)[0].fallback.child_ctx.scope_id)


@export
fn sc_normal_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the normal child scope is dirty."""
    return _b2i(_get[SafeCounterApp](app_ptr)[0].normal.child_ctx.is_dirty())


@export
fn sc_fallback_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the fallback child scope is dirty."""
    return _b2i(_get[SafeCounterApp](app_ptr)[0].fallback.child_ctx.is_dirty())


# ══════════════════════════════════════════════════════════════════════════════
# Phase 32.3 — ErrorNestApp (nested error boundaries demo)
#   Struct + lifecycle: src/apps/error_nest.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── ErrorNestApp WASM exports ────────────────────────────────────────────────


@export
fn en_init() -> Int64:
    """Initialize the error-nest app.  Returns app pointer."""
    return _to_i64(_en_init())


@export
fn en_destroy(app_ptr: Int64):
    """Destroy the error-nest app."""
    _en_destroy(_get[ErrorNestApp](app_ptr))


@export
fn en_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _en_rebuild(_get[ErrorNestApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn en_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _en_handle_event(
            _get[ErrorNestApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn en_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _en_flush(_get[ErrorNestApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn en_has_outer_error(app_ptr: Int64) -> Int32:
    """Return 1 if the outer boundary has captured an error."""
    return _b2i(_get[ErrorNestApp](app_ptr)[0].ctx.has_error())


@export
fn en_has_inner_error(app_ptr: Int64) -> Int32:
    """Return 1 if the inner boundary has captured an error."""
    return _b2i(
        _get[ErrorNestApp](app_ptr)[0].outer_normal.child_ctx.has_error()
    )


@export
fn en_outer_error_message(app_ptr: Int64) -> String:
    """Return the outer boundary's error message."""
    return _get[ErrorNestApp](app_ptr)[0].ctx.error_message()


@export
fn en_inner_error_message(app_ptr: Int64) -> String:
    """Return the inner boundary's error message."""
    return _get[ErrorNestApp](app_ptr)[0].outer_normal.child_ctx.error_message()


@export
fn en_outer_crash_handler(app_ptr: Int64) -> Int32:
    """Return the outer crash handler ID."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].outer_crash_handler)


@export
fn en_inner_crash_handler(app_ptr: Int64) -> Int32:
    """Return the inner crash handler ID."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].inner_crash_handler)


@export
fn en_outer_retry_handler(app_ptr: Int64) -> Int32:
    """Return the outer retry handler ID."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].outer_retry_handler)


@export
fn en_inner_retry_handler(app_ptr: Int64) -> Int32:
    """Return the inner retry handler ID."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].inner_retry_handler)


@export
fn en_outer_normal_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the outer normal child is mounted."""
    return _b2i(
        _get[ErrorNestApp](app_ptr)[0].outer_normal.child_ctx.is_mounted()
    )


@export
fn en_outer_fallback_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the outer fallback child is mounted."""
    return _b2i(
        _get[ErrorNestApp](app_ptr)[0].outer_fallback.child_ctx.is_mounted()
    )


@export
fn en_inner_normal_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the inner normal child is mounted."""
    return _b2i(
        _get[ErrorNestApp](app_ptr)[
            0
        ].outer_normal.inner_normal.child_ctx.is_mounted()
    )


@export
fn en_inner_fallback_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the inner fallback child is mounted."""
    return _b2i(
        _get[ErrorNestApp](app_ptr)[
            0
        ].outer_normal.inner_fallback.child_ctx.is_mounted()
    )


@export
fn en_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[ErrorNestApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn en_handler_count(app_ptr: Int64) -> Int32:
    """Return the total number of registered handlers."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].ctx.handler_count())


@export
fn en_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[ErrorNestApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn en_outer_scope_id(app_ptr: Int64) -> Int32:
    """Return the outer (root) scope ID."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].ctx.scope_id)


@export
fn en_inner_boundary_scope_id(app_ptr: Int64) -> Int32:
    """Return the inner boundary (outer_normal) scope ID."""
    return Int32(_get[ErrorNestApp](app_ptr)[0].outer_normal.child_ctx.scope_id)


@export
fn en_inner_normal_scope_id(app_ptr: Int64) -> Int32:
    """Return the inner normal child's scope ID."""
    return Int32(
        _get[ErrorNestApp](app_ptr)[
            0
        ].outer_normal.inner_normal.child_ctx.scope_id
    )


@export
fn en_inner_fallback_scope_id(app_ptr: Int64) -> Int32:
    """Return the inner fallback child's scope ID."""
    return Int32(
        _get[ErrorNestApp](app_ptr)[
            0
        ].outer_normal.inner_fallback.child_ctx.scope_id
    )


@export
fn en_outer_fallback_scope_id(app_ptr: Int64) -> Int32:
    """Return the outer fallback child's scope ID."""
    return Int32(
        _get[ErrorNestApp](app_ptr)[0].outer_fallback.child_ctx.scope_id
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 33.2 — DataLoaderApp (suspense demo)
#   Struct + lifecycle: src/apps/data_loader.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── DataLoaderApp WASM exports ───────────────────────────────────────────────


@export
fn dl_init() -> Int64:
    """Initialize the data-loader app.  Returns app pointer."""
    return _to_i64(_dl_init())


@export
fn dl_destroy(app_ptr: Int64):
    """Destroy the data-loader app."""
    _dl_destroy(_get[DataLoaderApp](app_ptr))


@export
fn dl_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _dl_rebuild(_get[DataLoaderApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn dl_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _dl_handle_event(
            _get[DataLoaderApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn dl_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _dl_flush(_get[DataLoaderApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn dl_resolve(app_ptr: Int64, data: String):
    """Resolve pending state with loaded data."""
    _dl_resolve(_get[DataLoaderApp](app_ptr)[0], data)


@export
fn dl_is_pending(app_ptr: Int64) -> Int32:
    """Return 1 if the app is in pending (loading) state."""
    return _b2i(_get[DataLoaderApp](app_ptr)[0].ctx.is_pending())


@export
fn dl_data_text(app_ptr: Int64) -> String:
    """Return the current data text."""
    return _get[DataLoaderApp](app_ptr)[0].data_text


@export
fn dl_load_handler(app_ptr: Int64) -> Int32:
    """Return the load button handler ID."""
    return Int32(_get[DataLoaderApp](app_ptr)[0].load_handler)


@export
fn dl_content_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the content child is mounted."""
    return _b2i(_get[DataLoaderApp](app_ptr)[0].content.child_ctx.is_mounted())


@export
fn dl_skeleton_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the skeleton child is mounted."""
    return _b2i(_get[DataLoaderApp](app_ptr)[0].skeleton.child_ctx.is_mounted())


@export
fn dl_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[DataLoaderApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn dl_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[DataLoaderApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn dl_parent_scope_id(app_ptr: Int64) -> Int32:
    """Return the parent (root) scope ID."""
    return Int32(_get[DataLoaderApp](app_ptr)[0].ctx.scope_id)


@export
fn dl_content_scope_id(app_ptr: Int64) -> Int32:
    """Return the content child's scope ID."""
    return Int32(_get[DataLoaderApp](app_ptr)[0].content.child_ctx.scope_id)


@export
fn dl_skeleton_scope_id(app_ptr: Int64) -> Int32:
    """Return the skeleton child's scope ID."""
    return Int32(_get[DataLoaderApp](app_ptr)[0].skeleton.child_ctx.scope_id)


# ══════════════════════════════════════════════════════════════════════════════
# Phase 33.3 — SuspenseNestApp (nested suspense boundaries demo)
#   Struct + lifecycle: src/apps/suspense_nest.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── SuspenseNestApp WASM exports ─────────────────────────────────────────────


@export
fn sn_init() -> Int64:
    """Initialize the suspense-nest app.  Returns app pointer."""
    return _to_i64(_sn_init())


@export
fn sn_destroy(app_ptr: Int64):
    """Destroy the suspense-nest app."""
    _sn_destroy(_get[SuspenseNestApp](app_ptr))


@export
fn sn_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _sn_rebuild(_get[SuspenseNestApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn sn_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _sn_handle_event(
            _get[SuspenseNestApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn sn_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _sn_flush(_get[SuspenseNestApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn sn_outer_resolve(app_ptr: Int64, data: String):
    """Resolve outer pending state with data."""
    _sn_outer_resolve(_get[SuspenseNestApp](app_ptr)[0], data)


@export
fn sn_inner_resolve(app_ptr: Int64, data: String):
    """Resolve inner pending state with data."""
    _sn_inner_resolve(_get[SuspenseNestApp](app_ptr)[0], data)


@export
fn sn_is_outer_pending(app_ptr: Int64) -> Int32:
    """Return 1 if the outer boundary is in pending state."""
    return _b2i(_get[SuspenseNestApp](app_ptr)[0].ctx.is_pending())


@export
fn sn_is_inner_pending(app_ptr: Int64) -> Int32:
    """Return 1 if the inner boundary is in pending state."""
    return _b2i(
        _get[SuspenseNestApp](app_ptr)[0].outer_content.child_ctx.is_pending()
    )


@export
fn sn_outer_data(app_ptr: Int64) -> String:
    """Return the current outer data text."""
    return _get[SuspenseNestApp](app_ptr)[0].outer_data


@export
fn sn_inner_data(app_ptr: Int64) -> String:
    """Return the current inner data text."""
    return _get[SuspenseNestApp](app_ptr)[0].inner_data


@export
fn sn_outer_load_handler(app_ptr: Int64) -> Int32:
    """Return the outer load button handler ID."""
    return Int32(_get[SuspenseNestApp](app_ptr)[0].outer_load_handler)


@export
fn sn_inner_load_handler(app_ptr: Int64) -> Int32:
    """Return the inner load button handler ID."""
    return Int32(_get[SuspenseNestApp](app_ptr)[0].inner_load_handler)


@export
fn sn_outer_content_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the outer content child is mounted."""
    return _b2i(
        _get[SuspenseNestApp](app_ptr)[0].outer_content.child_ctx.is_mounted()
    )


@export
fn sn_outer_skeleton_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the outer skeleton child is mounted."""
    return _b2i(
        _get[SuspenseNestApp](app_ptr)[0].outer_skeleton.child_ctx.is_mounted()
    )


@export
fn sn_inner_content_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the inner content child is mounted."""
    return _b2i(
        _get[SuspenseNestApp](app_ptr)[
            0
        ].outer_content.inner_content.child_ctx.is_mounted()
    )


@export
fn sn_inner_skeleton_mounted(app_ptr: Int64) -> Int32:
    """Return 1 if the inner skeleton child is mounted."""
    return _b2i(
        _get[SuspenseNestApp](app_ptr)[
            0
        ].outer_content.inner_skeleton.child_ctx.is_mounted()
    )


@export
fn sn_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[SuspenseNestApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn sn_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[SuspenseNestApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn sn_outer_scope_id(app_ptr: Int64) -> Int32:
    """Return the outer (root) scope ID."""
    return Int32(_get[SuspenseNestApp](app_ptr)[0].ctx.scope_id)


@export
fn sn_inner_boundary_scope_id(app_ptr: Int64) -> Int32:
    """Return the inner boundary (outer_content) scope ID."""
    return Int32(
        _get[SuspenseNestApp](app_ptr)[0].outer_content.child_ctx.scope_id
    )


@export
fn sn_inner_content_scope_id(app_ptr: Int64) -> Int32:
    """Return the inner content child's scope ID."""
    return Int32(
        _get[SuspenseNestApp](app_ptr)[
            0
        ].outer_content.inner_content.child_ctx.scope_id
    )


@export
fn sn_inner_skeleton_scope_id(app_ptr: Int64) -> Int32:
    """Return the inner skeleton child's scope ID."""
    return Int32(
        _get[SuspenseNestApp](app_ptr)[
            0
        ].outer_content.inner_skeleton.child_ctx.scope_id
    )


@export
fn sn_outer_skeleton_scope_id(app_ptr: Int64) -> Int32:
    """Return the outer skeleton child's scope ID."""
    return Int32(
        _get[SuspenseNestApp](app_ptr)[0].outer_skeleton.child_ctx.scope_id
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 34.1 — EffectDemoApp (effect-in-flush pattern)
#   Struct + lifecycle: src/apps/effect_demo.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── EffectDemoApp WASM exports ───────────────────────────────────────────────


@export
fn ed_init() -> Int64:
    """Initialize the effect-demo app.  Returns app pointer."""
    return _to_i64(_ed_init())


@export
fn ed_destroy(app_ptr: Int64):
    """Destroy the effect-demo app."""
    _ed_destroy(_get[EffectDemoApp](app_ptr))


@export
fn ed_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _ed_rebuild(_get[EffectDemoApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn ed_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _ed_handle_event(
            _get[EffectDemoApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn ed_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _ed_flush(_get[EffectDemoApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn ed_count_value(app_ptr: Int64) -> Int32:
    """Return the current count signal value."""
    return _get[EffectDemoApp](app_ptr)[0].count.peek()


@export
fn ed_doubled_value(app_ptr: Int64) -> Int32:
    """Return the current doubled signal value."""
    return _get[EffectDemoApp](app_ptr)[0].doubled.peek()


@export
fn ed_parity_text(app_ptr: Int64) -> String:
    """Return the current parity text ("even" or "odd")."""
    return _get[EffectDemoApp](app_ptr)[0].parity.peek()


@export
fn ed_effect_is_pending(app_ptr: Int64) -> Int32:
    """Return 1 if the count effect is pending."""
    return _b2i(_get[EffectDemoApp](app_ptr)[0].count_effect.is_pending())


@export
fn ed_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment button handler ID."""
    return Int32(_get[EffectDemoApp](app_ptr)[0].incr_handler)


@export
fn ed_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[EffectDemoApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn ed_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[EffectDemoApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 34.2 — EffectMemoApp (signal → memo → effect → signal chain)
#   Struct + lifecycle: src/apps/effect_memo.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── EffectMemoApp WASM exports ───────────────────────────────────────────────


@export
fn em_init() -> Int64:
    """Initialize the effect-memo app.  Returns app pointer."""
    return _to_i64(_em_init())


@export
fn em_destroy(app_ptr: Int64):
    """Destroy the effect-memo app."""
    _em_destroy(_get[EffectMemoApp](app_ptr))


@export
fn em_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _em_rebuild(_get[EffectMemoApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn em_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _em_handle_event(
            _get[EffectMemoApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn em_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _em_flush(_get[EffectMemoApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn em_input_value(app_ptr: Int64) -> Int32:
    """Return the current input signal value."""
    return _get[EffectMemoApp](app_ptr)[0].input.peek()


@export
fn em_tripled_value(app_ptr: Int64) -> Int32:
    """Return the current tripled memo value."""
    return _get[EffectMemoApp](app_ptr)[0].tripled.peek()


@export
fn em_label_text(app_ptr: Int64) -> String:
    """Return the current label text ("small" or "big")."""
    return _get[EffectMemoApp](app_ptr)[0].label.peek()


@export
fn em_effect_is_pending(app_ptr: Int64) -> Int32:
    """Return 1 if the label effect is pending."""
    return _b2i(_get[EffectMemoApp](app_ptr)[0].label_effect.is_pending())


@export
fn em_memo_value(app_ptr: Int64) -> Int32:
    """Return the raw memo output value (same as tripled)."""
    return _get[EffectMemoApp](app_ptr)[0].tripled.peek()


@export
fn em_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment button handler ID."""
    return Int32(_get[EffectMemoApp](app_ptr)[0].incr_handler)


@export
fn em_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[EffectMemoApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn em_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[EffectMemoApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 35.2 — MemoFormApp (MemoBool + MemoString in a form)
#   Struct + lifecycle: src/apps/memo_form.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── MemoFormApp WASM exports ─────────────────────────────────────────────────


@export
fn mf_init() -> Int64:
    """Initialize the memo-form app.  Returns app pointer."""
    return _to_i64(_mf_init())


@export
fn mf_destroy(app_ptr: Int64):
    """Destroy the memo-form app."""
    _mf_destroy(_get[MemoFormApp](app_ptr))


@export
fn mf_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _mf_rebuild(_get[MemoFormApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn mf_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _mf_handle_event(_get[MemoFormApp](app_ptr)[0], UInt32(hid), UInt8(evt))
    )


@export
fn mf_handle_event_string(
    app_ptr: Int64, hid: Int32, evt: Int32, value: String
) -> Int32:
    """Dispatch a string event (input/change).  Returns 1 if handled."""
    return _b2i(
        _mf_handle_event_string(
            _get[MemoFormApp](app_ptr)[0], UInt32(hid), UInt8(evt), value
        )
    )


@export
fn mf_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _mf_flush(_get[MemoFormApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn mf_input_text(app_ptr: Int64) -> String:
    """Return the current input signal text."""
    return _get[MemoFormApp](app_ptr)[0].input.peek()


@export
fn mf_is_valid(app_ptr: Int64) -> Int32:
    """Return 1 if the is_valid memo is True, 0 otherwise."""
    return _b2i(_get[MemoFormApp](app_ptr)[0].is_valid.peek())


@export
fn mf_status_text(app_ptr: Int64) -> String:
    """Return the current status memo text."""
    return _get[MemoFormApp](app_ptr)[0].status.peek()


@export
fn mf_is_valid_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the is_valid memo needs recomputation."""
    return _b2i(_get[MemoFormApp](app_ptr)[0].is_valid.is_dirty())


@export
fn mf_status_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the status memo needs recomputation."""
    return _b2i(_get[MemoFormApp](app_ptr)[0].status.is_dirty())


@export
fn mf_set_input(app_ptr: Int64, value: String):
    """Test helper: write a string directly to the input signal."""
    _get[MemoFormApp](app_ptr)[0].input.set(value)


@export
fn mf_input_handler(app_ptr: Int64) -> Int32:
    """Return the oninput_set_string handler ID."""
    return Int32(_get[MemoFormApp](app_ptr)[0].input_handler)


@export
fn mf_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[MemoFormApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn mf_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[MemoFormApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn mf_memo_count(app_ptr: Int64) -> Int32:
    """Return the number of live memos."""
    return Int32(
        _get[MemoFormApp](app_ptr)[0].ctx.shell.runtime[0].memos.count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 35.3 — MemoChainApp (mixed-type memo chain)
#   Struct + lifecycle: src/apps/memo_chain.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── MemoChainApp WASM exports ────────────────────────────────────────────────


@export
fn mc_init() -> Int64:
    """Initialize the memo-chain app.  Returns app pointer."""
    return _to_i64(_mc_init())


@export
fn mc_destroy(app_ptr: Int64):
    """Destroy the memo-chain app."""
    _mc_destroy(_get[MemoChainApp](app_ptr))


@export
fn mc_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _mc_rebuild(_get[MemoChainApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn mc_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _mc_handle_event(
            _get[MemoChainApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn mc_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _mc_flush(_get[MemoChainApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn mc_input_value(app_ptr: Int64) -> Int32:
    """Return the current input signal value."""
    return _get[MemoChainApp](app_ptr)[0].input.peek()


@export
fn mc_doubled_value(app_ptr: Int64) -> Int32:
    """Return the current doubled memo value."""
    return _get[MemoChainApp](app_ptr)[0].doubled.peek()


@export
fn mc_is_big(app_ptr: Int64) -> Int32:
    """Return 1 if the is_big memo is True, 0 otherwise."""
    return _b2i(_get[MemoChainApp](app_ptr)[0].is_big.peek())


@export
fn mc_label_text(app_ptr: Int64) -> String:
    """Return the current label memo text."""
    return _get[MemoChainApp](app_ptr)[0].label.peek()


@export
fn mc_doubled_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the doubled memo needs recomputation."""
    return _b2i(_get[MemoChainApp](app_ptr)[0].doubled.is_dirty())


@export
fn mc_is_big_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the is_big memo needs recomputation."""
    return _b2i(_get[MemoChainApp](app_ptr)[0].is_big.is_dirty())


@export
fn mc_label_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the label memo needs recomputation."""
    return _b2i(_get[MemoChainApp](app_ptr)[0].label.is_dirty())


@export
fn mc_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment button handler ID."""
    return Int32(_get[MemoChainApp](app_ptr)[0].incr_handler)


@export
fn mc_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[MemoChainApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn mc_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[MemoChainApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn mc_memo_count(app_ptr: Int64) -> Int32:
    """Return the number of live memos."""
    return Int32(
        _get[MemoChainApp](app_ptr)[0].ctx.shell.runtime[0].memos.count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 37.3 — EqualityDemoApp (equality-gated memo chain)
#   Struct + lifecycle: src/apps/equality_demo.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── EqualityDemoApp WASM exports ──────────────────────────────────────────────


@export
fn eq_init() -> Int64:
    """Initialize the equality-demo app.  Returns app pointer."""
    return _to_i64(_eq_init())


@export
fn eq_destroy(app_ptr: Int64):
    """Destroy the equality-demo app."""
    _eq_destroy(_get[EqualityDemoApp](app_ptr))


@export
fn eq_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial render of the equality-demo app."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _eq_rebuild(_get[EqualityDemoApp](app_ptr)[0], writer_ptr)
    return result


@export
fn eq_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event to the equality-demo app."""
    if _eq_handle_event(
        _get[EqualityDemoApp](app_ptr)[0], UInt32(hid), UInt8(evt)
    ):
        return 1
    return 0


@export
fn eq_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates for the equality-demo app."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _eq_flush(_get[EqualityDemoApp](app_ptr)[0], writer_ptr)
    return result


@export
fn eq_input_value(app_ptr: Int64) -> Int32:
    """Return the current input signal value."""
    return _get[EqualityDemoApp](app_ptr)[0].input.peek()


@export
fn eq_clamped_value(app_ptr: Int64) -> Int32:
    """Return the current clamped memo value."""
    return _get[EqualityDemoApp](app_ptr)[0].clamped.peek()


@export
fn eq_label_text(app_ptr: Int64) -> String:
    """Return the current label memo text."""
    return _get[EqualityDemoApp](app_ptr)[0].label.peek()


@export
fn eq_clamped_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the clamped memo needs recomputation."""
    return _b2i(_get[EqualityDemoApp](app_ptr)[0].clamped.is_dirty())


@export
fn eq_label_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the label memo needs recomputation."""
    return _b2i(_get[EqualityDemoApp](app_ptr)[0].label.is_dirty())


@export
fn eq_clamped_changed(app_ptr: Int64) -> Int32:
    """Return 1 if the clamped memo's last recomputation changed its value."""
    return _b2i(
        _get[EqualityDemoApp](app_ptr)[0]
        .ctx.shell.runtime[0]
        .memo_did_value_change(_get[EqualityDemoApp](app_ptr)[0].clamped.id)
    )


@export
fn eq_label_changed(app_ptr: Int64) -> Int32:
    """Return 1 if the label memo's last recomputation changed its value."""
    return _b2i(
        _get[EqualityDemoApp](app_ptr)[0]
        .ctx.shell.runtime[0]
        .memo_did_value_change(_get[EqualityDemoApp](app_ptr)[0].label.id)
    )


@export
fn eq_incr_handler(app_ptr: Int64) -> Int32:
    """Return the increment button handler ID."""
    return Int32(_get[EqualityDemoApp](app_ptr)[0].incr_handler)


@export
fn eq_decr_handler(app_ptr: Int64) -> Int32:
    """Return the decrement button handler ID."""
    return Int32(_get[EqualityDemoApp](app_ptr)[0].decr_handler)


@export
fn eq_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[EqualityDemoApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn eq_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[EqualityDemoApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn eq_memo_count(app_ptr: Int64) -> Int32:
    """Return the number of live memos."""
    return Int32(
        _get[EqualityDemoApp](app_ptr)[0].ctx.shell.runtime[0].memos.count()
    )


# ══════════════════════════════════════════════════════════════════════════════
# Phase 38.2 — BatchDemoApp (batch signal writes)
#   Struct + lifecycle: src/apps/batch_demo.mojo
# ══════════════════════════════════════════════════════════════════════════════


# ── BatchDemoApp WASM exports ────────────────────────────────────────────────


@export
fn bd_init() -> Int64:
    """Initialize the batch-demo app.  Returns app pointer."""
    return _to_i64(_bd_init())


@export
fn bd_destroy(app_ptr: Int64):
    """Destroy the batch-demo app."""
    _bd_destroy(_get[BatchDemoApp](app_ptr))


@export
fn bd_rebuild(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Initial mount.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _bd_rebuild(_get[BatchDemoApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn bd_handle_event(app_ptr: Int64, hid: Int32, evt: Int32) -> Int32:
    """Dispatch an event.  Returns 1 if handled, 0 otherwise."""
    return _b2i(
        _bd_handle_event(
            _get[BatchDemoApp](app_ptr)[0], UInt32(hid), UInt8(evt)
        )
    )


@export
fn bd_flush(app_ptr: Int64, buf_ptr: Int64, cap: Int32) -> Int32:
    """Flush pending updates.  Returns mutation buffer length."""
    var writer_ptr = _alloc_writer(buf_ptr, cap)
    var result = _bd_flush(_get[BatchDemoApp](app_ptr)[0], writer_ptr)
    _free_writer(writer_ptr)
    return result


@export
fn bd_set_names(app_ptr: Int64, first: String, last: String):
    """Set both names in a batch (begin_batch + writes + end_batch).

    This is a custom WASM export that calls the batch method directly,
    bypassing the normal dispatch_event path.
    """
    _get[BatchDemoApp](app_ptr)[0].set_names(first, last)


@export
fn bd_reset(app_ptr: Int64):
    """Reset all state in a batch (begin_batch + writes + end_batch)."""
    _get[BatchDemoApp](app_ptr)[0].reset()


@export
fn bd_full_name_text(app_ptr: Int64) -> String:
    """Return the current full_name memo text."""
    return _get[BatchDemoApp](app_ptr)[0].full_name.peek()


@export
fn bd_write_count(app_ptr: Int64) -> Int32:
    """Return the current write_count signal value."""
    return _get[BatchDemoApp](app_ptr)[0].write_count.peek()


@export
fn bd_first_name_text(app_ptr: Int64) -> String:
    """Return the current first_name signal text."""
    return _get[BatchDemoApp](app_ptr)[0].first_name.peek()


@export
fn bd_last_name_text(app_ptr: Int64) -> String:
    """Return the current last_name signal text."""
    return _get[BatchDemoApp](app_ptr)[0].last_name.peek()


@export
fn bd_full_name_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if the full_name memo needs recomputation."""
    return _b2i(_get[BatchDemoApp](app_ptr)[0].full_name.is_dirty())


@export
fn bd_full_name_changed(app_ptr: Int64) -> Int32:
    """Return 1 if the full_name memo's last recompute changed its value."""
    return _b2i(
        _get[BatchDemoApp](app_ptr)[0]
        .ctx.shell.runtime[0]
        .memo_did_value_change(_get[BatchDemoApp](app_ptr)[0].full_name.id)
    )


@export
fn bd_has_dirty(app_ptr: Int64) -> Int32:
    """Return 1 if any scope is dirty."""
    var app = _get[BatchDemoApp](app_ptr)
    if len(app[0].ctx.shell.runtime[0].dirty_scopes) > 0:
        return 1
    return 0


@export
fn bd_is_batching(app_ptr: Int64) -> Int32:
    """Return 1 if the runtime is currently in batch mode."""
    return _b2i(_get[BatchDemoApp](app_ptr)[0].ctx.is_batching())


@export
fn bd_set_handler(app_ptr: Int64) -> Int32:
    """Return the set-names button handler ID."""
    return Int32(_get[BatchDemoApp](app_ptr)[0].set_handler)


@export
fn bd_reset_handler(app_ptr: Int64) -> Int32:
    """Return the reset button handler ID."""
    return Int32(_get[BatchDemoApp](app_ptr)[0].reset_handler)


@export
fn bd_scope_count(app_ptr: Int64) -> Int32:
    """Return the number of live scopes."""
    return Int32(
        _get[BatchDemoApp](app_ptr)[0].ctx.shell.runtime[0].scope_count()
    )


@export
fn bd_memo_count(app_ptr: Int64) -> Int32:
    """Return the number of live memos."""
    return Int32(
        _get[BatchDemoApp](app_ptr)[0].ctx.shell.runtime[0].memos.count()
    )
