# TemplateBuilder, TemplateRegistry, and VNodeStore exercised through the real
# WASM binary via mojo-wasmtime (pure Mojo FFI bindings — no Python interop
# required).
#
# These tests verify that the template system (builder, registry, VNode store)
# works correctly when compiled to WASM and executed via the Wasmtime runtime.
#
# Run with:
#   mojo test test/test_templates.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true, assert_false

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    args_ptr_i32_ptr,
    args_ptr_i32_ptr_ptr,
    args_ptr_i32_ptr_ptr_i32,
    args_ptr_i32_ptr_i32_i32,
    args_ptr_i32_ptr_i32,
    args_ptr_ptr,
    args_ptr_ptr_i32,
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


fn _create_builder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], name: String
) raises -> Int:
    return Int(
        w[].call_i64(
            "tmpl_builder_create", args_ptr(w[].write_string_struct(name))
        )
    )


fn _destroy_builder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], b: Int
) raises:
    w[].call_void("tmpl_builder_destroy", args_ptr(b))


fn _create_vnode_store(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises -> Int:
    return Int(w[].call_i64("vnode_store_create", no_args()))


fn _destroy_vnode_store(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], s: Int
) raises:
    w[].call_void("vnode_store_destroy", args_ptr(s))


# ── Constants (matching src/vdom) ────────────────────────────────────────────
# Template node kinds
comptime TNODE_ELEMENT = 0
comptime TNODE_TEXT = 1
comptime TNODE_DYNAMIC = 2
comptime TNODE_DYNAMIC_TEXT = 3
# Template attribute kinds
comptime TATTR_STATIC = 0
comptime TATTR_DYNAMIC = 1
# VNode kinds
comptime VNODE_TEMPLATE_REF = 0
comptime VNODE_TEXT = 1
comptime VNODE_PLACEHOLDER = 2
comptime VNODE_FRAGMENT = 3
# AttributeValue kinds
comptime AVAL_TEXT = 0
comptime AVAL_INT = 1
comptime AVAL_FLOAT = 2
comptime AVAL_BOOL = 3
comptime AVAL_EVENT = 4
comptime AVAL_NONE = 5
# Dynamic node kinds
comptime DNODE_TEXT = 0
comptime DNODE_PLACEHOLDER = 1
# HTML tag constants
comptime TAG_DIV = 0
comptime TAG_SPAN = 1
comptime TAG_P = 2
comptime TAG_H1 = 3
comptime TAG_H2 = 4
comptime TAG_H3 = 5
comptime TAG_H4 = 6
comptime TAG_H5 = 7
comptime TAG_H6 = 8
comptime TAG_UL = 9
comptime TAG_OL = 10
comptime TAG_LI = 11
comptime TAG_BUTTON = 12
comptime TAG_INPUT = 13
comptime TAG_FORM = 14
comptime TAG_A = 15
comptime TAG_IMG = 16
comptime TAG_TABLE = 17
comptime TAG_TR = 18
comptime TAG_TD = 19
comptime TAG_TH = 20


# ══════════════════════════════════════════════════════════════════════════════
# Template Builder — basic lifecycle
# ══════════════════════════════════════════════════════════════════════════════


fn test_builder_basic_lifecycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)
    var b = _create_builder(w, "test-basic")

    # Push a single div root
    var div_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    assert_equal(div_idx, 0, "first element is at index 0")

    # Push a child span inside the div
    var span_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_SPAN, div_idx)
        )
    )
    assert_equal(span_idx, 1, "second element is at index 1")

    # Push a text node inside the span
    var text_ptr = w[].write_string_struct("hello")
    var text_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_text", args_ptr_ptr_i32(b, text_ptr, span_idx)
        )
    )
    assert_equal(text_idx, 2, "text node is at index 2")

    # Check builder counts
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))),
        3,
        "builder has 3 nodes",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        1,
        "builder has 1 root",
    )

    # Register
    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    assert_equal(tmpl_id, 0, "first template gets ID 0")
    assert_equal(
        Int(w[].call_i32("tmpl_count", args_ptr(rt))),
        1,
        "1 template registered",
    )

    _destroy_builder(w, b)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Template Registry — register and query
# ══════════════════════════════════════════════════════════════════════════════


fn test_registry_register_and_query(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    # Register first template
    var b1 = _create_builder(w, "alpha")
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b1, TAG_DIV, -1)
    )
    var id1 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b1)))
    assert_equal(id1, 0, "alpha gets ID 0")
    _destroy_builder(w, b1)

    # Register second template
    var b2 = _create_builder(w, "beta")
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b2, TAG_SPAN, -1)
    )
    var id2 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b2)))
    assert_equal(id2, 1, "beta gets ID 1")
    _destroy_builder(w, b2)

    assert_equal(
        Int(w[].call_i32("tmpl_count", args_ptr(rt))),
        2,
        "2 templates registered",
    )

    # Look up by name
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_contains_name",
                args_ptr_ptr(rt, w[].write_string_struct("alpha")),
            )
        ),
        1,
        "contains 'alpha'",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_contains_name",
                args_ptr_ptr(rt, w[].write_string_struct("beta")),
            )
        ),
        1,
        "contains 'beta'",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_contains_name",
                args_ptr_ptr(rt, w[].write_string_struct("gamma")),
            )
        ),
        0,
        "does not contain 'gamma'",
    )

    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_find_by_name",
                args_ptr_ptr(rt, w[].write_string_struct("alpha")),
            )
        ),
        0,
        "find 'alpha' -> 0",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_find_by_name",
                args_ptr_ptr(rt, w[].write_string_struct("beta")),
            )
        ),
        1,
        "find 'beta' -> 1",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_find_by_name",
                args_ptr_ptr(rt, w[].write_string_struct("gamma")),
            )
        ),
        -1,
        "find 'gamma' -> -1",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Template Structure — node queries
# ══════════════════════════════════════════════════════════════════════════════


fn test_template_structure_node_queries(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    # Build: div > (h1 > "Title", p > "Body")
    var b = _create_builder(w, "structure")
    var div_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    var h1_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_H1, div_idx)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("Title"), h1_idx),
    )
    var p_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_P, div_idx)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("Body"), p_idx),
    )

    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    # Roots
    assert_equal(
        Int(w[].call_i32("tmpl_root_count", args_ptr_i32(rt, tmpl_id))),
        1,
        "1 root",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_get_root_index", args_ptr_i32_i32(rt, tmpl_id, 0)
            )
        ),
        0,
        "root is node 0",
    )

    # Total nodes: div + h1 + "Title" + p + "Body" = 5
    assert_equal(
        Int(w[].call_i32("tmpl_node_count", args_ptr_i32(rt, tmpl_id))),
        5,
        "5 nodes total",
    )

    # div (node 0) is Element with TAG_DIV, 2 children
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 0))),
        TNODE_ELEMENT,
        "node 0 is Element",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, tmpl_id, 0))),
        TAG_DIV,
        "node 0 tag is DIV",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 0)
            )
        ),
        2,
        "div has 2 children",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_at", args_ptr_i32_i32_i32(rt, tmpl_id, 0, 0)
            )
        ),
        1,
        "div child 0 is node 1 (h1)",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_at", args_ptr_i32_i32_i32(rt, tmpl_id, 0, 1)
            )
        ),
        3,
        "div child 1 is node 3 (p)",
    )

    # h1 (node 1)
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 1))),
        TNODE_ELEMENT,
        "node 1 is Element",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, tmpl_id, 1))),
        TAG_H1,
        "node 1 tag is H1",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 1)
            )
        ),
        1,
        "h1 has 1 child",
    )

    # "Title" (node 2) is Text
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 2))),
        TNODE_TEXT,
        "node 2 is Text",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 2)
            )
        ),
        0,
        "text node has 0 children",
    )

    # p (node 3)
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 3))),
        TNODE_ELEMENT,
        "node 3 is Element",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, tmpl_id, 3))),
        TAG_P,
        "node 3 tag is P",
    )

    # "Body" (node 4) is Text
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 4))),
        TNODE_TEXT,
        "node 4 is Text",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Template Dynamic Slots — Dynamic and DynamicText nodes
# ══════════════════════════════════════════════════════════════════════════════


fn test_template_dynamic_slots(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    # Build: div > (dyntext[0], "static", dyn[0], dyntext[1])
    var b = _create_builder(w, "dynamic-slots")
    var div_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 0, div_idx)
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("static"), div_idx),
    )
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic", args_ptr_i32_i32(b, 0, div_idx)
    )
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 1, div_idx)
    )

    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    # Node count: div + dyntext0 + "static" + dyn0 + dyntext1 = 5
    assert_equal(
        Int(w[].call_i32("tmpl_node_count", args_ptr_i32(rt, tmpl_id))),
        5,
        "5 nodes",
    )

    # DynamicText node at index 1
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 1))),
        TNODE_DYNAMIC_TEXT,
        "node 1 is DynamicText",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_dynamic_index", args_ptr_i32_i32(rt, tmpl_id, 1)
            )
        ),
        0,
        "dyntext 0 has index 0",
    )

    # Static text at index 2
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 2))),
        TNODE_TEXT,
        "node 2 is Text",
    )

    # Dynamic node at index 3
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 3))),
        TNODE_DYNAMIC,
        "node 3 is Dynamic",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_dynamic_index", args_ptr_i32_i32(rt, tmpl_id, 3)
            )
        ),
        0,
        "dynamic 0 has index 0",
    )

    # DynamicText at index 4
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 4))),
        TNODE_DYNAMIC_TEXT,
        "node 4 is DynamicText",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_dynamic_index", args_ptr_i32_i32(rt, tmpl_id, 4)
            )
        ),
        1,
        "dyntext 1 has index 1",
    )

    # Slot counts
    assert_equal(
        Int(w[].call_i32("tmpl_dynamic_node_count", args_ptr_i32(rt, tmpl_id))),
        1,
        "1 Dynamic node slot",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_dynamic_text_count", args_ptr_i32(rt, tmpl_id))),
        2,
        "2 DynamicText slots",
    )

    # div has 4 children
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 0)
            )
        ),
        4,
        "div has 4 children",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Template Attributes — static and dynamic
# ══════════════════════════════════════════════════════════════════════════════


fn test_template_attributes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    # Build: button with class="btn", id="submit", and one dynamic attr
    var b = _create_builder(w, "attrs")
    var btn_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_BUTTON, -1)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("Click me"), btn_idx),
    )

    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(
            b,
            btn_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("btn"),
        ),
    )
    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(
            b,
            btn_idx,
            w[].write_string_struct("id"),
            w[].write_string_struct("submit"),
        ),
    )
    w[].call_void(
        "tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, btn_idx, 0)
    )

    assert_equal(
        Int(w[].call_i32("tmpl_builder_attr_count", args_ptr(b))),
        3,
        "builder has 3 attrs",
    )

    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    # Total attributes
    assert_equal(
        Int(w[].call_i32("tmpl_attr_total_count", args_ptr_i32(rt, tmpl_id))),
        3,
        "template has 3 attrs total",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_static_attr_count", args_ptr_i32(rt, tmpl_id))),
        2,
        "2 static attrs",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_dynamic_attr_count", args_ptr_i32(rt, tmpl_id))),
        1,
        "1 dynamic attr",
    )

    # Node-level attr count
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_attr_count", args_ptr_i32_i32(rt, tmpl_id, 0)
            )
        ),
        3,
        "button has 3 attrs",
    )

    # Attr kinds
    assert_equal(
        Int(w[].call_i32("tmpl_attr_kind", args_ptr_i32_i32(rt, tmpl_id, 0))),
        TATTR_STATIC,
        "attr 0 is static",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_attr_kind", args_ptr_i32_i32(rt, tmpl_id, 1))),
        TATTR_STATIC,
        "attr 1 is static",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_attr_kind", args_ptr_i32_i32(rt, tmpl_id, 2))),
        TATTR_DYNAMIC,
        "attr 2 is dynamic",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_attr_dynamic_index", args_ptr_i32_i32(rt, tmpl_id, 2)
            )
        ),
        0,
        "dynamic attr index is 0",
    )

    # Node first attr
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_first_attr", args_ptr_i32_i32(rt, tmpl_id, 0)
            )
        ),
        0,
        "button first attr at index 0",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Template Deduplication — same name returns same ID
# ══════════════════════════════════════════════════════════════════════════════


fn test_template_deduplication(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    # Register "counter" template
    var b1 = _create_builder(w, "counter")
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b1, TAG_DIV, -1)
    )
    var id1 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b1)))
    _destroy_builder(w, b1)

    # Register another template with the SAME name
    var b2 = _create_builder(w, "counter")
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b2, TAG_SPAN, -1)
    )
    var id2 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b2)))
    _destroy_builder(w, b2)

    assert_equal(id1, id2, "same name -> same ID (deduplicated)")
    assert_equal(
        Int(w[].call_i32("tmpl_count", args_ptr(rt))),
        1,
        "still only 1 template registered",
    )

    # The original template's structure is preserved (div, not span)
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id1, 0))),
        TAG_DIV,
        "original template structure preserved",
    )

    # Register a different name
    var b3 = _create_builder(w, "todo-item")
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b3, TAG_LI, -1)
    )
    var id3 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b3)))
    _destroy_builder(w, b3)

    assert_equal(id3, 1, "different name gets new ID")
    assert_equal(
        Int(w[].call_i32("tmpl_count", args_ptr(rt))),
        2,
        "now 2 templates registered",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# VNode Creation — basic kinds
# ══════════════════════════════════════════════════════════════════════════════


fn test_vnode_creation_basic_kinds(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var store = _create_vnode_store(w)

    # TemplateRef
    var tr = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(store, 42))
    )
    assert_equal(tr, 0, "first vnode at index 0")
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, tr))),
        VNODE_TEMPLATE_REF,
        "kind is TemplateRef",
    )
    assert_equal(
        Int(w[].call_i32("vnode_template_id", args_ptr_i32(store, tr))),
        42,
        "template_id is 42",
    )
    assert_equal(
        Int(w[].call_i32("vnode_has_key", args_ptr_i32(store, tr))), 0, "no key"
    )
    assert_equal(
        Int(w[].call_i32("vnode_dynamic_node_count", args_ptr_i32(store, tr))),
        0,
        "0 dynamic nodes",
    )
    assert_equal(
        Int(w[].call_i32("vnode_dynamic_attr_count", args_ptr_i32(store, tr))),
        0,
        "0 dynamic attrs",
    )

    # Text
    var txt = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(store, w[].write_string_struct("hello world")),
        )
    )
    assert_equal(txt, 1, "second vnode at index 1")
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, txt))),
        VNODE_TEXT,
        "kind is Text",
    )

    # Placeholder
    var ph = Int(
        w[].call_i32("vnode_push_placeholder", args_ptr_i32(store, 99))
    )
    assert_equal(ph, 2, "third vnode at index 2")
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, ph))),
        VNODE_PLACEHOLDER,
        "kind is Placeholder",
    )
    assert_equal(
        Int(w[].call_i32("vnode_element_id", args_ptr_i32(store, ph))),
        99,
        "element_id is 99",
    )

    # Fragment
    var frag = Int(w[].call_i32("vnode_push_fragment", args_ptr(store)))
    assert_equal(frag, 3, "fourth vnode at index 3")
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, frag))),
        VNODE_FRAGMENT,
        "kind is Fragment",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_fragment_child_count", args_ptr_i32(store, frag)
            )
        ),
        0,
        "empty fragment",
    )

    # Total count
    assert_equal(
        Int(w[].call_i32("vnode_count", args_ptr(store))),
        4,
        "4 vnodes in store",
    )

    _destroy_vnode_store(w, store)


# ══════════════════════════════════════════════════════════════════════════════
# VNode Dynamic Content — nodes and attrs on TemplateRef
# ══════════════════════════════════════════════════════════════════════════════


fn test_vnode_dynamic_content(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var store = _create_vnode_store(w)

    # Create a TemplateRef VNode
    var vn = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(store, 0))
    )

    # Add dynamic text nodes
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(store, vn, w[].write_string_struct("Count: 5")),
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(store, vn, w[].write_string_struct("Total: 10")),
    )

    assert_equal(
        Int(w[].call_i32("vnode_dynamic_node_count", args_ptr_i32(store, vn))),
        2,
        "2 dynamic nodes",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_node_kind", args_ptr_i32_i32(store, vn, 0)
            )
        ),
        DNODE_TEXT,
        "dyn node 0 is Text",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_node_kind", args_ptr_i32_i32(store, vn, 1)
            )
        ),
        DNODE_TEXT,
        "dyn node 1 is Text",
    )

    # Add a dynamic placeholder
    w[].call_void("vnode_push_dynamic_placeholder", args_ptr_i32(store, vn))
    assert_equal(
        Int(w[].call_i32("vnode_dynamic_node_count", args_ptr_i32(store, vn))),
        3,
        "3 dynamic nodes",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_node_kind", args_ptr_i32_i32(store, vn, 2)
            )
        ),
        DNODE_PLACEHOLDER,
        "dyn node 2 is Placeholder",
    )

    # Add dynamic text attribute
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            store,
            vn,
            w[].write_string_struct("class"),
            w[].write_string_struct("active"),
            5,
        ),
    )
    assert_equal(
        Int(w[].call_i32("vnode_dynamic_attr_count", args_ptr_i32(store, vn))),
        1,
        "1 dynamic attr",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_kind", args_ptr_i32_i32(store, vn, 0)
            )
        ),
        AVAL_TEXT,
        "attr 0 is text",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_element_id",
                args_ptr_i32_i32(store, vn, 0),
            )
        ),
        5,
        "attr 0 elem_id is 5",
    )

    _destroy_vnode_store(w, store)


# ══════════════════════════════════════════════════════════════════════════════
# VNode Fragments — children
# ══════════════════════════════════════════════════════════════════════════════


fn test_vnode_fragments(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var store = _create_vnode_store(w)

    # Create child vnodes
    var txt1 = Int(
        w[].call_i32(
            "vnode_push_text", args_ptr_ptr(store, w[].write_string_struct("A"))
        )
    )
    var txt2 = Int(
        w[].call_i32(
            "vnode_push_text", args_ptr_ptr(store, w[].write_string_struct("B"))
        )
    )
    var txt3 = Int(
        w[].call_i32(
            "vnode_push_text", args_ptr_ptr(store, w[].write_string_struct("C"))
        )
    )

    # Create fragment and add children
    var frag = Int(w[].call_i32("vnode_push_fragment", args_ptr(store)))
    w[].call_void(
        "vnode_push_fragment_child", args_ptr_i32_i32(store, frag, txt1)
    )
    w[].call_void(
        "vnode_push_fragment_child", args_ptr_i32_i32(store, frag, txt2)
    )
    w[].call_void(
        "vnode_push_fragment_child", args_ptr_i32_i32(store, frag, txt3)
    )

    assert_equal(
        Int(
            w[].call_i32(
                "vnode_fragment_child_count", args_ptr_i32(store, frag)
            )
        ),
        3,
        "fragment has 3 children",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_fragment_child_at", args_ptr_i32_i32(store, frag, 0)
            )
        ),
        0,
        "child 0 is vnode 0",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_fragment_child_at", args_ptr_i32_i32(store, frag, 1)
            )
        ),
        1,
        "child 1 is vnode 1",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_fragment_child_at", args_ptr_i32_i32(store, frag, 2)
            )
        ),
        2,
        "child 2 is vnode 2",
    )

    # Verify children are text nodes
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, txt1))),
        VNODE_TEXT,
        "child 0 is Text",
    )
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, txt2))),
        VNODE_TEXT,
        "child 1 is Text",
    )
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, txt3))),
        VNODE_TEXT,
        "child 2 is Text",
    )

    _destroy_vnode_store(w, store)


# ══════════════════════════════════════════════════════════════════════════════
# VNode Keys — keyed TemplateRef
# ══════════════════════════════════════════════════════════════════════════════


fn test_vnode_keys(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var store = _create_vnode_store(w)

    # Unkeyed
    var vn1 = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(store, 0))
    )
    assert_equal(
        Int(w[].call_i32("vnode_has_key", args_ptr_i32(store, vn1))),
        0,
        "unkeyed vnode has no key",
    )

    # Keyed
    var vn2 = Int(
        w[].call_i32(
            "vnode_push_template_ref_keyed",
            args_ptr_i32_ptr(store, 0, w[].write_string_struct("item-42")),
        )
    )
    assert_equal(
        Int(w[].call_i32("vnode_has_key", args_ptr_i32(store, vn2))),
        1,
        "keyed vnode has key",
    )

    # Both are TemplateRef
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, vn1))),
        VNODE_TEMPLATE_REF,
        "vn1 is TemplateRef",
    )
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, vn2))),
        VNODE_TEMPLATE_REF,
        "vn2 is TemplateRef",
    )

    _destroy_vnode_store(w, store)


# ══════════════════════════════════════════════════════════════════════════════
# VNode Mixed Attributes — text, int, bool, event, none
# ══════════════════════════════════════════════════════════════════════════════


fn test_vnode_mixed_attributes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var store = _create_vnode_store(w)
    var vn = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(store, 0))
    )

    # Text attribute
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            store,
            vn,
            w[].write_string_struct("class"),
            w[].write_string_struct("btn-primary"),
            1,
        ),
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_kind", args_ptr_i32_i32(store, vn, 0)
            )
        ),
        AVAL_TEXT,
        "attr 0 is text",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_element_id",
                args_ptr_i32_i32(store, vn, 0),
            )
        ),
        1,
        "attr 0 elem_id",
    )

    # Int attribute
    w[].call_void(
        "vnode_push_dynamic_attr_int",
        args_ptr_i32_ptr_i32_i32(
            store, vn, w[].write_string_struct("tabindex"), 3, 2
        ),
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_kind", args_ptr_i32_i32(store, vn, 1)
            )
        ),
        AVAL_INT,
        "attr 1 is int",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_element_id",
                args_ptr_i32_i32(store, vn, 1),
            )
        ),
        2,
        "attr 1 elem_id",
    )

    # Bool attribute
    w[].call_void(
        "vnode_push_dynamic_attr_bool",
        args_ptr_i32_ptr_i32_i32(
            store, vn, w[].write_string_struct("disabled"), 1, 3
        ),
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_kind", args_ptr_i32_i32(store, vn, 2)
            )
        ),
        AVAL_BOOL,
        "attr 2 is bool",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_element_id",
                args_ptr_i32_i32(store, vn, 2),
            )
        ),
        3,
        "attr 2 elem_id",
    )

    # Event handler
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            store, vn, w[].write_string_struct("onclick"), 77, 4
        ),
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_kind", args_ptr_i32_i32(store, vn, 3)
            )
        ),
        AVAL_EVENT,
        "attr 3 is event",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_element_id",
                args_ptr_i32_i32(store, vn, 3),
            )
        ),
        4,
        "attr 3 elem_id",
    )

    # None (removal)
    w[].call_void(
        "vnode_push_dynamic_attr_none",
        args_ptr_i32_ptr_i32(store, vn, w[].write_string_struct("hidden"), 5),
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_kind", args_ptr_i32_i32(store, vn, 4)
            )
        ),
        AVAL_NONE,
        "attr 4 is none",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "vnode_get_dynamic_attr_element_id",
                args_ptr_i32_i32(store, vn, 4),
            )
        ),
        5,
        "attr 4 elem_id",
    )

    assert_equal(
        Int(w[].call_i32("vnode_dynamic_attr_count", args_ptr_i32(store, vn))),
        5,
        "5 dynamic attrs total",
    )

    _destroy_vnode_store(w, store)


# ══════════════════════════════════════════════════════════════════════════════
# VNode Store Lifecycle — create, populate, clear, repopulate
# ══════════════════════════════════════════════════════════════════════════════


fn test_vnode_store_lifecycle(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var store = _create_vnode_store(w)

    # Add some nodes
    _ = w[].call_i32(
        "vnode_push_text", args_ptr_ptr(store, w[].write_string_struct("A"))
    )
    _ = w[].call_i32(
        "vnode_push_text", args_ptr_ptr(store, w[].write_string_struct("B"))
    )
    _ = w[].call_i32("vnode_push_placeholder", args_ptr_i32(store, 1))
    assert_equal(
        Int(w[].call_i32("vnode_count", args_ptr(store))),
        3,
        "3 vnodes before clear",
    )

    # Clear
    w[].call_void("vnode_store_clear", args_ptr(store))
    assert_equal(
        Int(w[].call_i32("vnode_count", args_ptr(store))),
        0,
        "0 vnodes after clear",
    )

    # Repopulate
    var idx = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(store, 5))
    )
    assert_equal(idx, 0, "indices restart at 0 after clear")
    assert_equal(
        Int(w[].call_i32("vnode_count", args_ptr(store))),
        1,
        "1 vnode after repopulate",
    )
    assert_equal(
        Int(w[].call_i32("vnode_kind", args_ptr_i32(store, idx))),
        VNODE_TEMPLATE_REF,
        "repopulated vnode is TemplateRef",
    )

    _destroy_vnode_store(w, store)


# ══════════════════════════════════════════════════════════════════════════════
# Template Builder — pre-build queries
# ══════════════════════════════════════════════════════════════════════════════


fn test_builder_pre_build_queries(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var b = _create_builder(w, "query-test")

    # Empty builder
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))),
        0,
        "empty: 0 nodes",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        0,
        "empty: 0 roots",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_attr_count", args_ptr(b))),
        0,
        "empty: 0 attrs",
    )

    # Add nodes and check counts incrementally
    var r1 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_UL, -1)
        )
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))),
        1,
        "1 node after push",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        1,
        "1 root after root push",
    )

    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_LI, r1)
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))), 2, "2 nodes"
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        1,
        "still 1 root",
    )

    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("item"), 1),
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))), 3, "3 nodes"
    )

    # Add another root
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_P, -1)
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        2,
        "2 roots now",
    )

    # Add an attribute
    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(
            b,
            r1,
            w[].write_string_struct("class"),
            w[].write_string_struct("list"),
        ),
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_attr_count", args_ptr(b))), 1, "1 attr"
    )

    w[].call_void("tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, r1, 0))
    assert_equal(
        Int(w[].call_i32("tmpl_builder_attr_count", args_ptr(b))), 2, "2 attrs"
    )

    _destroy_builder(w, b)


# ══════════════════════════════════════════════════════════════════════════════
# Complex Template — counter-like structure
# ══════════════════════════════════════════════════════════════════════════════


fn test_complex_template_counter(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    # Build a counter template:
    # div.counter
    #   h1 > dyntext[0]  ("Count: N")
    #   button > "+"      (onclick = dynamic attr 0)
    #   button > "-"      (onclick = dynamic attr 1)
    var b = _create_builder(w, "counter-complex")
    var div = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(
            b,
            div,
            w[].write_string_struct("class"),
            w[].write_string_struct("counter"),
        ),
    )

    var h1 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_H1, div)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 0, h1)
    )

    var btn1 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_BUTTON, div)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("+"), btn1),
    )
    w[].call_void(
        "tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, btn1, 0)
    )

    var btn2 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_BUTTON, div)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("-"), btn2),
    )
    w[].call_void(
        "tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, btn2, 1)
    )

    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    # Verify structure
    # Nodes: div(0), h1(1), dyntext(2), btn1(3), "+"(4), btn2(5), "-"(6) = 7
    assert_equal(
        Int(w[].call_i32("tmpl_node_count", args_ptr_i32(rt, tmpl_id))),
        7,
        "7 nodes in counter template",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_root_count", args_ptr_i32(rt, tmpl_id))),
        1,
        "1 root (div)",
    )

    # div children: h1, btn1, btn2
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 0)
            )
        ),
        3,
        "div has 3 children",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_at", args_ptr_i32_i32_i32(rt, tmpl_id, 0, 0)
            )
        ),
        1,
        "div child 0 = h1",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_at", args_ptr_i32_i32_i32(rt, tmpl_id, 0, 1)
            )
        ),
        3,
        "div child 1 = btn1",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_at", args_ptr_i32_i32_i32(rt, tmpl_id, 0, 2)
            )
        ),
        5,
        "div child 2 = btn2",
    )

    # h1 has 1 child: dyntext[0]
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 1)
            )
        ),
        1,
        "h1 has 1 child",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 2))),
        TNODE_DYNAMIC_TEXT,
        "h1 child is DynamicText",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_dynamic_index", args_ptr_i32_i32(rt, tmpl_id, 2)
            )
        ),
        0,
        "dyntext index 0",
    )

    # btn1 has 1 child: "+"
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, tmpl_id, 3))),
        TAG_BUTTON,
        "btn1 is BUTTON",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 3)
            )
        ),
        1,
        "btn1 has 1 child",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 4))),
        TNODE_TEXT,
        "btn1 child is Text",
    )

    # btn2 has 1 child: "-"
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, tmpl_id, 5))),
        TAG_BUTTON,
        "btn2 is BUTTON",
    )
    assert_equal(
        Int(
            w[].call_i32(
                "tmpl_node_child_count", args_ptr_i32_i32(rt, tmpl_id, 5)
            )
        ),
        1,
        "btn2 has 1 child",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_kind", args_ptr_i32_i32(rt, tmpl_id, 6))),
        TNODE_TEXT,
        "btn2 child is Text",
    )

    # Attribute counts
    assert_equal(
        Int(w[].call_i32("tmpl_static_attr_count", args_ptr_i32(rt, tmpl_id))),
        1,
        "1 static attr (class)",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_dynamic_attr_count", args_ptr_i32(rt, tmpl_id))),
        2,
        "2 dynamic attrs (onclick x2)",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_attr_total_count", args_ptr_i32(rt, tmpl_id))),
        3,
        "3 attrs total",
    )

    # Dynamic slot counts
    assert_equal(
        Int(w[].call_i32("tmpl_dynamic_text_count", args_ptr_i32(rt, tmpl_id))),
        1,
        "1 dynamic text slot",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_dynamic_node_count", args_ptr_i32(rt, tmpl_id))),
        0,
        "0 dynamic node slots",
    )

    # Now create a VNode that instantiates this template
    var store = _create_vnode_store(w)
    var vn = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(store, tmpl_id))
    )

    # Fill dynamic text: "Count: 5"
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(store, vn, w[].write_string_struct("Count: 5")),
    )

    # Fill dynamic attrs: onclick handlers
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            store, vn, w[].write_string_struct("onclick"), 1, 3
        ),
    )
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            store, vn, w[].write_string_struct("onclick"), 2, 5
        ),
    )

    assert_equal(
        Int(w[].call_i32("vnode_dynamic_node_count", args_ptr_i32(store, vn))),
        1,
        "vnode has 1 dynamic node",
    )
    assert_equal(
        Int(w[].call_i32("vnode_dynamic_attr_count", args_ptr_i32(store, vn))),
        2,
        "vnode has 2 dynamic attrs",
    )
    assert_equal(
        Int(w[].call_i32("vnode_template_id", args_ptr_i32(store, vn))),
        tmpl_id,
        "vnode references counter template",
    )

    _destroy_vnode_store(w, store)
    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Multiple Templates — different structures in one runtime
# ══════════════════════════════════════════════════════════════════════════════


fn test_multiple_templates_in_one_runtime(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var rt = _create_runtime(w)

    # Template 1: simple div > "Hello"
    var b1 = _create_builder(w, "hello")
    var div1 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b1, TAG_DIV, -1)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b1, w[].write_string_struct("Hello"), div1),
    )
    var id1 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b1)))
    _destroy_builder(w, b1)

    # Template 2: ul > li > "Item 1", li > "Item 2"
    var b2 = _create_builder(w, "list")
    var ul = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b2, TAG_UL, -1)
        )
    )
    var li1 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b2, TAG_LI, ul)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b2, w[].write_string_struct("Item 1"), li1),
    )
    var li2 = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b2, TAG_LI, ul)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b2, w[].write_string_struct("Item 2"), li2),
    )
    var id2 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b2)))
    _destroy_builder(w, b2)

    # Template 3: form > input + button
    var b3 = _create_builder(w, "form")
    var form = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b3, TAG_FORM, -1)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b3, TAG_INPUT, form)
    )
    var submit_btn = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b3, TAG_BUTTON, form)
        )
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b3, w[].write_string_struct("Submit"), submit_btn),
    )
    var id3 = Int(w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b3)))
    _destroy_builder(w, b3)

    # Verify IDs
    assert_equal(id1, 0, "hello gets ID 0")
    assert_equal(id2, 1, "list gets ID 1")
    assert_equal(id3, 2, "form gets ID 2")
    assert_equal(
        Int(w[].call_i32("tmpl_count", args_ptr(rt))),
        3,
        "3 templates registered",
    )

    # Cross-template queries
    assert_equal(
        Int(w[].call_i32("tmpl_node_count", args_ptr_i32(rt, id1))),
        2,
        "hello has 2 nodes",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_count", args_ptr_i32(rt, id2))),
        5,
        "list has 5 nodes",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_count", args_ptr_i32(rt, id3))),
        4,
        "form has 4 nodes",
    )

    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id1, 0))),
        TAG_DIV,
        "hello root is div",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id2, 0))),
        TAG_UL,
        "list root is ul",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id3, 0))),
        TAG_FORM,
        "form root is form",
    )

    # Verify list children
    assert_equal(
        Int(
            w[].call_i32("tmpl_node_child_count", args_ptr_i32_i32(rt, id2, 0))
        ),
        2,
        "ul has 2 children",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id2, 1))),
        TAG_LI,
        "first child is li",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id2, 3))),
        TAG_LI,
        "second child is li",
    )

    # Verify form children
    assert_equal(
        Int(
            w[].call_i32("tmpl_node_child_count", args_ptr_i32_i32(rt, id3, 0))
        ),
        2,
        "form has 2 children",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id3, 1))),
        TAG_INPUT,
        "first child is input",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_node_tag", args_ptr_i32_i32(rt, id3, 2))),
        TAG_BUTTON,
        "second child is button",
    )

    _destroy_runtime(w, rt)


# ══════════════════════════════════════════════════════════════════════════════
# Builder Reset — builder is empty after build()
# ══════════════════════════════════════════════════════════════════════════════


fn test_builder_reset_after_build(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var rt = _create_runtime(w)

    var b = _create_builder(w, "reset-test")
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
    )
    _ = w[].call_i32(
        "tmpl_builder_push_text",
        args_ptr_ptr_i32(b, w[].write_string_struct("hello"), 0),
    )
    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(
            b,
            0,
            w[].write_string_struct("class"),
            w[].write_string_struct("x"),
        ),
    )

    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))),
        2,
        "2 nodes before build",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        1,
        "1 root before build",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_attr_count", args_ptr(b))),
        1,
        "1 attr before build",
    )

    _ = w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))

    # After build, builder should be reset/empty
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))),
        0,
        "0 nodes after build",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_root_count", args_ptr(b))),
        0,
        "0 roots after build",
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_attr_count", args_ptr(b))),
        0,
        "0 attrs after build",
    )

    # Can reuse the builder for a new template
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_SPAN, -1)
    )
    assert_equal(
        Int(w[].call_i32("tmpl_builder_node_count", args_ptr(b))),
        1,
        "1 node after reuse",
    )

    _destroy_builder(w, b)
    _destroy_runtime(w, rt)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_builder_basic_lifecycle(w)
    test_registry_register_and_query(w)
    test_template_structure_node_queries(w)
    test_template_dynamic_slots(w)
    test_template_attributes(w)
    test_template_deduplication(w)
    test_vnode_creation_basic_kinds(w)
    test_vnode_dynamic_content(w)
    test_vnode_fragments(w)
    test_vnode_keys(w)
    test_vnode_mixed_attributes(w)
    test_vnode_store_lifecycle(w)
    test_builder_pre_build_queries(w)
    test_complex_template_counter(w)
    test_multiple_templates_in_one_runtime(w)
    test_builder_reset_after_build(w)
    print("templates: 16/16 passed")
