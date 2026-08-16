# DSL Ergonomic Builder — WASM-level tests via mojo-wasmtime.
#
# Exercises the dsl_test_*, dsl_node_*, dsl_vb_*, and dsl_to_template
# WASM exports through the real compiled binary.
#
# Run with:
#   mojo test -I ../mojo-wasmtime/src test/test_dsl.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    args_ptr_i32_ptr,
    args_ptr_i32_ptr_ptr,
    args_ptr_ptr,
    args_ptr_ptr_i32,
    args_ptr_ptr_ptr,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _create_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    return Int(w[].call_i64("runtime_create", no_args()))


fn _destroy_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], rt: Int
) raises:
    w[].call_void("runtime_destroy", args_ptr(rt))


fn _create_vnode_store(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    return Int(w[].call_i64("vnode_store_create", no_args()))


fn _destroy_vnode_store(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], s: Int
) raises:
    w[].call_void("vnode_store_destroy", args_ptr(s))


# ── Constants ────────────────────────────────────────────────────────────────

# Node kinds
comptime NODE_TEXT = 0
comptime NODE_ELEMENT = 1
comptime NODE_DYN_TEXT = 2
comptime NODE_DYN_NODE = 3
comptime NODE_STATIC_ATTR = 4
comptime NODE_DYN_ATTR = 5

# HTML tag constants
comptime TAG_DIV = 0
comptime TAG_SPAN = 1
comptime TAG_P = 2
comptime TAG_H1 = 10
comptime TAG_BUTTON = 19

# VNode kinds
comptime VNODE_TEMPLATE_REF = 0

# Template node kinds
comptime TNODE_ELEMENT = 0
comptime TNODE_TEXT = 1
comptime TNODE_DYNAMIC_TEXT = 3


# ══════════════════════════════════════════════════════════════════════════════
# Section 1: Self-contained Mojo-side tests (return 1 for pass)
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_text_node(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Int(w[].call_i32("dsl_test_text_node", no_args()))
    assert_equal(result, 1, "dsl_test_text_node passed")


fn test_dsl_dyn_text_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_dyn_text_node", no_args()))
    assert_equal(result, 1, "dsl_test_dyn_text_node passed")


fn test_dsl_dyn_node_slot(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_dyn_node_slot", no_args()))
    assert_equal(result, 1, "dsl_test_dyn_node_slot passed")


fn test_dsl_static_attr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_static_attr", no_args()))
    assert_equal(result, 1, "dsl_test_static_attr passed")


fn test_dsl_dyn_attr(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var result = Int(w[].call_i32("dsl_test_dyn_attr", no_args()))
    assert_equal(result, 1, "dsl_test_dyn_attr passed")


fn test_dsl_empty_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_empty_element", no_args()))
    assert_equal(result, 1, "dsl_test_empty_element passed")


fn test_dsl_element_with_children(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_element_with_children", no_args()))
    assert_equal(result, 1, "dsl_test_element_with_children passed")


fn test_dsl_element_with_attrs(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_element_with_attrs", no_args()))
    assert_equal(result, 1, "dsl_test_element_with_attrs passed")


fn test_dsl_element_mixed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_element_mixed", no_args()))
    assert_equal(result, 1, "dsl_test_element_mixed passed")


fn test_dsl_nested_elements(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_nested_elements", no_args()))
    assert_equal(result, 1, "dsl_test_nested_elements passed")


fn test_dsl_all_tag_helpers(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_all_tag_helpers", no_args()))
    assert_equal(result, 1, "dsl_test_all_tag_helpers passed")


fn test_dsl_count_utilities(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_count_utilities", no_args()))
    assert_equal(result, 1, "dsl_test_count_utilities passed")


# ══════════════════════════════════════════════════════════════════════════════
# Section 2: Template conversion tests
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_to_template_simple(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_to_template_simple", no_args()))
    assert_equal(result, 1, "dsl_test_to_template_simple passed")


fn test_dsl_to_template_attrs(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_to_template_attrs", no_args()))
    assert_equal(result, 1, "dsl_test_to_template_attrs passed")


fn test_dsl_to_template_multi_root(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_to_template_multi_root", no_args()))
    assert_equal(result, 1, "dsl_test_to_template_multi_root passed")


fn test_dsl_counter_template(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_counter_template", no_args()))
    assert_equal(result, 1, "dsl_test_counter_template passed")


fn test_dsl_template_equivalence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_template_equivalence", no_args()))
    assert_equal(result, 1, "dsl_test_template_equivalence passed")


# ══════════════════════════════════════════════════════════════════════════════
# Section 3: VNodeBuilder tests
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_vnode_builder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_vnode_builder", no_args()))
    assert_equal(result, 1, "dsl_test_vnode_builder passed")


fn test_dsl_vnode_builder_keyed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var result = Int(w[].call_i32("dsl_test_vnode_builder_keyed", no_args()))
    assert_equal(result, 1, "dsl_test_vnode_builder_keyed passed")


# ══════════════════════════════════════════════════════════════════════════════
# Section 4: Orchestrated Node construction via WASM exports
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_node_create_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create a text Node via WASM, verify kind."""
    var n = Int(
        w[].call_i64(
            "dsl_node_text", args_ptr(w[].write_string_struct("hello"))
        )
    )
    var kind = Int(w[].call_i32("dsl_node_kind", args_ptr(n)))
    assert_equal(kind, NODE_TEXT, "text node kind is NODE_TEXT")
    w[].call_void("dsl_node_destroy", args_ptr(n))


fn test_dsl_node_create_dyn_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create a dyn_text Node via WASM, verify kind and index."""
    var n = Int(w[].call_i64("dsl_node_dyn_text", args_i32(4)))
    var kind = Int(w[].call_i32("dsl_node_kind", args_ptr(n)))
    assert_equal(kind, NODE_DYN_TEXT, "dyn_text node kind")
    var idx = Int(w[].call_i32("dsl_node_dynamic_index", args_ptr(n)))
    assert_equal(idx, 4, "dynamic_index is 4")
    w[].call_void("dsl_node_destroy", args_ptr(n))


fn test_dsl_node_create_dyn_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create a dyn_node Node via WASM, verify kind and index."""
    var n = Int(w[].call_i64("dsl_node_dyn_node", args_i32(2)))
    var kind = Int(w[].call_i32("dsl_node_kind", args_ptr(n)))
    assert_equal(kind, NODE_DYN_NODE, "dyn_node kind")
    var idx = Int(w[].call_i32("dsl_node_dynamic_index", args_ptr(n)))
    assert_equal(idx, 2, "dynamic_index is 2")
    w[].call_void("dsl_node_destroy", args_ptr(n))


fn test_dsl_node_create_attr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create a static attr Node via WASM, verify kind."""
    var n = Int(
        w[].call_i64(
            "dsl_node_attr",
            args_ptr_ptr(
                w[].write_string_struct("class"),
                w[].write_string_struct("active"),
            ),
        )
    )
    var kind = Int(w[].call_i32("dsl_node_kind", args_ptr(n)))
    assert_equal(kind, NODE_STATIC_ATTR, "static attr kind")
    w[].call_void("dsl_node_destroy", args_ptr(n))


fn test_dsl_node_create_dyn_attr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create a dyn_attr Node via WASM, verify kind and index."""
    var n = Int(w[].call_i64("dsl_node_dyn_attr", args_i32(1)))
    var kind = Int(w[].call_i32("dsl_node_kind", args_ptr(n)))
    assert_equal(kind, NODE_DYN_ATTR, "dyn_attr kind")
    var idx = Int(w[].call_i32("dsl_node_dynamic_index", args_ptr(n)))
    assert_equal(idx, 1, "dynamic_index is 1")
    w[].call_void("dsl_node_destroy", args_ptr(n))


fn test_dsl_node_create_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create an empty element Node via WASM, verify kind and tag."""
    var n = Int(w[].call_i64("dsl_node_element", args_i32(TAG_DIV)))
    var kind = Int(w[].call_i32("dsl_node_kind", args_ptr(n)))
    assert_equal(kind, NODE_ELEMENT, "element kind")
    var tag = Int(w[].call_i32("dsl_node_tag", args_ptr(n)))
    assert_equal(tag, TAG_DIV, "tag is TAG_DIV")
    var items = Int(w[].call_i32("dsl_node_item_count", args_ptr(n)))
    assert_equal(items, 0, "0 items in empty element")
    w[].call_void("dsl_node_destroy", args_ptr(n))


fn test_dsl_node_add_items(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Add children and attrs to an element, verify counts."""

    # Create div
    var div = Int(w[].call_i64("dsl_node_element", args_i32(TAG_DIV)))

    # Create children and attr
    var txt = Int(
        w[].call_i64("dsl_node_text", args_ptr(w[].write_string_struct("hi")))
    )
    var a = Int(
        w[].call_i64(
            "dsl_node_attr",
            args_ptr_ptr(
                w[].write_string_struct("id"),
                w[].write_string_struct("x"),
            ),
        )
    )
    var dt = Int(w[].call_i64("dsl_node_dyn_text", args_i32(0)))

    # Add items (child pointers consumed)
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, a))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, txt))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, dt))

    var item_count = Int(w[].call_i32("dsl_node_item_count", args_ptr(div)))
    assert_equal(item_count, 3, "3 items total")

    var child_count = Int(w[].call_i32("dsl_node_child_count", args_ptr(div)))
    assert_equal(child_count, 2, "2 children (text + dyn_text)")

    var attr_count = Int(w[].call_i32("dsl_node_attr_count", args_ptr(div)))
    assert_equal(attr_count, 1, "1 attr")

    w[].call_void("dsl_node_destroy", args_ptr(div))


fn test_dsl_node_nested_tree(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Build nested tree and verify recursive counts."""

    # Build: div > [ span > text("inner"), button > dyn_text(0) + dyn_attr(0) ]
    var span = Int(w[].call_i64("dsl_node_element", args_i32(TAG_SPAN)))
    var inner_txt = Int(
        w[].call_i64(
            "dsl_node_text", args_ptr(w[].write_string_struct("inner"))
        )
    )
    w[].call_void("dsl_node_add_item", args_ptr_ptr(span, inner_txt))

    var btn = Int(w[].call_i64("dsl_node_element", args_i32(TAG_BUTTON)))
    var d_txt = Int(w[].call_i64("dsl_node_dyn_text", args_i32(0)))
    var d_attr = Int(w[].call_i64("dsl_node_dyn_attr", args_i32(0)))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(btn, d_txt))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(btn, d_attr))

    var div = Int(w[].call_i64("dsl_node_element", args_i32(TAG_DIV)))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, span))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, btn))

    # Verify counts
    var nodes = Int(w[].call_i32("dsl_node_count_nodes", args_ptr(div)))
    assert_equal(nodes, 5, "5 tree nodes (div+span+text+btn+dyn_text)")

    var dyn_text_count = Int(
        w[].call_i32("dsl_node_count_dyn_text", args_ptr(div))
    )
    assert_equal(dyn_text_count, 1, "1 dyn_text slot")

    var dyn_attr_count = Int(
        w[].call_i32("dsl_node_count_dyn_attr", args_ptr(div))
    )
    assert_equal(dyn_attr_count, 1, "1 dyn_attr slot")

    # count_all includes attrs: div + span + text + btn + dyn_text + dyn_attr = 6
    var all_count = Int(w[].call_i32("dsl_node_count_all", args_ptr(div)))
    assert_equal(all_count, 6, "6 total items including attrs")

    # No dyn_node slots in this tree
    var dyn_node_count = Int(
        w[].call_i32("dsl_node_count_dyn_node", args_ptr(div))
    )
    assert_equal(dyn_node_count, 0, "0 dyn_node slots")

    # No static attrs in this tree
    var static_attr_count = Int(
        w[].call_i32("dsl_node_count_static_attr", args_ptr(div))
    )
    assert_equal(static_attr_count, 0, "0 static_attr nodes")

    w[].call_void("dsl_node_destroy", args_ptr(div))


fn test_dsl_node_count_dyn_node_and_static_attr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Build a tree with dyn_node and static_attr to cover remaining count exports.
    """

    # Build: div > [ dyn_node(0), dyn_node(1), attr("class","x"), attr("id","y") ]
    var div = Int(w[].call_i64("dsl_node_element", args_i32(TAG_DIV)))

    var dn0 = Int(w[].call_i64("dsl_node_dyn_node", args_i32(0)))
    var dn1 = Int(w[].call_i64("dsl_node_dyn_node", args_i32(1)))
    var a0 = Int(
        w[].call_i64(
            "dsl_node_attr",
            args_ptr_ptr(
                w[].write_string_struct("class"),
                w[].write_string_struct("x"),
            ),
        )
    )
    var a1 = Int(
        w[].call_i64(
            "dsl_node_attr",
            args_ptr_ptr(
                w[].write_string_struct("id"),
                w[].write_string_struct("y"),
            ),
        )
    )

    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, dn0))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, dn1))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, a0))
    w[].call_void("dsl_node_add_item", args_ptr_ptr(div, a1))

    # dyn_node slots
    var dyn_node_count = Int(
        w[].call_i32("dsl_node_count_dyn_node", args_ptr(div))
    )
    assert_equal(dyn_node_count, 2, "2 dyn_node slots")

    # static_attr nodes
    var static_attr_count = Int(
        w[].call_i32("dsl_node_count_static_attr", args_ptr(div))
    )
    assert_equal(static_attr_count, 2, "2 static_attr nodes")

    # count_all: div has 4 items (2 dyn_node + 2 attr), each is a leaf = 5 total
    var all_count = Int(w[].call_i32("dsl_node_count_all", args_ptr(div)))
    assert_equal(all_count, 5, "5 total items (div + 2 dyn_node + 2 attr)")

    # count_nodes excludes attrs: div + 2 dyn_node = 3
    var node_count = Int(w[].call_i32("dsl_node_count_nodes", args_ptr(div)))
    assert_equal(node_count, 3, "3 tree nodes (div + 2 dyn_node)")

    w[].call_void("dsl_node_destroy", args_ptr(div))


fn test_dsl_node_to_template(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Build a Node tree, convert to template, verify structure."""
    var rt = _create_runtime(w)

    # Build: h1 > text("Title")
    var h1 = Int(w[].call_i64("dsl_node_element", args_i32(TAG_H1)))
    var txt = Int(
        w[].call_i64(
            "dsl_node_text", args_ptr(w[].write_string_struct("Title"))
        )
    )
    w[].call_void("dsl_node_add_item", args_ptr_ptr(h1, txt))

    # Convert to template (consumes node)
    # dsl_to_template signature: (node_ptr: Int64, name: String, rt_ptr: Int64) -> Int32
    var tmpl_id = Int(
        w[].call_i32(
            "dsl_to_template",
            args_ptr_ptr_ptr(h1, w[].write_string_struct("mojo-h1-test"), rt),
        )
    )

    # Verify template structure
    var node_count = Int(
        w[].call_i32("tmpl_node_count", args_ptr_i32(rt, Int32(tmpl_id)))
    )
    assert_equal(node_count, 2, "template has 2 nodes (h1 + text)")

    var root_count = Int(
        w[].call_i32("tmpl_root_count", args_ptr_i32(rt, Int32(tmpl_id)))
    )
    assert_equal(root_count, 1, "template has 1 root")

    _destroy_runtime(w, rt)


fn test_dsl_vb_create_and_query(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Create a VNodeBuilder via WASM, add dynamic content, verify."""

    # Use the self-contained test which handles all the orchestration
    var result = Int(w[].call_i32("dsl_test_vnode_builder", no_args()))
    assert_equal(result, 1, "VNodeBuilder self-contained test passed")


fn test_dsl_vb_keyed(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    """Create a keyed VNodeBuilder via WASM."""
    var result = Int(w[].call_i32("dsl_test_vnode_builder_keyed", no_args()))
    assert_equal(result, 1, "keyed VNodeBuilder self-contained test passed")


# ══════════════════════════════════════════════════════════════════════════════
# Section 5: Template equivalence (DSL vs manual builder)
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_template_equivalence_via_wasm(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Verify DSL-built and manually-built counter templates are equivalent."""
    var result = Int(w[].call_i32("dsl_test_template_equivalence", no_args()))
    assert_equal(result, 1, "template equivalence test passed")


# ══════════════════════════════════════════════════════════════════════════════
# Section 6: Counter template round-trip
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_counter_template_via_wasm(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Build counter template via DSL and verify all properties."""
    var result = Int(w[].call_i32("dsl_test_counter_template", no_args()))
    assert_equal(result, 1, "counter template test passed")


# ══════════════════════════════════════════════════════════════════════════════
# Section 7: Multi-root template
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_multi_root_via_wasm(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Multiple root nodes via to_template_multi."""
    var result = Int(w[].call_i32("dsl_test_to_template_multi_root", no_args()))
    assert_equal(result, 1, "multi-root template test passed")


# ══════════════════════════════════════════════════════════════════════════════
# Phase 20 — M20.3: oninput_set_string / onchange_set_string
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_oninput_set_string_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Oninput_set_string creates NODE_EVENT with correct fields."""
    var result = Int(
        w[].call_i32("dsl_test_oninput_set_string_node", no_args())
    )
    assert_equal(result, 1, "oninput_set_string_node passed")


fn test_dsl_onchange_set_string_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Onchange_set_string creates NODE_EVENT with correct fields."""
    var result = Int(
        w[].call_i32("dsl_test_onchange_set_string_node", no_args())
    )
    assert_equal(result, 1, "onchange_set_string_node passed")


fn test_dsl_oninput_in_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Oninput_set_string inside an element counts as dynamic attr."""
    var result = Int(w[].call_i32("dsl_test_oninput_in_element", no_args()))
    assert_equal(result, 1, "oninput_in_element passed")


# ══════════════════════════════════════════════════════════════════════════════
# Phase 20 — M20.4: bind_value / bind_attr
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_bind_value_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bind_value creates NODE_BIND_VALUE with attr_name='value'."""
    var result = Int(w[].call_i32("dsl_test_bind_value_node", no_args()))
    assert_equal(result, 1, "bind_value_node passed")


fn test_dsl_bind_attr_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bind_attr creates NODE_BIND_VALUE with custom attr name."""
    var result = Int(w[].call_i32("dsl_test_bind_attr_node", no_args()))
    assert_equal(result, 1, "bind_attr_node passed")


fn test_dsl_bind_value_in_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bind_value inside an element counts as dynamic attr."""
    var result = Int(w[].call_i32("dsl_test_bind_value_in_element", no_args()))
    assert_equal(result, 1, "bind_value_in_element passed")


fn test_dsl_two_way_binding_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bind_value + oninput_set_string together produce 2 dynamic attrs."""
    var result = Int(
        w[].call_i32("dsl_test_two_way_binding_element", no_args())
    )
    assert_equal(result, 1, "two_way_binding_element passed")


fn test_dsl_bind_value_to_template(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bind_value converts to TATTR_DYNAMIC in template."""
    var result = Int(w[].call_i32("dsl_test_bind_value_to_template", no_args()))
    assert_equal(result, 1, "bind_value_to_template passed")


fn test_dsl_two_way_to_template(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bind_value + oninput_set_string converts to 2 TATTR_DYNAMICs."""
    var result = Int(w[].call_i32("dsl_test_two_way_to_template", no_args()))
    assert_equal(result, 1, "two_way_to_template passed")


# ══════════════════════════════════════════════════════════════════════════════
# Phase 20 — M20.5: onclick_custom
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_onclick_custom_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Onclick_custom creates NODE_EVENT with ACTION_CUSTOM."""
    var result = Int(w[].call_i32("dsl_test_onclick_custom_node", no_args()))
    assert_equal(result, 1, "onclick_custom_node passed")


fn test_dsl_onclick_custom_in_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Onclick_custom inside a button counts as dynamic attr."""
    var result = Int(
        w[].call_i32("dsl_test_onclick_custom_in_element", no_args())
    )
    assert_equal(result, 1, "onclick_custom_in_element passed")


fn test_dsl_onclick_custom_with_binding(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Onclick_custom + bind_value + oninput_set_string in sibling elements."""
    var result = Int(
        w[].call_i32("dsl_test_onclick_custom_with_binding", no_args())
    )
    assert_equal(result, 1, "onclick_custom_with_binding passed")


# ══════════════════════════════════════════════════════════════════════════════
# Phase 22: onkeydown_enter_custom
# ══════════════════════════════════════════════════════════════════════════════


fn test_dsl_onkeydown_enter_custom_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Onkeydown_enter_custom creates NODE_EVENT with ACTION_KEY_ENTER_CUSTOM.
    """
    var result = Int(
        w[].call_i32("dsl_test_onkeydown_enter_custom_node", no_args())
    )
    assert_equal(result, 1, "onkeydown_enter_custom_node passed")


fn test_dsl_onkeydown_enter_custom_in_element(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Onkeydown_enter_custom inside an input counts as dynamic attr."""
    var result = Int(
        w[].call_i32("dsl_test_onkeydown_enter_custom_in_element", no_args())
    )
    assert_equal(result, 1, "onkeydown_enter_custom_in_element passed")


fn test_dsl_onkeydown_enter_custom_with_binding(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Onkeydown_enter_custom + bind_value + oninput + onclick_custom (Phase 22 TodoApp pattern).
    """
    var result = Int(
        w[].call_i32("dsl_test_onkeydown_enter_custom_with_binding", no_args())
    )
    assert_equal(result, 1, "onkeydown_enter_custom_with_binding passed")


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_dsl_text_node(w)
    test_dsl_dyn_text_node(w)
    test_dsl_dyn_node_slot(w)
    test_dsl_static_attr(w)
    test_dsl_dyn_attr(w)
    test_dsl_empty_element(w)
    test_dsl_element_with_children(w)
    test_dsl_element_with_attrs(w)
    test_dsl_element_mixed(w)
    test_dsl_nested_elements(w)
    test_dsl_all_tag_helpers(w)
    test_dsl_count_utilities(w)
    test_dsl_to_template_simple(w)
    test_dsl_to_template_attrs(w)
    test_dsl_to_template_multi_root(w)
    test_dsl_counter_template(w)
    test_dsl_template_equivalence(w)
    test_dsl_vnode_builder(w)
    test_dsl_vnode_builder_keyed(w)
    test_dsl_node_create_text(w)
    test_dsl_node_create_dyn_text(w)
    test_dsl_node_create_dyn_node(w)
    test_dsl_node_create_attr(w)
    test_dsl_node_create_dyn_attr(w)
    test_dsl_node_create_element(w)
    test_dsl_node_add_items(w)
    test_dsl_node_nested_tree(w)
    test_dsl_node_count_dyn_node_and_static_attr(w)
    test_dsl_node_to_template(w)
    test_dsl_vb_create_and_query(w)
    test_dsl_vb_keyed(w)
    test_dsl_template_equivalence_via_wasm(w)
    test_dsl_counter_template_via_wasm(w)
    test_dsl_multi_root_via_wasm(w)
    # Phase 20 — M20.3: oninput_set_string / onchange_set_string
    test_dsl_oninput_set_string_node(w)
    test_dsl_onchange_set_string_node(w)
    test_dsl_oninput_in_element(w)
    # Phase 20 — M20.4: bind_value / bind_attr
    test_dsl_bind_value_node(w)
    test_dsl_bind_attr_node(w)
    test_dsl_bind_value_in_element(w)
    test_dsl_two_way_binding_element(w)
    test_dsl_bind_value_to_template(w)
    test_dsl_two_way_to_template(w)
    # Phase 20 — M20.5: onclick_custom
    test_dsl_onclick_custom_node(w)
    test_dsl_onclick_custom_in_element(w)
    test_dsl_onclick_custom_with_binding(w)
    # Phase 22: onkeydown_enter_custom
    test_dsl_onkeydown_enter_custom_node(w)
    test_dsl_onkeydown_enter_custom_in_element(w)
    test_dsl_onkeydown_enter_custom_with_binding(w)
    print("dsl: 49/49 passed")
