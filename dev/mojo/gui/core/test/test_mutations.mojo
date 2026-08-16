# CreateEngine and DiffEngine exercised through the real WASM binary via
# mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that the create and diff engines work correctly when
# compiled to WASM and executed via the Wasmtime runtime.  Each test creates
# templates and VNodes via WASM exports, allocates a mutation buffer, runs
# the create/diff engines, then reads back mutation bytes to verify correctness.
#
# Run with:
#   mojo test test/test_mutations.mojo

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
    args_ptr_i32_ptr_ptr_i32,
    args_ptr_i32_ptr_i32,
    args_ptr_i32_ptr_i32_i32,
    args_ptr_ptr,
    args_ptr_ptr_i32,
    args_ptr_ptr_ptr_ptr_i32,
    args_ptr_ptr_ptr_ptr_i32_i32,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Constants ────────────────────────────────────────────────────────────────

comptime OP_END = 0x00
comptime OP_APPEND_CHILDREN = 0x01
comptime OP_ASSIGN_ID = 0x02
comptime OP_CREATE_PLACEHOLDER = 0x03
comptime OP_CREATE_TEXT_NODE = 0x04
comptime OP_LOAD_TEMPLATE = 0x05
comptime OP_REPLACE_WITH = 0x06
comptime OP_REPLACE_PLACEHOLDER = 0x07
comptime OP_INSERT_AFTER = 0x08
comptime OP_INSERT_BEFORE = 0x09
comptime OP_SET_ATTRIBUTE = 0x0A
comptime OP_SET_TEXT = 0x0B
comptime OP_NEW_EVENT_LISTENER = 0x0C
comptime OP_REMOVE_EVENT_LISTENER = 0x0D
comptime OP_REMOVE = 0x0E
comptime OP_PUSH_ROOT = 0x0F
comptime OP_REMOVE_ATTRIBUTE = 0x11

comptime TAG_DIV = 0
comptime TAG_SPAN = 1
comptime TAG_P = 2
comptime TAG_H1 = 3
comptime TAG_BUTTON = 12
comptime TAG_LI = 11

comptime BUF_CAP = 8192


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _read_u8(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Int:
    return Int(w[].call_i32("debug_read_byte", args_ptr_i32(buf, offset)))


fn _read_u32_le(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Int:
    var b0 = _read_u8(w, buf, offset)
    var b1 = _read_u8(w, buf, offset + 1)
    var b2 = _read_u8(w, buf, offset + 2)
    var b3 = _read_u8(w, buf, offset + 3)
    return b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)


fn _read_u16_le(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Int:
    var lo = _read_u8(w, buf, offset)
    var hi = _read_u8(w, buf, offset + 1)
    return lo | (hi << 8)


struct MutationInfo(Copyable):
    var op: Int
    var id: Int
    var id2: Int
    var id3: Int
    var text_len: Int
    var name_len: Int
    var ns: Int
    var path_len: Int
    var handler_id: Int

    fn __init__(out self, op: Int):
        self.op = op
        self.id = 0
        self.id2 = 0
        self.id3 = 0
        self.text_len = 0
        self.name_len = 0
        self.ns = 0
        self.path_len = 0
        self.handler_id = 0

    fn __copyinit__(out self, other: Self):
        self.op = other.op
        self.id = other.id
        self.id2 = other.id2
        self.id3 = other.id3
        self.text_len = other.text_len
        self.name_len = other.name_len
        self.ns = other.ns
        self.path_len = other.path_len
        self.handler_id = other.handler_id

    fn __moveinit__(out self, deinit other: Self):
        self.op = other.op
        self.id = other.id
        self.id2 = other.id2
        self.id3 = other.id3
        self.text_len = other.text_len
        self.name_len = other.name_len
        self.ns = other.ns
        self.path_len = other.path_len
        self.handler_id = other.handler_id


fn _read_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, length: Int
) raises -> List[MutationInfo]:
    """Decode all mutations from the WASM buffer up to the End sentinel."""
    var result = List[MutationInfo]()
    var pos = 0
    while pos < length:
        var op = _read_u8(w, buf, pos)
        pos += 1
        if op == OP_END:
            break
        var m = MutationInfo(op)

        if op == OP_CREATE_TEXT_NODE:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.text_len = _read_u32_le(w, buf, pos)
            pos += 4
            pos += m.text_len

        elif op == OP_CREATE_PLACEHOLDER:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_LOAD_TEMPLATE:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.id2 = _read_u32_le(w, buf, pos)
            pos += 4
            m.id3 = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_APPEND_CHILDREN:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.id2 = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_ASSIGN_ID:
            m.path_len = _read_u8(w, buf, pos)
            pos += 1
            pos += m.path_len
            m.id = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_SET_TEXT:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.text_len = _read_u32_le(w, buf, pos)
            pos += 4
            pos += m.text_len

        elif op == OP_SET_ATTRIBUTE:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.ns = _read_u8(w, buf, pos)
            pos += 1
            m.name_len = _read_u16_le(w, buf, pos)
            pos += 2
            pos += m.name_len
            m.text_len = _read_u32_le(w, buf, pos)
            pos += 4
            pos += m.text_len

        elif op == OP_NEW_EVENT_LISTENER:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.handler_id = _read_u32_le(w, buf, pos)
            pos += 4
            m.name_len = _read_u16_le(w, buf, pos)
            pos += 2
            pos += m.name_len

        elif op == OP_REMOVE_EVENT_LISTENER:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.name_len = _read_u16_le(w, buf, pos)
            pos += 2
            pos += m.name_len

        elif op == OP_REMOVE:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_PUSH_ROOT:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_REPLACE_WITH:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.id2 = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_REPLACE_PLACEHOLDER:
            m.path_len = _read_u8(w, buf, pos)
            pos += 1
            pos += m.path_len
            m.id = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_INSERT_AFTER:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.id2 = _read_u32_le(w, buf, pos)
            pos += 4

        elif op == OP_INSERT_BEFORE:
            m.id = _read_u32_le(w, buf, pos)
            pos += 4
            m.id2 = _read_u32_le(w, buf, pos)
            pos += 4

        result.append(m^)
    return result^


fn _count_op(mutations: List[MutationInfo], op: Int) -> Int:
    var count = 0
    for i in range(len(mutations)):
        if mutations[i].op == op:
            count += 1
    return count


fn _find_first(mutations: List[MutationInfo], op: Int) -> Int:
    for i in range(len(mutations)):
        if mutations[i].op == op:
            return i
    return -1


# ── Test context: manages runtime, eid alloc, vnode store, writer, buffer ────


struct WasmTestContext(Movable):
    """Manages WASM resources for a create/diff engine test."""

    var w: UnsafePointer[WasmInstance, MutExternalOrigin]
    var rt: Int
    var eid: Int
    var store: Int
    var buf: Int
    var writer: Int

    fn __init__(
        out self, w: UnsafePointer[WasmInstance, MutExternalOrigin]
    ) raises:
        self.w = w
        self.rt = Int(w[].call_i64("runtime_create", no_args()))
        self.eid = Int(w[].call_i64("eid_alloc_create", no_args()))
        self.store = Int(w[].call_i64("vnode_store_create", no_args()))
        self.buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(BUF_CAP)))
        self.writer = Int(
            w[].call_i64("writer_create", args_ptr_i32(self.buf, BUF_CAP))
        )

    fn __moveinit__(out self, deinit other: Self):
        self.w = other.w
        self.rt = other.rt
        self.eid = other.eid
        self.store = other.store
        self.buf = other.buf
        self.writer = other.writer

    fn finalize_and_read(mut self) raises -> List[MutationInfo]:
        """Finalize the writer and read back all mutations."""
        var offset = Int(
            self.w[].call_i32("writer_finalize", args_ptr(self.writer))
        )
        return _read_mutations(self.w, self.buf, offset)

    fn reset_writer(mut self) raises:
        """Reset the writer for a new mutation sequence."""
        self.w[].call_void("writer_destroy", args_ptr(self.writer))
        # Zero out the buffer
        for i in range(BUF_CAP):
            _ = self.w[].call_i32(
                "debug_write_byte", args_ptr_i32_i32(self.buf, i, 0)
            )
        self.writer = Int(
            self.w[].call_i64("writer_create", args_ptr_i32(self.buf, BUF_CAP))
        )

    fn destroy(mut self) raises:
        self.w[].call_void("writer_destroy", args_ptr(self.writer))
        self.w[].call_void("mutation_buf_free", args_ptr(self.buf))
        self.w[].call_void("vnode_store_destroy", args_ptr(self.store))
        self.w[].call_void("eid_alloc_destroy", args_ptr(self.eid))
        self.w[].call_void("runtime_destroy", args_ptr(self.rt))


# ── Template registration helpers ────────────────────────────────────────────


fn _register_div_template(mut ctx: WasmTestContext, name: String) raises -> Int:
    """Register a simple <div></div> template, return ID."""
    var b = Int(
        ctx.w[].call_i64(
            "tmpl_builder_create",
            args_ptr(ctx.w[].write_string_struct(name)),
        )
    )
    _ = ctx.w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
    )
    var tmpl_id = Int(
        ctx.w[].call_i32("tmpl_builder_register", args_ptr_ptr(ctx.rt, b))
    )
    ctx.w[].call_void("tmpl_builder_destroy", args_ptr(b))
    return tmpl_id


fn _register_div_with_dyn_text(
    mut ctx: WasmTestContext, name: String
) raises -> Int:
    """Register <div>{dyntext_0}</div>, return ID."""
    var b = Int(
        ctx.w[].call_i64(
            "tmpl_builder_create",
            args_ptr(ctx.w[].write_string_struct(name)),
        )
    )
    var div_idx = Int(
        ctx.w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    _ = ctx.w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 0, div_idx)
    )
    var tmpl_id = Int(
        ctx.w[].call_i32("tmpl_builder_register", args_ptr_ptr(ctx.rt, b))
    )
    ctx.w[].call_void("tmpl_builder_destroy", args_ptr(b))
    return tmpl_id


fn _register_div_with_dyn_attr(
    mut ctx: WasmTestContext, name: String
) raises -> Int:
    """Register <div {dynattr_0}></div>, return ID."""
    var b = Int(
        ctx.w[].call_i64(
            "tmpl_builder_create",
            args_ptr(ctx.w[].write_string_struct(name)),
        )
    )
    var div_idx = Int(
        ctx.w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    ctx.w[].call_void(
        "tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, div_idx, 0)
    )
    var tmpl_id = Int(
        ctx.w[].call_i32("tmpl_builder_register", args_ptr_ptr(ctx.rt, b))
    )
    ctx.w[].call_void("tmpl_builder_destroy", args_ptr(b))
    return tmpl_id


fn _register_div_with_dyn_node(
    mut ctx: WasmTestContext, name: String
) raises -> Int:
    """Register <div>{dyn_node_0}</div>, return ID."""
    var b = Int(
        ctx.w[].call_i64(
            "tmpl_builder_create",
            args_ptr(ctx.w[].write_string_struct(name)),
        )
    )
    var div_idx = Int(
        ctx.w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    _ = ctx.w[].call_i32(
        "tmpl_builder_push_dynamic", args_ptr_i32_i32(b, 0, div_idx)
    )
    var tmpl_id = Int(
        ctx.w[].call_i32("tmpl_builder_register", args_ptr_ptr(ctx.rt, b))
    )
    ctx.w[].call_void("tmpl_builder_destroy", args_ptr(b))
    return tmpl_id


fn _register_complex_template(
    mut ctx: WasmTestContext, name: String
) raises -> Int:
    """Register a complex template:
    <div class="container">
      <h1>"Title"</h1>
      <p>{dyntext_0}</p>
      <button {dynattr_0}>{dyntext_1}</button>
    </div>
    """
    var w = ctx.w
    var b = Int(
        w[].call_i64(
            "tmpl_builder_create",
            args_ptr(w[].write_string_struct(name)),
        )
    )
    var div_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
        )
    )
    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(
            b,
            div_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("container"),
        ),
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
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 0, p_idx)
    )

    var btn_idx = Int(
        w[].call_i32(
            "tmpl_builder_push_element",
            args_ptr_i32_i32(b, TAG_BUTTON, div_idx),
        )
    )
    w[].call_void(
        "tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, btn_idx, 0)
    )
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 1, btn_idx)
    )

    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(ctx.rt, b))
    )
    w[].call_void("tmpl_builder_destroy", args_ptr(b))
    return tmpl_id


# ══════════════════════════════════════════════════════════════════════════════
# CREATE ENGINE TESTS
# ══════════════════════════════════════════════════════════════════════════════


fn test_create_text_vnode(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var ctx = WasmTestContext(w)

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("hello world")),
        )
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "text vnode creates 1 root")

    var mutations = ctx.finalize_and_read()

    assert_equal(len(mutations), 1, "1 mutation emitted for text vnode")
    assert_equal(
        mutations[0].op,
        OP_CREATE_TEXT_NODE,
        "mutation is CreateTextNode",
    )
    assert_true(mutations[0].id > 0, "element id is non-zero")

    # Check mount state
    assert_equal(
        Int(w[].call_i32("vnode_is_mounted", args_ptr_i32(ctx.store, vn_idx))),
        1,
        "text vnode is mounted",
    )
    assert_equal(
        Int(
            w[].call_i32("vnode_root_id_count", args_ptr_i32(ctx.store, vn_idx))
        ),
        1,
        "text vnode has 1 root id",
    )
    assert_true(
        Int(
            w[].call_i32(
                "vnode_get_root_id", args_ptr_i32_i32(ctx.store, vn_idx, 0)
            )
        )
        > 0,
        "root id is non-zero",
    )

    ctx.destroy()


fn test_create_placeholder_vnode(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var ctx = WasmTestContext(w)

    var vn_idx = Int(
        w[].call_i32("vnode_push_placeholder", args_ptr_i32(ctx.store, 0))
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "placeholder creates 1 root")

    var mutations = ctx.finalize_and_read()

    assert_equal(len(mutations), 1, "1 mutation for placeholder")
    assert_equal(
        mutations[0].op,
        OP_CREATE_PLACEHOLDER,
        "mutation is CreatePlaceholder",
    )

    assert_equal(
        Int(w[].call_i32("vnode_is_mounted", args_ptr_i32(ctx.store, vn_idx))),
        1,
        "placeholder is mounted",
    )
    assert_equal(
        Int(
            w[].call_i32("vnode_root_id_count", args_ptr_i32(ctx.store, vn_idx))
        ),
        1,
        "placeholder has 1 root id",
    )

    ctx.destroy()


fn test_create_simple_template_ref(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_template(ctx, "simple-div-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "template ref creates 1 root (div)")

    var mutations = ctx.finalize_and_read()

    # Should have at least a LoadTemplate mutation
    var load_count = _count_op(mutations, OP_LOAD_TEMPLATE)
    assert_equal(load_count, 1, "1 LoadTemplate mutation")

    var load_idx = _find_first(mutations, OP_LOAD_TEMPLATE)
    assert_true(load_idx >= 0, "LoadTemplate found")
    assert_equal(
        mutations[load_idx].id,
        tmpl_id,
        "LoadTemplate uses correct template ID",
    )

    # Check mount state
    assert_equal(
        Int(w[].call_i32("vnode_is_mounted", args_ptr_i32(ctx.store, vn_idx))),
        1,
        "template ref is mounted",
    )
    assert_equal(
        Int(
            w[].call_i32("vnode_root_id_count", args_ptr_i32(ctx.store, vn_idx))
        ),
        1,
        "template ref has 1 root id",
    )

    ctx.destroy()


fn test_create_template_ref_with_dyn_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_text(ctx, "dyn-text-div-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, vn_idx, w[].write_string_struct("Count: 42")
        ),
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "template with dyntext creates 1 root")

    var mutations = ctx.finalize_and_read()

    # Should have LoadTemplate
    var load_count = _count_op(mutations, OP_LOAD_TEMPLATE)
    assert_equal(load_count, 1, "1 LoadTemplate")

    # Should have AssignId and/or SetText for the dynamic text
    var assign_count = _count_op(mutations, OP_ASSIGN_ID)
    var set_text_count = _count_op(mutations, OP_SET_TEXT)
    assert_true(
        assign_count > 0 or set_text_count > 0,
        "has AssignId or SetText for dynamic text",
    )

    # Check mount state: should have dynamic node IDs
    assert_true(
        Int(
            w[].call_i32(
                "vnode_dyn_node_id_count", args_ptr_i32(ctx.store, vn_idx)
            )
        )
        > 0,
        "has dynamic node IDs",
    )

    ctx.destroy()


fn test_create_template_ref_with_dyn_attr(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "dyn-attr-div-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            vn_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("active"),
            0,
        ),
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "template with dynattr creates 1 root")

    var mutations = ctx.finalize_and_read()

    var load_count = _count_op(mutations, OP_LOAD_TEMPLATE)
    assert_equal(load_count, 1, "1 LoadTemplate")

    # Should have a SetAttribute mutation
    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_true(set_attr_count > 0, "has SetAttribute for dynamic attr")

    # Check mount state: should have dynamic attr IDs
    assert_true(
        Int(
            w[].call_i32(
                "vnode_dyn_attr_id_count", args_ptr_i32(ctx.store, vn_idx)
            )
        )
        > 0,
        "has dynamic attr IDs",
    )

    ctx.destroy()


fn test_create_template_ref_with_event(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "event-div-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, vn_idx, w[].write_string_struct("onclick"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have a NewEventListener mutation
    var has_listener = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_NEW_EVENT_LISTENER:
            has_listener = True
    assert_true(has_listener, "has NewEventListener for event attr")

    ctx.destroy()


fn test_create_template_ref_with_dyn_text_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Create a template ref with a Dynamic (full) node slot filled with text.
    """
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_node(ctx, "dyn-node-div-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, vn_idx, w[].write_string_struct("dynamic text")
        ),
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "creates 1 root")

    var mutations = ctx.finalize_and_read()

    # Should have CreateTextNode for the dynamic node
    var has_create_text = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_CREATE_TEXT_NODE:
            has_create_text = True
    assert_true(has_create_text, "has CreateTextNode for dynamic node")

    # Should have ReplacePlaceholder
    var has_replace = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_REPLACE_PLACEHOLDER:
            has_replace = True
    assert_true(has_replace, "has ReplacePlaceholder for dynamic node")

    ctx.destroy()


fn test_create_template_ref_with_dyn_placeholder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Create a template ref with a Dynamic node slot filled with placeholder.
    """
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_node(ctx, "dyn-ph-div-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_placeholder", args_ptr_i32(ctx.store, vn_idx)
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have CreatePlaceholder for the dynamic node
    var has_create_ph = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_CREATE_PLACEHOLDER:
            has_create_ph = True
    assert_true(has_create_ph, "has CreatePlaceholder for dynamic placeholder")

    # Should have ReplacePlaceholder
    var has_replace = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_REPLACE_PLACEHOLDER:
            has_replace = True
    assert_true(has_replace, "has ReplacePlaceholder")

    ctx.destroy()


fn test_create_fragment_vnode(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var ctx = WasmTestContext(w)

    # Create 3 text children
    var c1 = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var c2 = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("B")),
        )
    )
    var c3 = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("C")),
        )
    )

    # Create fragment
    var frag_idx = Int(w[].call_i32("vnode_push_fragment", args_ptr(ctx.store)))
    w[].call_void(
        "vnode_push_fragment_child", args_ptr_i32_i32(ctx.store, frag_idx, c1)
    )
    w[].call_void(
        "vnode_push_fragment_child", args_ptr_i32_i32(ctx.store, frag_idx, c2)
    )
    w[].call_void(
        "vnode_push_fragment_child", args_ptr_i32_i32(ctx.store, frag_idx, c3)
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, frag_idx
            ),
        )
    )
    assert_equal(num_roots, 3, "fragment creates 3 roots (one per child)")

    var mutations = ctx.finalize_and_read()

    # Should have 3 CreateTextNode mutations
    var create_text_count = _count_op(mutations, OP_CREATE_TEXT_NODE)
    assert_equal(create_text_count, 3, "3 CreateTextNode mutations")

    ctx.destroy()


fn test_create_element_id_uniqueness(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Multiple create calls produce unique ElementIds."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_template(ctx, "unique-div-mut")

    var vn1 = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    var vn2 = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    var vn3 = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(ctx.writer, ctx.eid, ctx.rt, ctx.store, vn1),
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(ctx.writer, ctx.eid, ctx.rt, ctx.store, vn2),
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(ctx.writer, ctx.eid, ctx.rt, ctx.store, vn3),
    )

    var id1 = Int(
        w[].call_i32("vnode_get_root_id", args_ptr_i32_i32(ctx.store, vn1, 0))
    )
    var id2 = Int(
        w[].call_i32("vnode_get_root_id", args_ptr_i32_i32(ctx.store, vn2, 0))
    )
    var id3 = Int(
        w[].call_i32("vnode_get_root_id", args_ptr_i32_i32(ctx.store, vn3, 0))
    )

    assert_true(id1 != id2, "id1 != id2")
    assert_true(id2 != id3, "id2 != id3")
    assert_true(id1 != id3, "id1 != id3")

    ctx.destroy()


fn test_create_empty_fragment(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Empty fragment creates 0 roots."""
    var ctx = WasmTestContext(w)

    var frag_idx = Int(w[].call_i32("vnode_push_fragment", args_ptr(ctx.store)))

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, frag_idx
            ),
        )
    )
    assert_equal(num_roots, 0, "empty fragment creates 0 roots")

    var mutations = ctx.finalize_and_read()
    assert_equal(len(mutations), 0, "empty fragment produces 0 mutations")

    ctx.destroy()


fn test_create_complex_template_multi_slots(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Create a complex template with multiple dynamic slots."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_complex_template(ctx, "complex-mut")

    var vn_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    # dyntext_0 -> "Description"
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, vn_idx, w[].write_string_struct("Description")
        ),
    )
    # dyntext_1 -> "Click me"
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, vn_idx, w[].write_string_struct("Click me")
        ),
    )
    # dynattr_0 -> onclick event
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, vn_idx, w[].write_string_struct("onclick"), 42, 0
        ),
    )

    var num_roots = Int(
        w[].call_i32(
            "create_vnode",
            args_ptr_ptr_ptr_ptr_i32(
                ctx.writer, ctx.eid, ctx.rt, ctx.store, vn_idx
            ),
        )
    )
    assert_equal(num_roots, 1, "complex template creates 1 root")

    var mutations = ctx.finalize_and_read()

    # Should have LoadTemplate
    var load_count = _count_op(mutations, OP_LOAD_TEMPLATE)
    assert_equal(load_count, 1, "1 LoadTemplate")

    # Should have AssignId mutations for dynamic slots
    var assign_count = _count_op(mutations, OP_ASSIGN_ID)
    assert_true(assign_count > 0, "has AssignId for dynamic slots")

    # Should have SetText mutations for dynamic text
    var set_text_count = _count_op(mutations, OP_SET_TEXT)
    assert_true(set_text_count > 0, "has SetText for dynamic text")

    # Should have NewEventListener for the onclick
    var listener_count = _count_op(mutations, OP_NEW_EVENT_LISTENER)
    assert_true(listener_count > 0, "has NewEventListener for onclick")

    ctx.destroy()


# ══════════════════════════════════════════════════════════════════════════════
# DIFF ENGINE TESTS
# ══════════════════════════════════════════════════════════════════════════════


fn test_diff_same_text_zero_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Diffing two Text VNodes with the same text -> 0 mutations."""
    var ctx = WasmTestContext(w)

    # Create old text vnode and mount it
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("hello")),
        )
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )

    # Reset writer for diff
    ctx.reset_writer()

    # Create new text vnode with same text
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("hello")),
        )
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()
    assert_equal(len(mutations), 0, "same text produces 0 mutations")

    ctx.destroy()


fn test_diff_text_changed_produces_set_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Diffing two Text VNodes with different text -> SetText."""
    var ctx = WasmTestContext(w)

    # Create old text vnode and mount it
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("hello")),
        )
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )

    # Remember the old root id
    var old_root_id = Int(
        w[].call_i32(
            "vnode_get_root_id", args_ptr_i32_i32(ctx.store, old_idx, 0)
        )
    )

    # Reset writer for diff
    ctx.reset_writer()

    # Create new text vnode with different text
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("world")),
        )
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()
    assert_true(len(mutations) > 0, "text change produces mutations")

    # Should have a SetText mutation targeting the old root id
    var has_set_text = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_SET_TEXT:
            has_set_text = True
            assert_equal(
                mutations[i].id,
                old_root_id,
                "SetText targets old root id",
            )
    assert_true(has_set_text, "SetText emitted")

    ctx.destroy()


fn test_diff_text_empty_to_content(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Diffing '' -> 'hello' produces SetText."""
    var ctx = WasmTestContext(w)

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("")),
        )
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("hello")),
        )
    )
    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var set_text_count = _count_op(mutations, OP_SET_TEXT)
    assert_equal(set_text_count, 1, "1 SetText for '' -> 'hello'")

    ctx.destroy()


fn test_diff_placeholder_to_placeholder_zero_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Diffing two Placeholders -> 0 mutations."""
    var ctx = WasmTestContext(w)

    var old_idx = Int(
        w[].call_i32("vnode_push_placeholder", args_ptr_i32(ctx.store, 0))
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32("vnode_push_placeholder", args_ptr_i32(ctx.store, 0))
    )
    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()
    assert_equal(
        len(mutations), 0, "placeholder -> placeholder produces 0 mutations"
    )

    ctx.destroy()


fn test_diff_same_template_same_dyn_values_zero_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Same template, same dynamic text -> 0 mutations."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_text(ctx, "same-dyn-mut")

    # Old VNode
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, old_idx, w[].write_string_struct("Count: 5")
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    # New VNode with same dynamic text
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, new_idx, w[].write_string_struct("Count: 5")
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()
    assert_equal(
        len(mutations),
        0,
        "same template + same dyntext produces 0 mutations",
    )

    ctx.destroy()


fn test_diff_same_template_dyn_text_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Same template, dynamic text changed -> SetText."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_text(ctx, "changed-dyn-mut")

    # Old VNode
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, old_idx, w[].write_string_struct("Count: 5")
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )

    # Get the dynamic node ID assigned during create
    var old_dyn_id = Int(
        w[].call_i32(
            "vnode_get_dyn_node_id", args_ptr_i32_i32(ctx.store, old_idx, 0)
        )
    )

    ctx.reset_writer()

    # New VNode with different dynamic text
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, new_idx, w[].write_string_struct("Count: 10")
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have a SetText targeting the dynamic node's element ID
    var has_set_text = False
    for i in range(len(mutations)):
        if mutations[i].op == OP_SET_TEXT:
            has_set_text = True
            assert_equal(
                mutations[i].id,
                old_dyn_id,
                "SetText targets dynamic node element",
            )
    assert_true(has_set_text, "SetText emitted for changed dynamic text")

    ctx.destroy()


fn test_diff_same_template_attr_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Same template, dynamic attribute changed -> SetAttribute."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "attr-changed-mut")

    # Old VNode
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            old_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("old-class"),
            0,
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    # New VNode with different attr value
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            new_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("new-class"),
            0,
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_true(set_attr_count > 0, "SetAttribute emitted for changed attr")

    ctx.destroy()


fn test_diff_same_template_attr_unchanged_zero_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Same template, same attribute value -> 0 mutations."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "attr-same-mut")

    # Old VNode
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            old_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("same"),
            0,
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    # New VNode with same attr value
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            new_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("same"),
            0,
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()
    assert_equal(len(mutations), 0, "same attr value produces 0 mutations")

    ctx.destroy()


fn test_diff_bool_attr_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Bool attribute changed -> SetAttribute."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "bool-attr-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_bool",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, old_idx, w[].write_string_struct("disabled"), 0, 0
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_bool",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, new_idx, w[].write_string_struct("disabled"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_true(set_attr_count > 0, "SetAttribute emitted for bool change")

    ctx.destroy()


fn test_diff_text_to_placeholder_replacement(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Text -> Placeholder (different kind) -> replacement."""
    var ctx = WasmTestContext(w)

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("hello")),
        )
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32("vnode_push_placeholder", args_ptr_i32(ctx.store, 0))
    )
    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have a create and a replace
    var has_create = (
        _count_op(mutations, OP_CREATE_PLACEHOLDER) > 0
        or _count_op(mutations, OP_CREATE_TEXT_NODE) > 0
    )
    var has_replace = _count_op(mutations, OP_REPLACE_WITH) > 0
    assert_true(has_create, "has create for replacement")
    assert_true(has_replace, "has ReplaceWith for kind change")

    ctx.destroy()


fn test_diff_different_templates_replacement(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Different templates -> full replacement."""
    var ctx = WasmTestContext(w)

    var tmpl_a = _register_div_template(ctx, "tmpl-a-mut")
    var tmpl_b = _register_div_template(ctx, "tmpl-b-mut")

    var old_idx = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_a))
    )
    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32("vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_b))
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have LoadTemplate for the new template + ReplaceWith
    var has_load = _count_op(mutations, OP_LOAD_TEMPLATE) > 0
    var has_replace = _count_op(mutations, OP_REPLACE_WITH) > 0
    assert_true(has_load, "LoadTemplate for new template in replacement")
    assert_true(has_replace, "ReplaceWith for different templates")

    ctx.destroy()


fn test_diff_fragment_children_text_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Fragment diff: child text changed."""
    var ctx = WasmTestContext(w)

    # Old fragment: [A, B]
    var oa = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var ob = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("B")),
        )
    )
    var old_frag_idx = Int(
        w[].call_i32("vnode_push_fragment", args_ptr(ctx.store))
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, old_frag_idx, oa),
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, old_frag_idx, ob),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_frag_idx
        ),
    )
    ctx.reset_writer()

    # New fragment: [A, C] (B -> C)
    var na = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var nc = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("C")),
        )
    )
    var new_frag_idx = Int(
        w[].call_i32("vnode_push_fragment", args_ptr(ctx.store))
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, new_frag_idx, na),
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, new_frag_idx, nc),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_frag_idx, new_frag_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # "A" same -> no mutation, "B" -> "C" -> SetText
    var set_text_count = _count_op(mutations, OP_SET_TEXT)
    assert_equal(set_text_count, 1, "1 SetText for B -> C")

    ctx.destroy()


fn test_diff_fragment_children_removed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Fragment diff: children removed."""
    var ctx = WasmTestContext(w)

    # Old fragment: [A, B, C]
    var oa = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var ob = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("B")),
        )
    )
    var oc = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("C")),
        )
    )
    var old_frag_idx = Int(
        w[].call_i32("vnode_push_fragment", args_ptr(ctx.store))
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, old_frag_idx, oa),
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, old_frag_idx, ob),
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, old_frag_idx, oc),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_frag_idx
        ),
    )
    ctx.reset_writer()

    # New fragment: [A] (B, C removed)
    var na = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var new_frag_idx = Int(
        w[].call_i32("vnode_push_fragment", args_ptr(ctx.store))
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, new_frag_idx, na),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_frag_idx, new_frag_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have Remove mutations for the extra children
    var remove_count = _count_op(mutations, OP_REMOVE)
    assert_true(remove_count >= 2, "at least 2 Remove mutations for B and C")

    ctx.destroy()


fn test_diff_fragment_children_added(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Fragment diff: children added."""
    var ctx = WasmTestContext(w)

    # Old fragment: [A]
    var oa = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var old_frag_idx = Int(
        w[].call_i32("vnode_push_fragment", args_ptr(ctx.store))
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, old_frag_idx, oa),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_frag_idx
        ),
    )
    ctx.reset_writer()

    # New fragment: [A, B, C]
    var na = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("A")),
        )
    )
    var nb = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("B")),
        )
    )
    var nc = Int(
        w[].call_i32(
            "vnode_push_text",
            args_ptr_ptr(ctx.store, w[].write_string_struct("C")),
        )
    )
    var new_frag_idx = Int(
        w[].call_i32("vnode_push_fragment", args_ptr(ctx.store))
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, new_frag_idx, na),
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, new_frag_idx, nb),
    )
    w[].call_void(
        "vnode_push_fragment_child",
        args_ptr_i32_i32(ctx.store, new_frag_idx, nc),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_frag_idx, new_frag_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have CreateTextNode for B and C, and InsertAfter
    var create_count = _count_op(mutations, OP_CREATE_TEXT_NODE)
    assert_true(create_count >= 2, "at least 2 CreateTextNode for B and C")

    var has_insert = _count_op(mutations, OP_INSERT_AFTER) > 0
    assert_true(has_insert, "has InsertAfter for added children")

    ctx.destroy()


fn test_diff_event_listener_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Event listener handler changed -> RemoveEventListener + NewEventListener.
    """
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "event-change-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, old_idx, w[].write_string_struct("onclick"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, new_idx, w[].write_string_struct("onclick"), 2, 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var has_remove = _count_op(mutations, OP_REMOVE_EVENT_LISTENER) > 0
    var has_new = _count_op(mutations, OP_NEW_EVENT_LISTENER) > 0
    assert_true(has_remove, "RemoveEventListener for old handler")
    assert_true(has_new, "NewEventListener for new handler")

    ctx.destroy()


fn test_diff_same_event_listener_zero_mutations(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Same event listener -> 0 mutations."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "event-same-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, old_idx, w[].write_string_struct("onclick"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_event",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, new_idx, w[].write_string_struct("onclick"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()
    assert_equal(len(mutations), 0, "same event listener produces 0 mutations")

    ctx.destroy()


fn test_diff_attr_type_changed_text_to_bool(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Attribute type changed (text -> bool) -> SetAttribute."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "type-change-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            old_idx,
            w[].write_string_struct("disabled"),
            w[].write_string_struct("yes"),
            0,
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_bool",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, new_idx, w[].write_string_struct("disabled"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_true(set_attr_count > 0, "SetAttribute for type change")

    ctx.destroy()


fn test_diff_attr_removed_text_to_none(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Attribute removed (text -> none) -> RemoveAttribute."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "attr-remove-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            old_idx,
            w[].write_string_struct("class"),
            w[].write_string_struct("active"),
            0,
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_none",
        args_ptr_i32_ptr_i32(
            ctx.store, new_idx, w[].write_string_struct("class"), 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var remove_attr_count = _count_op(mutations, OP_REMOVE_ATTRIBUTE)
    assert_true(remove_attr_count > 0, "RemoveAttribute for attr removal")

    ctx.destroy()


fn test_diff_attr_none_to_text(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Attribute appearing (none -> text) -> SetAttribute."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "attr-appear-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_none",
        args_ptr_i32_ptr_i32(
            ctx.store, old_idx, w[].write_string_struct("disabled"), 0
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            new_idx,
            w[].write_string_struct("disabled"),
            w[].write_string_struct(""),
            0,
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_true(set_attr_count > 0, "SetAttribute for attr appearance")

    # Verify no RemoveAttribute was emitted
    var remove_attr_count = _count_op(mutations, OP_REMOVE_ATTRIBUTE)
    assert_equal(remove_attr_count, 0, "no RemoveAttribute for appearance")

    ctx.destroy()


fn test_diff_bool_attr_true_to_false_remove(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Boolean HTML attr pattern: AVAL_TEXT('') -> AVAL_NONE -> RemoveAttribute.
    """
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "bool-rm-mut")

    # Old: attribute present (AVAL_TEXT(""))
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_text",
        args_ptr_i32_ptr_ptr_i32(
            ctx.store,
            old_idx,
            w[].write_string_struct("hidden"),
            w[].write_string_struct(""),
            0,
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    # New: attribute absent (AVAL_NONE)
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_none",
        args_ptr_i32_ptr_i32(
            ctx.store, new_idx, w[].write_string_struct("hidden"), 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var remove_attr_count = _count_op(mutations, OP_REMOVE_ATTRIBUTE)
    assert_true(
        remove_attr_count > 0,
        "RemoveAttribute for bool attr true->false",
    )

    # No SetAttribute should be emitted
    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_equal(set_attr_count, 0, "no SetAttribute for bool removal")

    ctx.destroy()


fn test_diff_int_attr_changed(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Integer attribute value changed -> SetAttribute."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_attr(ctx, "int-attr-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_int",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, old_idx, w[].write_string_struct("tabindex"), 1, 0
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_attr_int",
        args_ptr_i32_ptr_i32_i32(
            ctx.store, new_idx, w[].write_string_struct("tabindex"), 5, 0
        ),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    var set_attr_count = _count_op(mutations, OP_SET_ATTRIBUTE)
    assert_true(set_attr_count > 0, "SetAttribute for int change")

    ctx.destroy()


fn test_diff_mount_state_transfer_preserves_ids(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Diff transfers mount state: ElementIds on new VNode match old."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_text(ctx, "transfer-test-mut")

    # Old VNode
    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(ctx.store, old_idx, w[].write_string_struct("old")),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )

    var old_root_id = Int(
        w[].call_i32(
            "vnode_get_root_id", args_ptr_i32_i32(ctx.store, old_idx, 0)
        )
    )
    var old_dyn_id = Int(
        w[].call_i32(
            "vnode_get_dyn_node_id", args_ptr_i32_i32(ctx.store, old_idx, 0)
        )
    )

    ctx.reset_writer()

    # New VNode
    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(ctx.store, new_idx, w[].write_string_struct("new")),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    # Check that new VNode got the same ElementIds
    var new_root_id = Int(
        w[].call_i32(
            "vnode_get_root_id", args_ptr_i32_i32(ctx.store, new_idx, 0)
        )
    )
    var new_dyn_id = Int(
        w[].call_i32(
            "vnode_get_dyn_node_id", args_ptr_i32_i32(ctx.store, new_idx, 0)
        )
    )

    assert_equal(new_root_id, old_root_id, "root ID transferred to new VNode")
    assert_equal(
        new_dyn_id, old_dyn_id, "dynamic node ID transferred to new VNode"
    )

    ctx.destroy()


fn test_diff_sequential_diffs_state_chain(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Sequential diffs (state chain): v0 -> v1 -> v2 -> v3."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_text(ctx, "chain-test-mut")

    # v0: initial
    var v0_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(ctx.store, v0_idx, w[].write_string_struct("state-0")),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, v0_idx
        ),
    )
    ctx.reset_writer()

    # v0 -> v1
    var v1_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(ctx.store, v1_idx, w[].write_string_struct("state-1")),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, v0_idx, v1_idx
        ),
    )

    var muts1 = ctx.finalize_and_read()
    var st1 = _count_op(muts1, OP_SET_TEXT)
    assert_equal(st1, 1, "v0 -> v1: 1 SetText")

    ctx.reset_writer()

    # v1 -> v2
    var v2_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(ctx.store, v2_idx, w[].write_string_struct("state-2")),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, v1_idx, v2_idx
        ),
    )

    var muts2 = ctx.finalize_and_read()
    var st2 = _count_op(muts2, OP_SET_TEXT)
    assert_equal(st2, 1, "v1 -> v2: 1 SetText")

    ctx.reset_writer()

    # v2 -> v3 (same text -> 0)
    var v3_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(ctx.store, v3_idx, w[].write_string_struct("state-2")),
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, v2_idx, v3_idx
        ),
    )

    var muts3 = ctx.finalize_and_read()
    assert_equal(len(muts3), 0, "v2 -> v3 same text: 0 mutations")

    ctx.destroy()


fn test_diff_dyn_node_text_to_placeholder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Dynamic node: text -> placeholder -> replacement."""
    var ctx = WasmTestContext(w)

    var tmpl_id = _register_div_with_dyn_node(ctx, "dyn-text-to-ph-mut")

    var old_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_text_node",
        args_ptr_i32_ptr(
            ctx.store, old_idx, w[].write_string_struct("some text")
        ),
    )

    _ = w[].call_i32(
        "create_vnode",
        args_ptr_ptr_ptr_ptr_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx
        ),
    )
    ctx.reset_writer()

    var new_idx = Int(
        w[].call_i32(
            "vnode_push_template_ref", args_ptr_i32(ctx.store, tmpl_id)
        )
    )
    w[].call_void(
        "vnode_push_dynamic_placeholder", args_ptr_i32(ctx.store, new_idx)
    )

    _ = w[].call_i32(
        "diff_vnodes",
        args_ptr_ptr_ptr_ptr_i32_i32(
            ctx.writer, ctx.eid, ctx.rt, ctx.store, old_idx, new_idx
        ),
    )

    var mutations = ctx.finalize_and_read()

    # Should have CreatePlaceholder and ReplaceWith
    var has_create_ph = _count_op(mutations, OP_CREATE_PLACEHOLDER) > 0
    var has_replace = _count_op(mutations, OP_REPLACE_WITH) > 0
    assert_true(has_create_ph, "CreatePlaceholder for text -> placeholder")
    assert_true(has_replace, "ReplaceWith for text -> placeholder")

    ctx.destroy()


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_create_text_vnode(w)
    test_create_placeholder_vnode(w)
    test_create_simple_template_ref(w)
    test_create_template_ref_with_dyn_text(w)
    test_create_template_ref_with_dyn_attr(w)
    test_create_template_ref_with_event(w)
    test_create_template_ref_with_dyn_text_node(w)
    test_create_template_ref_with_dyn_placeholder(w)
    test_create_fragment_vnode(w)
    test_create_element_id_uniqueness(w)
    test_create_empty_fragment(w)
    test_create_complex_template_multi_slots(w)
    test_diff_same_text_zero_mutations(w)
    test_diff_text_changed_produces_set_text(w)
    test_diff_text_empty_to_content(w)
    test_diff_placeholder_to_placeholder_zero_mutations(w)
    test_diff_same_template_same_dyn_values_zero_mutations(w)
    test_diff_same_template_dyn_text_changed(w)
    test_diff_same_template_attr_changed(w)
    test_diff_same_template_attr_unchanged_zero_mutations(w)
    test_diff_bool_attr_changed(w)
    test_diff_text_to_placeholder_replacement(w)
    test_diff_different_templates_replacement(w)
    test_diff_fragment_children_text_changed(w)
    test_diff_fragment_children_removed(w)
    test_diff_fragment_children_added(w)
    test_diff_event_listener_changed(w)
    test_diff_same_event_listener_zero_mutations(w)
    test_diff_attr_type_changed_text_to_bool(w)
    test_diff_attr_removed_text_to_none(w)
    test_diff_attr_none_to_text(w)
    test_diff_bool_attr_true_to_false_remove(w)
    test_diff_int_attr_changed(w)
    test_diff_mount_state_transfer_preserves_ids(w)
    test_diff_sequential_diffs_state_chain(w)
    test_diff_dyn_node_text_to_placeholder(w)
    print("mutations: 36/36 passed")
