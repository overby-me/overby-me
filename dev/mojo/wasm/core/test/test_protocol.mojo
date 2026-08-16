# MutationWriter binary encoding exercised through the real WASM binary
# via mojo-wasmtime (pure Mojo FFI bindings — no Python interop required).
#
# These tests verify that the binary encoding of DOM mutations works correctly
# when compiled to WASM and executed via the Wasmtime runtime.  Each test
# allocates a buffer in WASM memory, writes mutations via WASM exports, then
# reads back raw bytes via debug_read_byte to verify the encoding matches
# the expected binary layout.
#
# Run with:
#   mojo test test/test_protocol.mojo

from memory import UnsafePointer
from testing import assert_equal, assert_true

from wasm_harness import (
    WasmInstance,
    get_instance,
    args_i32,
    args_ptr,
    args_ptr_i32,
    args_ptr_i32_i32,
    args_ptr_i32_i32_i32,
    args_ptr_i32_i32_ptr,
    args_ptr_i32_i32_i32_ptr,
    args_ptr_i32_i32_i32_i32,
    args_ptr_i32_i32_i32_ptr_ptr,
    args_ptr_i32_ptr_i32,
    args_ptr_i32_ptr_i32_i32,
    args_ptr_i32_ptr_ptr,
    args_ptr_ptr,
    args_ptr_ptr_i32,
    no_args,
)


fn _get_wasm() raises -> UnsafePointer[WasmInstance, MutExternalOrigin]:
    return get_instance()


# ── Constants (matching bridge/protocol) ─────────────────────────────────────

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
comptime OP_REGISTER_TEMPLATE = 0x10
comptime OP_REMOVE_ATTRIBUTE = 0x11

comptime BUF_CAP = 4096


# ── Helpers ──────────────────────────────────────────────────────────────────


fn _alloc_buf(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises -> Int:
    """Allocate a mutation buffer in WASM linear memory."""
    return Int(w[].call_i64("mutation_buf_alloc", args_i32(BUF_CAP)))


fn _free_buf(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int
) raises:
    """Free a mutation buffer."""
    w[].call_void("mutation_buf_free", args_ptr(buf))


fn _read_u8(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Int:
    """Read a single byte from WASM memory."""
    return Int(w[].call_i32("debug_read_byte", args_ptr_i32(buf, offset)))


fn _read_u16_le(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Int:
    """Read a little-endian u16 from WASM memory."""
    var lo = _read_u8(w, buf, offset)
    var hi = _read_u8(w, buf, offset + 1)
    return lo | (hi << 8)


fn _read_u32_le(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Int:
    """Read a little-endian u32 from WASM memory."""
    var b0 = _read_u8(w, buf, offset)
    var b1 = _read_u8(w, buf, offset + 1)
    var b2 = _read_u8(w, buf, offset + 2)
    var b3 = _read_u8(w, buf, offset + 3)
    return b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)


# ── End sentinel ─────────────────────────────────────────────────────────────


fn test_end_sentinel(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(w[].call_i32("write_op_end", args_ptr_i32(buf, 0)))

    assert_equal(_read_u8(w, buf, 0), OP_END, "end writes 0x00")
    assert_equal(off, 1, "offset advances by 1")

    _free_buf(w, buf)


fn test_empty_buffer_starts_at_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    # Writer starts at offset 0 (the write_op functions take off as input)
    # Just verify we can read zero
    assert_equal(_read_u8(w, buf, 0), 0, "fresh buffer byte is 0")

    _free_buf(w, buf)


fn test_writer_with_initial_offset(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var off = Int(w[].call_i32("write_op_end", args_ptr_i32(buf, 10)))
    assert_equal(off, 11, "after end at offset 10, offset is 11")
    assert_equal(_read_u8(w, buf, 10), OP_END, "end at offset 10")

    _free_buf(w, buf)


# ── AppendChildren ───────────────────────────────────────────────────────────


fn test_append_children(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_append_children", args_ptr_i32_i32_i32(buf, 0, 7, 3)
        )
    )
    var final_off = Int(w[].call_i32("write_op_end", args_ptr_i32(buf, off)))

    assert_equal(
        _read_u8(w, buf, 0), OP_APPEND_CHILDREN, "opcode is APPEND_CHILDREN"
    )
    assert_equal(_read_u32_le(w, buf, 1), 7, "id is 7")
    assert_equal(_read_u32_le(w, buf, 5), 3, "m is 3")
    assert_equal(_read_u8(w, buf, 9), OP_END, "terminated with END")
    assert_equal(final_off, 10, "offset is 1 + 4 + 4 + 1 = 10")

    _free_buf(w, buf)


fn test_append_children_zero(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    _ = w[].call_i32(
        "write_op_append_children", args_ptr_i32_i32_i32(buf, 0, 1, 0)
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, 9))

    assert_equal(_read_u8(w, buf, 0), OP_APPEND_CHILDREN)
    assert_equal(_read_u32_le(w, buf, 1), 1, "id is 1")
    assert_equal(_read_u32_le(w, buf, 5), 0, "m is 0 (zero children)")

    _free_buf(w, buf)


# ── CreatePlaceholder ────────────────────────────────────────────────────────


fn test_create_placeholder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_create_placeholder", args_ptr_i32_i32(buf, 0, 42)
        )
    )
    var final_off = Int(w[].call_i32("write_op_end", args_ptr_i32(buf, off)))

    assert_equal(
        _read_u8(w, buf, 0),
        OP_CREATE_PLACEHOLDER,
        "opcode is CREATE_PLACEHOLDER",
    )
    assert_equal(_read_u32_le(w, buf, 1), 42, "id is 42")
    assert_equal(_read_u8(w, buf, 5), OP_END)
    assert_equal(final_off, 6, "offset is 1 + 4 + 1 = 6")

    _free_buf(w, buf)


# ── CreateTextNode ───────────────────────────────────────────────────────────


fn test_create_text_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var text_ptr = w[].write_string_struct("hello")
    var off = Int(
        w[].call_i32(
            "write_op_create_text_node",
            args_ptr_i32_i32_ptr(buf, 0, 5, text_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(
        _read_u8(w, buf, 0), OP_CREATE_TEXT_NODE, "opcode is CREATE_TEXT_NODE"
    )
    assert_equal(_read_u32_le(w, buf, 1), 5, "id is 5")
    # u32 length prefix
    assert_equal(_read_u32_le(w, buf, 5), 5, "text length is 5")
    # text bytes
    assert_equal(_read_u8(w, buf, 9), Int(ord("h")))
    assert_equal(_read_u8(w, buf, 10), Int(ord("e")))
    assert_equal(_read_u8(w, buf, 11), Int(ord("l")))
    assert_equal(_read_u8(w, buf, 12), Int(ord("l")))
    assert_equal(_read_u8(w, buf, 13), Int(ord("o")))
    assert_equal(_read_u8(w, buf, 14), OP_END)

    _free_buf(w, buf)


fn test_create_text_node_empty_string(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var text_ptr = w[].write_string_struct("")
    var off = Int(
        w[].call_i32(
            "write_op_create_text_node",
            args_ptr_i32_i32_ptr(buf, 0, 1, text_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_CREATE_TEXT_NODE)
    assert_equal(_read_u32_le(w, buf, 1), 1, "id is 1")
    assert_equal(_read_u32_le(w, buf, 5), 0, "text length is 0")
    assert_equal(_read_u8(w, buf, 9), OP_END)

    _free_buf(w, buf)


fn test_create_text_node_unicode(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var text = String("héllo")
    var text_len = len(text)
    var text_ptr = w[].write_string_struct(text)
    var off = Int(
        w[].call_i32(
            "write_op_create_text_node",
            args_ptr_i32_i32_ptr(buf, 0, 2, text_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_CREATE_TEXT_NODE)
    assert_equal(_read_u32_le(w, buf, 1), 2, "id is 2")
    assert_equal(
        _read_u32_le(w, buf, 5), text_len, "text length matches UTF-8 bytes"
    )

    _free_buf(w, buf)


# ── LoadTemplate ─────────────────────────────────────────────────────────────


fn test_load_template(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_load_template",
            args_ptr_i32_i32_i32_i32(buf, 0, 10, 0, 100),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(
        _read_u8(w, buf, 0), OP_LOAD_TEMPLATE, "opcode is LOAD_TEMPLATE"
    )
    assert_equal(_read_u32_le(w, buf, 1), 10, "tmpl_id is 10")
    assert_equal(_read_u32_le(w, buf, 5), 0, "index is 0")
    assert_equal(_read_u32_le(w, buf, 9), 100, "id is 100")
    assert_equal(_read_u8(w, buf, 13), OP_END)
    assert_equal(off, 13, "offset is 1 + 4 + 4 + 4 = 13")

    _free_buf(w, buf)


# ── ReplaceWith ──────────────────────────────────────────────────────────────


fn test_replace_with(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_replace_with", args_ptr_i32_i32_i32(buf, 0, 5, 2)
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_REPLACE_WITH, "opcode is REPLACE_WITH")
    assert_equal(_read_u32_le(w, buf, 1), 5, "id is 5")
    assert_equal(_read_u32_le(w, buf, 5), 2, "m is 2")
    assert_equal(_read_u8(w, buf, 9), OP_END)

    _free_buf(w, buf)


# ── InsertAfter ──────────────────────────────────────────────────────────────


fn test_insert_after(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_insert_after", args_ptr_i32_i32_i32(buf, 0, 8, 1)
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_INSERT_AFTER, "opcode is INSERT_AFTER")
    assert_equal(_read_u32_le(w, buf, 1), 8, "id is 8")
    assert_equal(_read_u32_le(w, buf, 5), 1, "m is 1")
    assert_equal(_read_u8(w, buf, 9), OP_END)

    _free_buf(w, buf)


# ── InsertBefore ─────────────────────────────────────────────────────────────


fn test_insert_before(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_insert_before", args_ptr_i32_i32_i32(buf, 0, 9, 4)
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(
        _read_u8(w, buf, 0), OP_INSERT_BEFORE, "opcode is INSERT_BEFORE"
    )
    assert_equal(_read_u32_le(w, buf, 1), 9, "id is 9")
    assert_equal(_read_u32_le(w, buf, 5), 4, "m is 4")
    assert_equal(_read_u8(w, buf, 9), OP_END)

    _free_buf(w, buf)


# ── Remove ───────────────────────────────────────────────────────────────────


fn test_remove(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(w[].call_i32("write_op_remove", args_ptr_i32_i32(buf, 0, 15)))
    var final_off = Int(w[].call_i32("write_op_end", args_ptr_i32(buf, off)))

    assert_equal(_read_u8(w, buf, 0), OP_REMOVE, "opcode is REMOVE")
    assert_equal(_read_u32_le(w, buf, 1), 15, "id is 15")
    assert_equal(_read_u8(w, buf, 5), OP_END)
    assert_equal(final_off, 6, "offset is 1 + 4 + 1 = 6")

    _free_buf(w, buf)


# ── PushRoot ─────────────────────────────────────────────────────────────────


fn test_push_root(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32("write_op_push_root", args_ptr_i32_i32(buf, 0, 20))
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_PUSH_ROOT, "opcode is PUSH_ROOT")
    assert_equal(_read_u32_le(w, buf, 1), 20, "id is 20")
    assert_equal(_read_u8(w, buf, 5), OP_END)

    _free_buf(w, buf)


# ── SetText ──────────────────────────────────────────────────────────────────


fn test_set_text(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var text_ptr = w[].write_string_struct("world")
    var off = Int(
        w[].call_i32(
            "write_op_set_text", args_ptr_i32_i32_ptr(buf, 0, 3, text_ptr)
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_SET_TEXT, "opcode is SET_TEXT")
    assert_equal(_read_u32_le(w, buf, 1), 3, "id is 3")
    assert_equal(_read_u32_le(w, buf, 5), 5, "text length is 5")
    assert_equal(_read_u8(w, buf, 9), Int(ord("w")))
    assert_equal(_read_u8(w, buf, 10), Int(ord("o")))
    assert_equal(_read_u8(w, buf, 11), Int(ord("r")))
    assert_equal(_read_u8(w, buf, 12), Int(ord("l")))
    assert_equal(_read_u8(w, buf, 13), Int(ord("d")))
    assert_equal(_read_u8(w, buf, 14), OP_END)

    _free_buf(w, buf)


# ── SetAttribute ─────────────────────────────────────────────────────────────


fn test_set_attribute(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("class")
    var val_ptr = w[].write_string_struct("active")
    var off = Int(
        w[].call_i32(
            "write_op_set_attribute",
            args_ptr_i32_i32_i32_ptr_ptr(buf, 0, 7, 0, name_ptr, val_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0
    assert_equal(
        _read_u8(w, buf, pos), OP_SET_ATTRIBUTE, "opcode is SET_ATTRIBUTE"
    )
    pos += 1

    assert_equal(_read_u32_le(w, buf, pos), 7, "id is 7")
    pos += 4

    assert_equal(_read_u8(w, buf, pos), 0, "ns is 0 (no namespace)")
    pos += 1

    # name is u16-length-prefixed
    assert_equal(_read_u16_le(w, buf, pos), 5, "name length is 5")
    pos += 2
    assert_equal(_read_u8(w, buf, pos), Int(ord("c")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("l")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("a")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("s")))
    assert_equal(_read_u8(w, buf, pos + 4), Int(ord("s")))
    pos += 5

    # value is u32-length-prefixed
    assert_equal(_read_u32_le(w, buf, pos), 6, "value length is 6")
    pos += 4
    assert_equal(_read_u8(w, buf, pos), Int(ord("a")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("c")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("t")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("i")))
    assert_equal(_read_u8(w, buf, pos + 4), Int(ord("v")))
    assert_equal(_read_u8(w, buf, pos + 5), Int(ord("e")))
    pos += 6

    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


fn test_set_attribute_with_namespace(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("href")
    var val_ptr = w[].write_string_struct("url")
    _ = w[].call_i32(
        "write_op_set_attribute",
        args_ptr_i32_i32_i32_ptr_ptr(buf, 0, 1, 1, name_ptr, val_ptr),
    )

    assert_equal(_read_u8(w, buf, 0), OP_SET_ATTRIBUTE)
    assert_equal(_read_u32_le(w, buf, 1), 1, "id is 1")
    assert_equal(_read_u8(w, buf, 5), 1, "ns is 1 (xlink)")

    _free_buf(w, buf)


fn test_set_attribute_empty_value(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("disabled")
    var val_ptr = w[].write_string_struct("")
    _ = w[].call_i32(
        "write_op_set_attribute",
        args_ptr_i32_i32_i32_ptr_ptr(buf, 0, 1, 0, name_ptr, val_ptr),
    )

    var pos = 0
    assert_equal(_read_u8(w, buf, pos), OP_SET_ATTRIBUTE)
    pos += 1 + 4 + 1  # op + id + ns

    # name: "disabled" = 8 chars
    assert_equal(_read_u16_le(w, buf, pos), 8, "name length is 8")
    pos += 2 + 8

    # value: empty
    assert_equal(_read_u32_le(w, buf, pos), 0, "value length is 0")
    pos += 4

    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


# ── RemoveAttribute ──────────────────────────────────────────────────────────


fn test_remove_attribute(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("disabled")
    var off = Int(
        w[].call_i32(
            "write_op_remove_attribute",
            args_ptr_i32_i32_i32_ptr(buf, 0, 53, 0, name_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0
    assert_equal(
        _read_u8(w, buf, pos),
        OP_REMOVE_ATTRIBUTE,
        "opcode is REMOVE_ATTRIBUTE",
    )
    pos += 1

    assert_equal(_read_u32_le(w, buf, pos), 53, "id is 53")
    pos += 4

    assert_equal(_read_u8(w, buf, pos), 0, "ns is 0 (no namespace)")
    pos += 1

    # name is u16-length-prefixed
    assert_equal(_read_u16_le(w, buf, pos), 8, "name length is 8")
    pos += 2
    assert_equal(_read_u8(w, buf, pos), Int(ord("d")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("i")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("s")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("a")))
    assert_equal(_read_u8(w, buf, pos + 4), Int(ord("b")))
    assert_equal(_read_u8(w, buf, pos + 5), Int(ord("l")))
    assert_equal(_read_u8(w, buf, pos + 6), Int(ord("e")))
    assert_equal(_read_u8(w, buf, pos + 7), Int(ord("d")))
    pos += 8

    # No value payload (unlike SetAttribute)
    assert_equal(_read_u8(w, buf, pos), OP_END)

    # Verify byte count: 1 (op) + 4 (id) + 1 (ns) + 2 (name_len) + 8 (name) = 16
    assert_equal(off, 16, "RemoveAttribute('disabled') writes 16 bytes")

    _free_buf(w, buf)


fn test_remove_attribute_with_namespace(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("href")
    _ = w[].call_i32(
        "write_op_remove_attribute",
        args_ptr_i32_i32_i32_ptr(buf, 0, 54, 1, name_ptr),
    )

    assert_equal(_read_u8(w, buf, 0), OP_REMOVE_ATTRIBUTE)
    assert_equal(_read_u32_le(w, buf, 1), 54, "id is 54")
    assert_equal(_read_u8(w, buf, 5), 1, "ns is 1 (xlink)")

    _free_buf(w, buf)


# ── NewEventListener ─────────────────────────────────────────────────────────


fn test_new_event_listener(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("click")
    var off = Int(
        w[].call_i32(
            "write_op_new_event_listener",
            args_ptr_i32_i32_i32_ptr(buf, 0, 11, 42, name_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0
    assert_equal(
        _read_u8(w, buf, pos),
        OP_NEW_EVENT_LISTENER,
        "opcode is NEW_EVENT_LISTENER",
    )
    pos += 1

    assert_equal(_read_u32_le(w, buf, pos), 11, "id is 11")
    pos += 4

    assert_equal(_read_u32_le(w, buf, pos), 42, "handler_id is 42")
    pos += 4

    # name is u16-length-prefixed
    assert_equal(_read_u16_le(w, buf, pos), 5, "name length is 5")
    pos += 2
    assert_equal(_read_u8(w, buf, pos), Int(ord("c")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("l")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("i")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("c")))
    assert_equal(_read_u8(w, buf, pos + 4), Int(ord("k")))
    pos += 5

    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


# ── RemoveEventListener ─────────────────────────────────────────────────────


fn test_remove_event_listener(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var name_ptr = w[].write_string_struct("click")
    var off = Int(
        w[].call_i32(
            "write_op_remove_event_listener",
            args_ptr_i32_i32_ptr(buf, 0, 11, name_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0
    assert_equal(
        _read_u8(w, buf, pos),
        OP_REMOVE_EVENT_LISTENER,
        "opcode is REMOVE_EVENT_LISTENER",
    )
    pos += 1

    assert_equal(_read_u32_le(w, buf, pos), 11, "id is 11")
    pos += 4

    assert_equal(_read_u16_le(w, buf, pos), 5, "name length is 5")
    pos += 2 + 5

    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


# ── AssignId ─────────────────────────────────────────────────────────────────


fn test_assign_id(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    # Build a path in WASM memory: [0, 1, 2]
    var path_ptr = Int(w[].call_i64("mutation_buf_alloc", args_i32(3)))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 0, 0))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 1, 1))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 2, 2))

    var off = Int(
        w[].call_i32(
            "write_op_assign_id",
            args_ptr_i32_ptr_i32_i32(buf, 0, path_ptr, 3, 50),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0
    assert_equal(_read_u8(w, buf, pos), OP_ASSIGN_ID, "opcode is ASSIGN_ID")
    pos += 1

    # path_len (u8)
    assert_equal(_read_u8(w, buf, pos), 3, "path_len is 3")
    pos += 1

    # path bytes
    assert_equal(_read_u8(w, buf, pos), 0, "path[0] is 0")
    assert_equal(_read_u8(w, buf, pos + 1), 1, "path[1] is 1")
    assert_equal(_read_u8(w, buf, pos + 2), 2, "path[2] is 2")
    pos += 3

    # id
    assert_equal(_read_u32_le(w, buf, pos), 50, "id is 50")
    pos += 4

    assert_equal(_read_u8(w, buf, pos), OP_END)

    w[].call_void("mutation_buf_free", args_ptr(path_ptr))
    _free_buf(w, buf)


fn test_assign_id_empty_path(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var path_ptr = Int(w[].call_i64("mutation_buf_alloc", args_i32(1)))
    var off = Int(
        w[].call_i32(
            "write_op_assign_id",
            args_ptr_i32_ptr_i32_i32(buf, 0, path_ptr, 0, 1),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_ASSIGN_ID)
    assert_equal(_read_u8(w, buf, 1), 0, "path_len is 0")
    assert_equal(_read_u32_le(w, buf, 2), 1, "id is 1")
    assert_equal(_read_u8(w, buf, 6), OP_END)

    w[].call_void("mutation_buf_free", args_ptr(path_ptr))
    _free_buf(w, buf)


fn test_assign_id_single_element_path(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var path_ptr = Int(w[].call_i64("mutation_buf_alloc", args_i32(1)))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 0, 5))

    var off = Int(
        w[].call_i32(
            "write_op_assign_id",
            args_ptr_i32_ptr_i32_i32(buf, 0, path_ptr, 1, 99),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_ASSIGN_ID)
    assert_equal(_read_u8(w, buf, 1), 1, "path_len is 1")
    assert_equal(_read_u8(w, buf, 2), 5, "path[0] is 5")
    assert_equal(_read_u32_le(w, buf, 3), 99, "id is 99")
    assert_equal(_read_u8(w, buf, 7), OP_END)

    w[].call_void("mutation_buf_free", args_ptr(path_ptr))
    _free_buf(w, buf)


# ── ReplacePlaceholder ───────────────────────────────────────────────────────


fn test_replace_placeholder(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var path_ptr = Int(w[].call_i64("mutation_buf_alloc", args_i32(2)))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 0, 0))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 1, 3))

    var off = Int(
        w[].call_i32(
            "write_op_replace_placeholder",
            args_ptr_i32_ptr_i32_i32(buf, 0, path_ptr, 2, 1),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0
    assert_equal(
        _read_u8(w, buf, pos),
        OP_REPLACE_PLACEHOLDER,
        "opcode is REPLACE_PLACEHOLDER",
    )
    pos += 1

    assert_equal(_read_u8(w, buf, pos), 2, "path_len is 2")
    pos += 1

    assert_equal(_read_u8(w, buf, pos), 0, "path[0] is 0")
    assert_equal(_read_u8(w, buf, pos + 1), 3, "path[1] is 3")
    pos += 2

    assert_equal(_read_u32_le(w, buf, pos), 1, "m is 1")
    pos += 4

    assert_equal(_read_u8(w, buf, pos), OP_END)

    w[].call_void("mutation_buf_free", args_ptr(path_ptr))
    _free_buf(w, buf)


# ── Multiple mutations in sequence ───────────────────────────────────────────


fn test_multiple_mutations_in_sequence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32("write_op_create_placeholder", args_ptr_i32_i32(buf, 0, 1))
    )
    off = Int(w[].call_i32("write_op_push_root", args_ptr_i32_i32(buf, off, 2)))
    off = Int(
        w[].call_i32(
            "write_op_append_children",
            args_ptr_i32_i32_i32(buf, off, 0, 2),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0

    # First: create_placeholder
    assert_equal(_read_u8(w, buf, pos), OP_CREATE_PLACEHOLDER)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 1)
    pos += 4

    # Second: push_root
    assert_equal(_read_u8(w, buf, pos), OP_PUSH_ROOT)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 2)
    pos += 4

    # Third: append_children
    assert_equal(_read_u8(w, buf, pos), OP_APPEND_CHILDREN)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 0)
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 2)
    pos += 4

    # End
    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


# ── Mixed mutations with strings ─────────────────────────────────────────────


fn test_mixed_mutations_with_strings(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32(
            "write_op_load_template",
            args_ptr_i32_i32_i32_i32(buf, 0, 0, 0, 1),
        )
    )
    var hi_ptr = w[].write_string_struct("hi")
    off = Int(
        w[].call_i32(
            "write_op_create_text_node",
            args_ptr_i32_i32_ptr(buf, off, 2, hi_ptr),
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_append_children",
            args_ptr_i32_i32_i32(buf, off, 1, 1),
        )
    )
    var click_ptr = w[].write_string_struct("click")
    off = Int(
        w[].call_i32(
            "write_op_new_event_listener",
            args_ptr_i32_i32_i32_ptr(buf, off, 1, 99, click_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    var pos = 0

    # LoadTemplate
    assert_equal(_read_u8(w, buf, pos), OP_LOAD_TEMPLATE)
    pos += 1 + 4 + 4 + 4  # op + tmpl_id + index + id = 13

    # CreateTextNode
    assert_equal(_read_u8(w, buf, pos), OP_CREATE_TEXT_NODE)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 2, "text node id is 2")
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 2, "text length is 2")
    pos += 4
    assert_equal(_read_u8(w, buf, pos), Int(ord("h")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("i")))
    pos += 2

    # AppendChildren
    assert_equal(_read_u8(w, buf, pos), OP_APPEND_CHILDREN)
    pos += 1 + 4 + 4

    # NewEventListener
    assert_equal(_read_u8(w, buf, pos), OP_NEW_EVENT_LISTENER)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 1, "event listener element id")
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 99, "event listener handler_id")
    pos += 4
    assert_equal(_read_u16_le(w, buf, pos), 5, "event name length is 5")
    pos += 2 + 5

    # End
    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


# ── Max u32 values ───────────────────────────────────────────────────────────


fn test_max_u32_values(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Ensure the writer correctly encodes the maximum u32 value."""
    var buf = _alloc_buf(w)

    # 0xFFFFFFFF = 4294967295
    var off = Int(
        w[].call_i32("write_op_push_root", args_ptr_i32_i32(buf, 0, -1))
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_PUSH_ROOT)
    assert_equal(
        _read_u32_le(w, buf, 1), 4294967295, "max u32 encodes correctly"
    )

    _free_buf(w, buf)


fn test_zero_ids(w: UnsafePointer[WasmInstance, MutExternalOrigin]) raises:
    var buf = _alloc_buf(w)

    var off = Int(
        w[].call_i32("write_op_push_root", args_ptr_i32_i32(buf, 0, 0))
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_PUSH_ROOT)
    assert_equal(_read_u32_le(w, buf, 1), 0, "zero id encodes correctly")

    _free_buf(w, buf)


# ── Long string payload ─────────────────────────────────────────────────────


fn test_long_string_payload(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Test encoding a 1KB string in a text node."""
    var buf = Int(w[].call_i64("mutation_buf_alloc", args_i32(8192)))

    # Build a 1024-char string
    var long_str = String("")
    for _ in range(1024):
        long_str += "x"

    var text_ptr = w[].write_string_struct(long_str)
    var off = Int(
        w[].call_i32(
            "write_op_create_text_node",
            args_ptr_i32_i32_ptr(buf, 0, 1, text_ptr),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    assert_equal(_read_u8(w, buf, 0), OP_CREATE_TEXT_NODE)
    assert_equal(_read_u32_le(w, buf, 1), 1, "id is 1")
    assert_equal(_read_u32_le(w, buf, 5), 1024, "text length is 1024")

    # Verify all bytes are 'x'
    var all_x = True
    for i in range(1024):
        if _read_u8(w, buf, 9 + i) != Int(ord("x")):
            all_x = False
            break

    assert_true(all_x, "all 1024 bytes are 'x'")

    # End sentinel
    assert_equal(_read_u8(w, buf, 9 + 1024), OP_END)

    w[].call_void("mutation_buf_free", args_ptr(buf))


# ── Test sequence (composite integration test) ───────────────────────────────


fn test_write_test_sequence(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Test the write_test_sequence composite helper that writes 5 mutations."""
    var buf = _alloc_buf(w)

    var total = Int(w[].call_i32("write_test_sequence", args_ptr(buf)))
    assert_true(total > 0, "write_test_sequence wrote some bytes")

    # Sequence:
    #   1. LoadTemplate(tmpl_id=1, index=0, id=10)
    #   2. CreateTextNode(id=11, text="hello")
    #   3. AppendChildren(id=10, m=1)
    #   4. PushRoot(id=10)
    #   5. End

    var pos = 0

    # 1. LoadTemplate
    assert_equal(_read_u8(w, buf, pos), OP_LOAD_TEMPLATE)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 1, "tmpl_id is 1")
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 0, "index is 0")
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 10, "id is 10")
    pos += 4

    # 2. CreateTextNode
    assert_equal(_read_u8(w, buf, pos), OP_CREATE_TEXT_NODE)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 11, "text node id is 11")
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 5, "text length is 5")
    pos += 4
    assert_equal(_read_u8(w, buf, pos), Int(ord("h")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("e")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("l")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("l")))
    assert_equal(_read_u8(w, buf, pos + 4), Int(ord("o")))
    pos += 5

    # 3. AppendChildren
    assert_equal(_read_u8(w, buf, pos), OP_APPEND_CHILDREN)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 10, "append to id 10")
    pos += 4
    assert_equal(_read_u32_le(w, buf, pos), 1, "m is 1")
    pos += 4

    # 4. PushRoot
    assert_equal(_read_u8(w, buf, pos), OP_PUSH_ROOT)
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 10, "push root id 10")
    pos += 4

    # 5. End
    assert_equal(_read_u8(w, buf, pos), OP_END)

    _free_buf(w, buf)


# ── Debug ptr roundtrip ──────────────────────────────────────────────────────


fn test_debug_ptr_roundtrip(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Verify that debug_ptr_roundtrip correctly round-trips a pointer."""
    var buf = _alloc_buf(w)

    var result = Int(w[].call_i64("debug_ptr_roundtrip", args_ptr(buf)))
    assert_equal(result, buf, "pointer round-trips correctly")

    _free_buf(w, buf)


# ── Debug read/write byte ────────────────────────────────────────────────────


fn test_debug_read_write_byte(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    var buf = _alloc_buf(w)

    # Write various bytes and read them back
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(buf, 0, 0xAB))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(buf, 1, 0x00))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(buf, 2, 0xFF))

    assert_equal(_read_u8(w, buf, 0), 0xAB, "byte 0 is 0xAB")
    assert_equal(_read_u8(w, buf, 1), 0x00, "byte 1 is 0x00")
    assert_equal(_read_u8(w, buf, 2), 0xFF, "byte 2 is 0xFF")

    _free_buf(w, buf)


# ── All opcodes in one buffer ────────────────────────────────────────────────


fn test_all_opcodes_in_one_buffer(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Write one of each opcode into a single buffer and verify opcodes appear
    in the correct order."""
    var buf = _alloc_buf(w)

    var path_ptr = Int(w[].call_i64("mutation_buf_alloc", args_i32(1)))
    _ = w[].call_i32("debug_write_byte", args_ptr_i32_i32(path_ptr, 0, 0))

    var off = 0
    # Write one of each mutation type
    off = Int(
        w[].call_i32(
            "write_op_append_children",
            args_ptr_i32_i32_i32(buf, off, 1, 1),
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_assign_id",
            args_ptr_i32_ptr_i32_i32(buf, off, path_ptr, 1, 2),
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_create_placeholder", args_ptr_i32_i32(buf, off, 3)
        )
    )
    var t_ptr = w[].write_string_struct("t")
    off = Int(
        w[].call_i32(
            "write_op_create_text_node",
            args_ptr_i32_i32_ptr(buf, off, 4, t_ptr),
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_load_template",
            args_ptr_i32_i32_i32_i32(buf, off, 5, 0, 6),
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_replace_with", args_ptr_i32_i32_i32(buf, off, 7, 1)
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_replace_placeholder",
            args_ptr_i32_ptr_i32_i32(buf, off, path_ptr, 1, 1),
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_insert_after", args_ptr_i32_i32_i32(buf, off, 8, 1)
        )
    )
    off = Int(
        w[].call_i32(
            "write_op_insert_before", args_ptr_i32_i32_i32(buf, off, 9, 1)
        )
    )
    var a_name = w[].write_string_struct("a")
    var b_val = w[].write_string_struct("b")
    off = Int(
        w[].call_i32(
            "write_op_set_attribute",
            args_ptr_i32_i32_i32_ptr_ptr(buf, off, 10, 0, a_name, b_val),
        )
    )
    var x_ptr = w[].write_string_struct("x")
    off = Int(
        w[].call_i32(
            "write_op_set_text", args_ptr_i32_i32_ptr(buf, off, 11, x_ptr)
        )
    )
    var e_ptr = w[].write_string_struct("e")
    off = Int(
        w[].call_i32(
            "write_op_new_event_listener",
            args_ptr_i32_i32_i32_ptr(buf, off, 12, 0, e_ptr),
        )
    )
    var e_ptr2 = w[].write_string_struct("e")
    off = Int(
        w[].call_i32(
            "write_op_remove_event_listener",
            args_ptr_i32_i32_ptr(buf, off, 13, e_ptr2),
        )
    )
    off = Int(w[].call_i32("write_op_remove", args_ptr_i32_i32(buf, off, 14)))
    off = Int(
        w[].call_i32("write_op_push_root", args_ptr_i32_i32(buf, off, 15))
    )
    var ra_name = w[].write_string_struct("h")
    off = Int(
        w[].call_i32(
            "write_op_remove_attribute",
            args_ptr_i32_i32_i32_ptr(buf, off, 16, 0, ra_name),
        )
    )
    _ = w[].call_i32("write_op_end", args_ptr_i32(buf, off))

    # Walk through and extract just the opcodes
    var pos = 0

    # APPEND_CHILDREN: op(1) + id(4) + m(4) = 9
    assert_equal(_read_u8(w, buf, pos), OP_APPEND_CHILDREN)
    pos += 9

    # ASSIGN_ID: op(1) + path_len(1) + path(1) + id(4) = 7
    assert_equal(_read_u8(w, buf, pos), OP_ASSIGN_ID)
    pos += 7

    # CREATE_PLACEHOLDER: op(1) + id(4) = 5
    assert_equal(_read_u8(w, buf, pos), OP_CREATE_PLACEHOLDER)
    pos += 5

    # CREATE_TEXT_NODE: op(1) + id(4) + len(4) + "t"(1) = 10
    assert_equal(_read_u8(w, buf, pos), OP_CREATE_TEXT_NODE)
    pos += 10

    # LOAD_TEMPLATE: op(1) + tmpl_id(4) + index(4) + id(4) = 13
    assert_equal(_read_u8(w, buf, pos), OP_LOAD_TEMPLATE)
    pos += 13

    # REPLACE_WITH: op(1) + id(4) + m(4) = 9
    assert_equal(_read_u8(w, buf, pos), OP_REPLACE_WITH)
    pos += 9

    # REPLACE_PLACEHOLDER: op(1) + path_len(1) + path(1) + m(4) = 7
    assert_equal(_read_u8(w, buf, pos), OP_REPLACE_PLACEHOLDER)
    pos += 7

    # INSERT_AFTER: op(1) + id(4) + m(4) = 9
    assert_equal(_read_u8(w, buf, pos), OP_INSERT_AFTER)
    pos += 9

    # INSERT_BEFORE: op(1) + id(4) + m(4) = 9
    assert_equal(_read_u8(w, buf, pos), OP_INSERT_BEFORE)
    pos += 9

    # SET_ATTRIBUTE: op(1) + id(4) + ns(1) + name_len(2) + "a"(1) + val_len(4) + "b"(1) = 14
    assert_equal(_read_u8(w, buf, pos), OP_SET_ATTRIBUTE)
    pos += 14

    # SET_TEXT: op(1) + id(4) + len(4) + "x"(1) = 10
    assert_equal(_read_u8(w, buf, pos), OP_SET_TEXT)
    pos += 10

    # NEW_EVENT_LISTENER: op(1) + id(4) + handler_id(4) + name_len(2) + "e"(1) = 12
    assert_equal(_read_u8(w, buf, pos), OP_NEW_EVENT_LISTENER)
    pos += 12

    # REMOVE_EVENT_LISTENER: op(1) + id(4) + name_len(2) + "e"(1) = 8
    assert_equal(_read_u8(w, buf, pos), OP_REMOVE_EVENT_LISTENER)
    pos += 8

    # REMOVE: op(1) + id(4) = 5
    assert_equal(_read_u8(w, buf, pos), OP_REMOVE)
    pos += 5

    # PUSH_ROOT: op(1) + id(4) = 5
    assert_equal(_read_u8(w, buf, pos), OP_PUSH_ROOT)
    pos += 5

    # REMOVE_ATTRIBUTE: op(1) + id(4) + ns(1) + name_len(2) + "h"(1) = 9
    assert_equal(_read_u8(w, buf, pos), OP_REMOVE_ATTRIBUTE)
    pos += 9

    # END
    assert_equal(_read_u8(w, buf, pos), OP_END)

    w[].call_void("mutation_buf_free", args_ptr(path_ptr))
    _free_buf(w, buf)


# ── Template node / attr kind constants ──────────────────────────────────────

comptime TNODE_ELEMENT = 0x00
comptime TNODE_TEXT = 0x01
comptime TNODE_DYNAMIC = 0x02
comptime TNODE_DYNAMIC_TEXT = 0x03
comptime TATTR_STATIC = 0x00
comptime TATTR_DYNAMIC = 0x01

# HTML tag constants (matching src/vdom/tags.mojo)
comptime TAG_DIV = 0
comptime TAG_SPAN = 1
comptime TAG_BUTTON = 19


# ── Template registration helpers ────────────────────────────────────────────


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


fn _read_short_str_bytes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Tuple[Int, Int]:
    """Read a u16-length-prefixed string's length and return (length, new_offset).

    Does NOT read the string content itself — caller should verify byte-by-byte.
    """
    var slen = _read_u16_le(w, buf, offset)
    return Tuple(slen, offset + 2 + slen)


fn _read_str_bytes(
    w: UnsafePointer[WasmInstance, MutExternalOrigin], buf: Int, offset: Int
) raises -> Tuple[Int, Int]:
    """Read a u32-length-prefixed string's length and return (length, new_offset).

    Does NOT read the string content itself — caller should verify byte-by-byte.
    """
    var slen = _read_u32_le(w, buf, offset)
    return Tuple(slen, offset + 4 + slen)


# ── Register Template tests ──────────────────────────────────────────────────


fn test_register_template_minimal(
    w: UnsafePointer[WasmInstance, MutExternalOrigin]
) raises:
    """Serialize a minimal template: div > dyn_text[0].

    Template structure:
      node[0] = Element(TAG_DIV), children=[1], no attrs
      node[1] = DynamicText(index=0)
      root_indices = [0]

    Verifies: opcode, header fields, node encodings, root indices.
    """
    var rt = _create_runtime(w)
    var b = _create_builder(w, String("min"))

    # div (root, parent=-1)
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
    )
    # dyn_text[0] (parent=0 → div)
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 0, 0)
    )
    # Register
    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    # Serialize
    var buf = _alloc_buf(w)
    var off = Int(
        w[].call_i32(
            "write_op_register_template",
            args_ptr_i32_ptr_i32(buf, 0, rt, Int32(tmpl_id)),
        )
    )

    var pos = 0

    # Opcode
    assert_equal(
        _read_u8(w, buf, pos),
        OP_REGISTER_TEMPLATE,
        "opcode is REGISTER_TEMPLATE",
    )
    pos += 1

    # tmpl_id (u32)
    assert_equal(_read_u32_le(w, buf, pos), tmpl_id, "tmpl_id")
    pos += 4

    # name: u16 len + "min"
    assert_equal(_read_u16_le(w, buf, pos), 3, "name length is 3")
    pos += 2
    assert_equal(_read_u8(w, buf, pos), Int(ord("m")), "name[0]='m'")
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("i")), "name[1]='i'")
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("n")), "name[2]='n'")
    pos += 3

    # root_count (u16) = 1
    assert_equal(_read_u16_le(w, buf, pos), 1, "root_count is 1")
    pos += 2

    # node_count (u16) = 2
    assert_equal(_read_u16_le(w, buf, pos), 2, "node_count is 2")
    pos += 2

    # attr_count (u16) = 0
    assert_equal(_read_u16_le(w, buf, pos), 0, "attr_count is 0")
    pos += 2

    # ── Node 0: Element(TAG_DIV) ──
    assert_equal(
        _read_u8(w, buf, pos), TNODE_ELEMENT, "node[0] kind is ELEMENT"
    )
    pos += 1
    assert_equal(_read_u8(w, buf, pos), TAG_DIV, "node[0] tag is DIV")
    pos += 1
    # child_count = 1
    assert_equal(_read_u16_le(w, buf, pos), 1, "node[0] child_count is 1")
    pos += 2
    # child[0] = 1 (the dyn_text node)
    assert_equal(_read_u16_le(w, buf, pos), 1, "node[0] child[0] is 1")
    pos += 2
    # attr_first = 0, attr_count = 0
    assert_equal(_read_u16_le(w, buf, pos), 0, "node[0] attr_first is 0")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 0, "node[0] attr_count is 0")
    pos += 2

    # ── Node 1: DynamicText(index=0) ──
    assert_equal(
        _read_u8(w, buf, pos),
        TNODE_DYNAMIC_TEXT,
        "node[1] kind is DYNAMIC_TEXT",
    )
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 0, "node[1] dynamic_index is 0")
    pos += 4

    # ── No attributes ──

    # ── Root indices: [0] ──
    assert_equal(_read_u16_le(w, buf, pos), 0, "root[0] is 0")
    pos += 2

    # Verify total bytes written matches offset
    assert_equal(off, pos, "total bytes matches expected")

    _free_buf(w, buf)
    _destroy_runtime(w, rt)


fn test_register_template_with_text_and_attrs(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Serialize a template with static text, static attrs, and dynamic attrs.

    Template structure (counter-like):
      node[0] = Element(TAG_DIV), children=[1, 2]
      node[1] = Element(TAG_SPAN), children=[3]
      node[2] = Element(TAG_BUTTON), children=[4]
        static_attr: type="submit"
        dynamic_attr: index=0
      node[3] = DynamicText(index=0)
      node[4] = Text("+")
      root_indices = [0]

    Verifies: text node encoding, static/dynamic attr encoding.
    """
    var rt = _create_runtime(w)
    var b = _create_builder(w, String("ctr"))

    # div (root)
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
    )
    # span (child of div)
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_SPAN, 0)
    )
    # button (child of div)
    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_BUTTON, 0)
    )
    # dyn_text[0] (child of span)
    _ = w[].call_i32(
        "tmpl_builder_push_dynamic_text", args_ptr_i32_i32(b, 0, 1)
    )
    # text "+" (child of button)
    var plus_ptr = w[].write_string_struct("+")
    _ = w[].call_i32("tmpl_builder_push_text", args_ptr_ptr_i32(b, plus_ptr, 2))
    # static attr on button: type="submit"
    var type_ptr = w[].write_string_struct("type")
    var submit_ptr = w[].write_string_struct("submit")
    w[].call_void(
        "tmpl_builder_push_static_attr",
        args_ptr_i32_ptr_ptr(b, 2, type_ptr, submit_ptr),
    )
    # dynamic attr on button: index=0
    w[].call_void("tmpl_builder_push_dynamic_attr", args_ptr_i32_i32(b, 2, 0))

    # Register
    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    # Serialize
    var buf = _alloc_buf(w)
    var off = Int(
        w[].call_i32(
            "write_op_register_template",
            args_ptr_i32_ptr_i32(buf, 0, rt, Int32(tmpl_id)),
        )
    )

    var pos = 0

    # Opcode
    assert_equal(_read_u8(w, buf, pos), OP_REGISTER_TEMPLATE)
    pos += 1

    # tmpl_id
    assert_equal(_read_u32_le(w, buf, pos), tmpl_id)
    pos += 4

    # name: "ctr"
    assert_equal(_read_u16_le(w, buf, pos), 3, "name length")
    pos += 2 + 3  # skip "ctr"

    # root_count = 1
    assert_equal(_read_u16_le(w, buf, pos), 1, "root_count")
    pos += 2

    # node_count = 5
    assert_equal(_read_u16_le(w, buf, pos), 5, "node_count")
    pos += 2

    # attr_count = 2 (1 static + 1 dynamic)
    assert_equal(_read_u16_le(w, buf, pos), 2, "attr_count")
    pos += 2

    # ── Node 0: Element(TAG_DIV), 2 children [1,2], 0 attrs ──
    assert_equal(_read_u8(w, buf, pos), TNODE_ELEMENT, "n0 kind")
    pos += 1
    assert_equal(_read_u8(w, buf, pos), TAG_DIV, "n0 tag")
    pos += 1
    assert_equal(_read_u16_le(w, buf, pos), 2, "n0 child_count")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 1, "n0 child[0]")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 2, "n0 child[1]")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 0, "n0 attr_first")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 0, "n0 attr_count")
    pos += 2

    # ── Node 1: Element(TAG_SPAN), 1 child [3], 0 attrs ──
    assert_equal(_read_u8(w, buf, pos), TNODE_ELEMENT, "n1 kind")
    pos += 1
    assert_equal(_read_u8(w, buf, pos), TAG_SPAN, "n1 tag")
    pos += 1
    assert_equal(_read_u16_le(w, buf, pos), 1, "n1 child_count")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 3, "n1 child[0]")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 0, "n1 attr_first")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 0, "n1 attr_count")
    pos += 2

    # ── Node 2: Element(TAG_BUTTON), 1 child [4], 2 attrs at first=0 ──
    assert_equal(_read_u8(w, buf, pos), TNODE_ELEMENT, "n2 kind")
    pos += 1
    assert_equal(_read_u8(w, buf, pos), TAG_BUTTON, "n2 tag")
    pos += 1
    assert_equal(_read_u16_le(w, buf, pos), 1, "n2 child_count")
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 4, "n2 child[0]")
    pos += 2
    # attr_first and attr_count set by TemplateBuilder
    var n2_attr_first = _read_u16_le(w, buf, pos)
    pos += 2
    assert_equal(_read_u16_le(w, buf, pos), 2, "n2 attr_count")
    pos += 2

    # ── Node 3: DynamicText(index=0) ──
    assert_equal(_read_u8(w, buf, pos), TNODE_DYNAMIC_TEXT, "n3 kind")
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 0, "n3 dynamic_index")
    pos += 4

    # ── Node 4: Text("+") ──
    assert_equal(_read_u8(w, buf, pos), TNODE_TEXT, "n4 kind")
    pos += 1
    # u32 text length = 1
    assert_equal(_read_u32_le(w, buf, pos), 1, "n4 text length")
    pos += 4
    assert_equal(_read_u8(w, buf, pos), Int(ord("+")), "n4 text is '+'")
    pos += 1

    # ── Attr 0: Static("type", "submit") ──
    assert_equal(_read_u8(w, buf, pos), TATTR_STATIC, "a0 kind static")
    pos += 1
    # name: u16 len + "type"
    assert_equal(_read_u16_le(w, buf, pos), 4, "a0 name len")
    pos += 2
    assert_equal(_read_u8(w, buf, pos), Int(ord("t")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("y")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("p")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("e")))
    pos += 4
    # value: u32 len + "submit"
    assert_equal(_read_u32_le(w, buf, pos), 6, "a0 value len")
    pos += 4
    assert_equal(_read_u8(w, buf, pos), Int(ord("s")))
    assert_equal(_read_u8(w, buf, pos + 1), Int(ord("u")))
    assert_equal(_read_u8(w, buf, pos + 2), Int(ord("b")))
    assert_equal(_read_u8(w, buf, pos + 3), Int(ord("m")))
    assert_equal(_read_u8(w, buf, pos + 4), Int(ord("i")))
    assert_equal(_read_u8(w, buf, pos + 5), Int(ord("t")))
    pos += 6

    # ── Attr 1: Dynamic(index=0) ──
    assert_equal(_read_u8(w, buf, pos), TATTR_DYNAMIC, "a1 kind dynamic")
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 0, "a1 dynamic_index")
    pos += 4

    # ── Root indices: [0] ──
    assert_equal(_read_u16_le(w, buf, pos), 0, "root[0]")
    pos += 2

    assert_equal(off, pos, "total bytes matches")

    _free_buf(w, buf)
    _destroy_runtime(w, rt)


fn test_register_template_with_dynamic_node(
    w: UnsafePointer[WasmInstance, MutExternalOrigin],
) raises:
    """Serialize a template with a Dynamic node slot (not DynamicText).

    Template: div > dynamic[0]
      node[0] = Element(TAG_DIV), children=[1]
      node[1] = Dynamic(index=0)
      root_indices = [0]
    """
    var rt = _create_runtime(w)
    var b = _create_builder(w, String("dyn"))

    _ = w[].call_i32(
        "tmpl_builder_push_element", args_ptr_i32_i32(b, TAG_DIV, -1)
    )
    _ = w[].call_i32("tmpl_builder_push_dynamic", args_ptr_i32_i32(b, 0, 0))
    var tmpl_id = Int(
        w[].call_i32("tmpl_builder_register", args_ptr_ptr(rt, b))
    )
    _destroy_builder(w, b)

    var buf = _alloc_buf(w)
    var off = Int(
        w[].call_i32(
            "write_op_register_template",
            args_ptr_i32_ptr_i32(buf, 0, rt, Int32(tmpl_id)),
        )
    )

    # Skip header: op(1) + id(4) + name_u16(2) + "dyn"(3) + root(2) + node(2) + attr(2)
    var pos = 1 + 4 + 2 + 3 + 2 + 2 + 2  # = 16

    # Node 0: Element(TAG_DIV), 1 child
    assert_equal(_read_u8(w, buf, pos), TNODE_ELEMENT, "n0 element")
    pos += 1  # kind
    pos += 1  # tag
    assert_equal(_read_u16_le(w, buf, pos), 1, "n0 1 child")
    pos += 2  # child_count
    pos += 2  # child[0]
    pos += 2  # attr_first
    pos += 2  # attr_count

    # Node 1: Dynamic(index=0)
    assert_equal(_read_u8(w, buf, pos), TNODE_DYNAMIC, "n1 kind is DYNAMIC")
    pos += 1
    assert_equal(_read_u32_le(w, buf, pos), 0, "n1 dynamic_index")
    pos += 4

    # Root indices
    assert_equal(_read_u16_le(w, buf, pos), 0, "root[0]")
    pos += 2

    assert_equal(off, pos, "total bytes")

    _free_buf(w, buf)
    _destroy_runtime(w, rt)


fn main() raises:
    from wasm_harness import get_instance

    var w = get_instance()
    test_end_sentinel(w)
    test_empty_buffer_starts_at_zero(w)
    test_writer_with_initial_offset(w)
    test_append_children(w)
    test_append_children_zero(w)
    test_create_placeholder(w)
    test_create_text_node(w)
    test_create_text_node_empty_string(w)
    test_create_text_node_unicode(w)
    test_load_template(w)
    test_replace_with(w)
    test_insert_after(w)
    test_insert_before(w)
    test_remove(w)
    test_push_root(w)
    test_set_text(w)
    test_set_attribute(w)
    test_set_attribute_with_namespace(w)
    test_set_attribute_empty_value(w)
    test_remove_attribute(w)
    test_remove_attribute_with_namespace(w)
    test_new_event_listener(w)
    test_remove_event_listener(w)
    test_assign_id(w)
    test_assign_id_empty_path(w)
    test_assign_id_single_element_path(w)
    test_replace_placeholder(w)
    test_multiple_mutations_in_sequence(w)
    test_mixed_mutations_with_strings(w)
    test_max_u32_values(w)
    test_zero_ids(w)
    test_long_string_payload(w)
    test_write_test_sequence(w)
    test_debug_ptr_roundtrip(w)
    test_debug_read_write_byte(w)
    test_all_opcodes_in_one_buffer(w)
    test_register_template_minimal(w)
    test_register_template_with_text_and_attrs(w)
    test_register_template_with_dynamic_node(w)
    print("protocol: 39/39 passed")
