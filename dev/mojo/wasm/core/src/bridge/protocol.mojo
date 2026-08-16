# Mutation Buffer Protocol
#
# Binary encoding for stack-based DOM mutations sent from Mojo (WASM) to JS.
# Each mutation is a variable-length record: [op: u8, ...payload].
# All multi-byte integers are little-endian (matching WASM native endianness).
# Strings are length-prefixed: u32 length + UTF-8 bytes (or u16 for short strings).
# The End sentinel (0x00) marks the end of a mutation sequence.

from vdom.template import (
    Template,
    TemplateNode,
    TemplateAttribute,
    TNODE_ELEMENT,
    TNODE_TEXT,
    TNODE_DYNAMIC,
    TNODE_DYNAMIC_TEXT,
    TATTR_STATIC,
    TATTR_DYNAMIC,
)


# ── Opcodes ──────────────────────────────────────────────────────────────────

comptime OP_END = UInt8(0x00)
comptime OP_APPEND_CHILDREN = UInt8(0x01)
comptime OP_ASSIGN_ID = UInt8(0x02)
comptime OP_CREATE_PLACEHOLDER = UInt8(0x03)
comptime OP_CREATE_TEXT_NODE = UInt8(0x04)
comptime OP_LOAD_TEMPLATE = UInt8(0x05)
comptime OP_REPLACE_WITH = UInt8(0x06)
comptime OP_REPLACE_PLACEHOLDER = UInt8(0x07)
comptime OP_INSERT_AFTER = UInt8(0x08)
comptime OP_INSERT_BEFORE = UInt8(0x09)
comptime OP_SET_ATTRIBUTE = UInt8(0x0A)
comptime OP_SET_TEXT = UInt8(0x0B)
comptime OP_NEW_EVENT_LISTENER = UInt8(0x0C)
comptime OP_REMOVE_EVENT_LISTENER = UInt8(0x0D)
comptime OP_REMOVE = UInt8(0x0E)
comptime OP_PUSH_ROOT = UInt8(0x0F)
comptime OP_REGISTER_TEMPLATE = UInt8(0x10)
comptime OP_REMOVE_ATTRIBUTE = UInt8(0x11)


# ── MutationWriter ───────────────────────────────────────────────────────────


struct MutationWriter(Movable):
    """Writes binary-encoded DOM mutations to a pre-allocated buffer.

    The buffer must be allocated in WASM linear memory so the JS interpreter
    can read from it via the shared `WebAssembly.Memory`.  The caller is
    responsible for ensuring the buffer is large enough for all mutations
    written before `finalize()` is called.
    """

    var buf: UnsafePointer[UInt8, MutExternalOrigin]
    var offset: Int
    var capacity: Int

    fn __moveinit__(out self, deinit other: Self):
        self.buf = other.buf
        self.offset = other.offset
        self.capacity = other.capacity

    # ── Construction ─────────────────────────────────────────────────────

    fn __init__(
        out self, buf: UnsafePointer[UInt8, MutExternalOrigin], capacity: Int
    ):
        """Create a writer that starts at the beginning of `buf`."""
        self.buf = buf
        self.offset = 0
        self.capacity = capacity

    fn __init__(
        out self,
        buf: UnsafePointer[UInt8, MutExternalOrigin],
        offset: Int,
        capacity: Int,
    ):
        """Create a writer that starts at `offset` within `buf`."""
        self.buf = buf
        self.offset = offset
        self.capacity = capacity

    # ── Primitive encoders ───────────────────────────────────────────────

    @always_inline
    fn _write_u8(mut self, val: UInt8):
        self.buf[self.offset] = val
        self.offset += 1

    @always_inline
    fn _write_u16_le(mut self, val: UInt16):
        self.buf[self.offset] = UInt8(val & 0xFF)
        self.buf[self.offset + 1] = UInt8((val >> 8) & 0xFF)
        self.offset += 2

    @always_inline
    fn _write_u32_le(mut self, val: UInt32):
        self.buf[self.offset] = UInt8(val & 0xFF)
        self.buf[self.offset + 1] = UInt8((val >> 8) & 0xFF)
        self.buf[self.offset + 2] = UInt8((val >> 16) & 0xFF)
        self.buf[self.offset + 3] = UInt8((val >> 24) & 0xFF)
        self.offset += 4

    fn _write_str(mut self, text: String):
        """Write a u32-length-prefixed UTF-8 string."""
        var text_len = len(text)
        self._write_u32_le(UInt32(text_len))
        var ptr = text.unsafe_ptr()
        for i in range(text_len):
            self.buf[self.offset + i] = ptr[i]
        self.offset += text_len

    fn _write_short_str(mut self, text: String):
        """Write a u16-length-prefixed UTF-8 string (for names ≤ 65535 bytes).
        """
        var text_len = len(text)
        self._write_u16_le(UInt16(text_len))
        var ptr = text.unsafe_ptr()
        for i in range(text_len):
            self.buf[self.offset + i] = ptr[i]
        self.offset += text_len

    fn _write_path[
        origin: Origin
    ](mut self, path_ptr: UnsafePointer[UInt8, origin], path_len: Int):
        """Write a u8-length-prefixed byte path (template traversal indices)."""
        self._write_u8(UInt8(path_len))
        for i in range(path_len):
            self.buf[self.offset] = path_ptr[i]
            self.offset += 1

    # ── Mutation operations ──────────────────────────────────────────────

    fn end(mut self):
        """Write the End sentinel (0x00). Must be called after all mutations."""
        self._write_u8(OP_END)

    fn finalize(mut self):
        """Alias for `end()` — write the End sentinel to terminate the buffer.
        """
        self.end()

    fn append_children(mut self, id: UInt32, m: UInt32):
        """Pop `m` nodes from the stack and append them as children of `id`.

        | op (u8) | id (u32) | m (u32) |
        """
        self._write_u8(OP_APPEND_CHILDREN)
        self._write_u32_le(id)
        self._write_u32_le(m)

    fn assign_id[
        origin: Origin
    ](
        mut self,
        path_ptr: UnsafePointer[UInt8, origin],
        path_len: Int,
        id: UInt32,
    ):
        """Assign an ElementId to the node at `path` inside the current template.

        | op (u8) | path_len (u8) | path ([u8]) | id (u32) |
        """
        self._write_u8(OP_ASSIGN_ID)
        self._write_path(path_ptr, path_len)
        self._write_u32_le(id)

    fn create_placeholder(mut self, id: UInt32):
        """Create an empty placeholder node (for conditional/suspended content).

        | op (u8) | id (u32) |
        """
        self._write_u8(OP_CREATE_PLACEHOLDER)
        self._write_u32_le(id)

    fn create_text_node(mut self, id: UInt32, text: String):
        """Create a text node with the given content and id.

        | op (u8) | id (u32) | len (u32) | text ([u8]) |
        """
        self._write_u8(OP_CREATE_TEXT_NODE)
        self._write_u32_le(id)
        self._write_str(text)

    fn load_template(mut self, tmpl_id: UInt32, index: UInt32, id: UInt32):
        """Clone a pre-compiled template and assign it an ElementId.

        | op (u8) | tmpl_id (u32) | index (u32) | id (u32) |
        """
        self._write_u8(OP_LOAD_TEMPLATE)
        self._write_u32_le(tmpl_id)
        self._write_u32_le(index)
        self._write_u32_le(id)

    fn replace_with(mut self, id: UInt32, m: UInt32):
        """Replace node `id` with `m` nodes popped from the stack.

        | op (u8) | id (u32) | m (u32) |
        """
        self._write_u8(OP_REPLACE_WITH)
        self._write_u32_le(id)
        self._write_u32_le(m)

    fn replace_placeholder[
        origin: Origin
    ](
        mut self,
        path_ptr: UnsafePointer[UInt8, origin],
        path_len: Int,
        m: UInt32,
    ):
        """Replace the placeholder at `path` with `m` nodes from the stack.

        | op (u8) | path_len (u8) | path ([u8]) | m (u32) |
        """
        self._write_u8(OP_REPLACE_PLACEHOLDER)
        self._write_path(path_ptr, path_len)
        self._write_u32_le(m)

    fn insert_after(mut self, id: UInt32, m: UInt32):
        """Insert `m` stack nodes after node `id`.

        | op (u8) | id (u32) | m (u32) |
        """
        self._write_u8(OP_INSERT_AFTER)
        self._write_u32_le(id)
        self._write_u32_le(m)

    fn insert_before(mut self, id: UInt32, m: UInt32):
        """Insert `m` stack nodes before node `id`.

        | op (u8) | id (u32) | m (u32) |
        """
        self._write_u8(OP_INSERT_BEFORE)
        self._write_u32_le(id)
        self._write_u32_le(m)

    fn set_attribute(
        mut self, id: UInt32, ns: UInt8, name: String, value: String
    ):
        """Set an attribute on the element with the given id.

        `ns` is a namespace tag (0 = no namespace, 1 = xlink, etc.).

        | op (u8) | id (u32) | ns (u8) | name_len (u16) | name ([u8]) | val_len (u32) | val ([u8]) |
        """
        self._write_u8(OP_SET_ATTRIBUTE)
        self._write_u32_le(id)
        self._write_u8(ns)
        self._write_short_str(name)
        self._write_str(value)

    fn remove_attribute(mut self, id: UInt32, ns: UInt8, name: String):
        """Remove an attribute from the element with the given id.

        Used for boolean attributes (disabled, checked, hidden, etc.)
        where removing the attribute is semantically different from
        setting it to an empty string.

        `ns` is a namespace tag (0 = no namespace, 1 = xlink, etc.).

        | op (u8) | id (u32) | ns (u8) | name_len (u16) | name ([u8]) |
        """
        self._write_u8(OP_REMOVE_ATTRIBUTE)
        self._write_u32_le(id)
        self._write_u8(ns)
        self._write_short_str(name)

    fn set_text(mut self, id: UInt32, text: String):
        """Update the text content of node `id`.

        | op (u8) | id (u32) | len (u32) | text ([u8]) |
        """
        self._write_u8(OP_SET_TEXT)
        self._write_u32_le(id)
        self._write_str(text)

    fn new_event_listener(
        mut self, id: UInt32, handler_id: UInt32, name: String
    ):
        """Attach an event listener to element `id`.

        The handler_id identifies the WASM-side event handler so the JS
        runtime can dispatch events back without external mapping.

        | op (u8) | id (u32) | handler_id (u32) | name_len (u16) | name ([u8]) |
        """
        self._write_u8(OP_NEW_EVENT_LISTENER)
        self._write_u32_le(id)
        self._write_u32_le(handler_id)
        self._write_short_str(name)

    fn remove_event_listener(mut self, id: UInt32, name: String):
        """Remove an event listener from element `id`.

        | op (u8) | id (u32) | name_len (u16) | name ([u8]) |
        """
        self._write_u8(OP_REMOVE_EVENT_LISTENER)
        self._write_u32_le(id)
        self._write_short_str(name)

    fn remove(mut self, id: UInt32):
        """Remove node `id` from the DOM.

        | op (u8) | id (u32) |
        """
        self._write_u8(OP_REMOVE)
        self._write_u32_le(id)

    fn push_root(mut self, id: UInt32):
        """Push node `id` onto the stack.

        | op (u8) | id (u32) |
        """
        self._write_u8(OP_PUSH_ROOT)
        self._write_u32_le(id)

    # ── Template registration ────────────────────────────────────────────

    fn register_template(mut self, tmpl: Template):
        """Serialize a Template's full static structure into the buffer.

        The JS interpreter uses this to build cloneable DOM template roots
        without the app manually constructing them in JavaScript.

        Wire format:
          | op (u8)                          — OP_REGISTER_TEMPLATE
          | tmpl_id (u32)                    — template registry ID
          | name_len (u16) | name ([u8])     — template name
          | root_count (u16)                 — number of root node indices
          | node_count (u16)                 — total nodes in flat array
          | attr_count (u16)                 — total attributes in flat array
          | [nodes × node_count]             — serialized TemplateNodes
          | [attrs × attr_count]             — serialized TemplateAttributes
          | [root_indices × root_count as u16]

        Node wire format (kind-tagged):
          Element:     | 0x00 | tag (u8) | child_count (u16) | [child_indices as u16…] | attr_first (u16) | attr_count (u16) |
          Text:        | 0x01 | text_len (u32) | text ([u8]) |
          Dynamic:     | 0x02 | dynamic_index (u32) |
          DynamicText: | 0x03 | dynamic_index (u32) |

        Attribute wire format (kind-tagged):
          Static:  | 0x00 | name_len (u16) | name | value_len (u32) | value |
          Dynamic: | 0x01 | dynamic_index (u32) |
        """
        self._write_u8(OP_REGISTER_TEMPLATE)

        # Header
        self._write_u32_le(tmpl.id)
        self._write_short_str(tmpl.name)
        self._write_u16_le(UInt16(tmpl.root_count()))
        self._write_u16_le(UInt16(tmpl.node_count()))
        self._write_u16_le(UInt16(tmpl.attr_total_count()))

        # Nodes
        for i in range(tmpl.node_count()):
            var node_ptr = tmpl.get_node_ptr(i)
            var kind = node_ptr[0].kind
            self._write_u8(kind)
            if kind == TNODE_ELEMENT:
                self._write_u8(node_ptr[0].html_tag)
                var cc = node_ptr[0].child_count()
                self._write_u16_le(UInt16(cc))
                for c in range(cc):
                    self._write_u16_le(UInt16(node_ptr[0].child_at(c)))
                self._write_u16_le(UInt16(node_ptr[0].first_attr))
                self._write_u16_le(UInt16(node_ptr[0].num_attrs))
            elif kind == TNODE_TEXT:
                self._write_str(node_ptr[0].text)
            elif kind == TNODE_DYNAMIC:
                self._write_u32_le(node_ptr[0].dynamic_index)
            elif kind == TNODE_DYNAMIC_TEXT:
                self._write_u32_le(node_ptr[0].dynamic_index)

        # Attributes
        for i in range(tmpl.attr_total_count()):
            var a = tmpl.get_attr(i)
            self._write_u8(a.kind)
            if a.kind == TATTR_STATIC:
                self._write_short_str(a.name)
                self._write_str(a.value)
            elif a.kind == TATTR_DYNAMIC:
                self._write_u32_le(a.dynamic_index)

        # Root indices
        for i in range(tmpl.root_count()):
            self._write_u16_le(UInt16(tmpl.get_root_index(i)))
