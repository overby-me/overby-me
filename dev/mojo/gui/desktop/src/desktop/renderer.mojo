# Mutation Interpreter — Reads binary opcodes and translates to Blitz FFI calls.
#
# This module is the Mojo-side equivalent of the JS `Interpreter` class in
# the web renderer. It reads the binary mutation buffer produced by
# mojo-gui/core's MutationWriter and translates each opcode into the
# corresponding Blitz C FFI call via the `Blitz` wrapper.
#
# Architecture:
#
#   MutationWriter (core)
#     │ writes binary opcodes to heap buffer
#     ▼
#   MutationInterpreter (this module)
#     │ reads opcodes, decodes payloads
#     │ calls Blitz FFI via the Blitz wrapper
#     ▼
#   libmojo_blitz (Rust cdylib)
#     │ manipulates blitz-dom
#     ▼
#   Blitz (Stylo + Taffy + Vello)
#
# The interpreter maintains a stack of Blitz node IDs, mirroring the JS
# interpreter's node stack. Opcodes like PUSH_ROOT push nodes onto the
# stack; opcodes like APPEND_CHILDREN pop from it.
#
# Opcode reference (from core/src/bridge/protocol.mojo):
#
#   0x00  OP_END                  — End of mutation sequence
#   0x01  OP_APPEND_CHILDREN      — Pop m nodes, append to parent id
#   0x02  OP_ASSIGN_ID            — Assign element ID to node at path
#   0x03  OP_CREATE_PLACEHOLDER   — Create placeholder with id
#   0x04  OP_CREATE_TEXT_NODE     — Create text node with id and text
#   0x05  OP_LOAD_TEMPLATE        — Clone template, assign id, push to stack
#   0x06  OP_REPLACE_WITH         — Replace node id with m stack nodes
#   0x07  OP_REPLACE_PLACEHOLDER  — Replace placeholder at path with m stack nodes
#   0x08  OP_INSERT_AFTER         — Insert m stack nodes after id
#   0x09  OP_INSERT_BEFORE        — Insert m stack nodes before id
#   0x0A  OP_SET_ATTRIBUTE        — Set attribute on id
#   0x0B  OP_SET_TEXT             — Set text content of id
#   0x0C  OP_NEW_EVENT_LISTENER   — Add event listener on id
#   0x0D  OP_REMOVE_EVENT_LISTENER— Remove event listener from id
#   0x0E  OP_REMOVE               — Remove node id from DOM
#   0x0F  OP_PUSH_ROOT            — Push node id onto the stack
#   0x10  OP_REGISTER_TEMPLATE    — Register a template definition
#   0x11  OP_REMOVE_ATTRIBUTE     — Remove attribute from id
#
# Wire format details are documented in core/src/bridge/protocol.mojo.
#
# Usage (from desktop/src/desktop/launcher.mojo):
#
#   var interpreter = MutationInterpreter(blitz)
#   var mount_len = app.mount(writer_ptr)
#   if mount_len > 0:
#       blitz.begin_mutations()
#       interpreter.apply(buf_ptr, Int(mount_len))
#       blitz.end_mutations()
#       blitz.request_redraw()

from std.memory import Pointer, alloc
from .blitz import Blitz
from html.tags import tag_name


# ══════════════════════════════════════════════════════════════════════════════
# Opcodes — must match core/src/bridge/protocol.mojo exactly
# ══════════════════════════════════════════════════════════════════════════════

comptime OP_END: UInt8 = 0x00
comptime OP_APPEND_CHILDREN: UInt8 = 0x01
comptime OP_ASSIGN_ID: UInt8 = 0x02
comptime OP_CREATE_PLACEHOLDER: UInt8 = 0x03
comptime OP_CREATE_TEXT_NODE: UInt8 = 0x04
comptime OP_LOAD_TEMPLATE: UInt8 = 0x05
comptime OP_REPLACE_WITH: UInt8 = 0x06
comptime OP_REPLACE_PLACEHOLDER: UInt8 = 0x07
comptime OP_INSERT_AFTER: UInt8 = 0x08
comptime OP_INSERT_BEFORE: UInt8 = 0x09
comptime OP_SET_ATTRIBUTE: UInt8 = 0x0A
comptime OP_SET_TEXT: UInt8 = 0x0B
comptime OP_NEW_EVENT_LISTENER: UInt8 = 0x0C
comptime OP_REMOVE_EVENT_LISTENER: UInt8 = 0x0D
comptime OP_REMOVE: UInt8 = 0x0E
comptime OP_PUSH_ROOT: UInt8 = 0x0F
comptime OP_REGISTER_TEMPLATE: UInt8 = 0x10
comptime OP_REMOVE_ATTRIBUTE: UInt8 = 0x11

# ── Template node kinds (from core/src/vdom/template.mojo) ───────────────

comptime TNODE_ELEMENT: UInt8 = 0x00
comptime TNODE_TEXT: UInt8 = 0x01
comptime TNODE_DYNAMIC: UInt8 = 0x02
comptime TNODE_DYNAMIC_TEXT: UInt8 = 0x03

# ── Template attribute kinds ─────────────────────────────────────────────

comptime TATTR_STATIC: UInt8 = 0x00
comptime TATTR_DYNAMIC: UInt8 = 0x01


# ══════════════════════════════════════════════════════════════════════════════
# BufReader — Reads primitive values from a byte buffer
# ══════════════════════════════════════════════════════════════════════════════


struct BufReader:
    """Reads little-endian integers and length-prefixed strings from a byte buffer.

    This mirrors the encoding logic in MutationWriter, reading back the
    same format:
      - u8:  single byte
      - u16: 2 bytes little-endian
      - u32: 4 bytes little-endian
      - str: u32 length prefix + UTF-8 bytes
      - short_str: u16 length prefix + UTF-8 bytes
      - path: u8 length prefix + byte array
    """

    var buf: Pointer[UInt8, MutUntrackedOrigin]
    var offset: Int
    var length: Int

    def __init__(
        out self, buf: Pointer[UInt8, MutUntrackedOrigin], length: Int
    ):
        self.buf = buf
        self.offset = 0
        self.length = length

    def has_remaining(self) -> Bool:
        """Check if there are more bytes to read."""
        return self.offset < self.length

    @always_inline
    def read_u8(mut self) -> UInt8:
        """Read a single byte."""
        var val = self.buf[self.offset]
        self.offset += 1
        return val

    @always_inline
    def read_u16_le(mut self) -> UInt16:
        """Read a 16-bit little-endian unsigned integer."""
        var lo = UInt16(self.buf[self.offset])
        var hi = UInt16(self.buf[self.offset + 1])
        self.offset += 2
        return lo | (hi << 8)

    @always_inline
    def read_u32_le(mut self) -> UInt32:
        """Read a 32-bit little-endian unsigned integer."""
        var b0 = UInt32(self.buf[self.offset])
        var b1 = UInt32(self.buf[self.offset + 1])
        var b2 = UInt32(self.buf[self.offset + 2])
        var b3 = UInt32(self.buf[self.offset + 3])
        self.offset += 4
        return b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)

    def read_str(mut self) -> String:
        """Read a u32-length-prefixed UTF-8 string."""
        var str_len = Int(self.read_u32_le())
        if str_len == 0:
            return String("")
        var result = String("")
        for i in range(str_len):
            # Build string byte by byte
            var byte = self.buf[self.offset + i]
            result += chr(Int(byte))
        self.offset += str_len
        return result

    def read_short_str(mut self) -> String:
        """Read a u16-length-prefixed UTF-8 string (for names <= 65535 bytes).
        """
        var str_len = Int(self.read_u16_le())
        if str_len == 0:
            return String("")
        var result = String("")
        for i in range(str_len):
            var byte = self.buf[self.offset + i]
            result += chr(Int(byte))
        self.offset += str_len
        return result

    def read_path_len(mut self) -> Int:
        """Read a u8 path length."""
        return Int(self.read_u8())

    def read_path_bytes(
        mut self, path_len: Int
    ) -> Pointer[UInt8, MutUntrackedOrigin]:
        """Read path_len bytes and return a pointer to the start.

        The pointer points directly into the buffer. The caller must not
        free it and must use it before the buffer is freed/overwritten.

        Returns:
            Pointer to the start of the path bytes in the buffer.
        """
        var ptr = self.buf + self.offset
        self.offset += path_len
        return ptr

    def skip(mut self, count: Int):
        """Skip forward by count bytes."""
        self.offset += count


# ══════════════════════════════════════════════════════════════════════════════
# MutationInterpreter — The core opcode interpreter
# ══════════════════════════════════════════════════════════════════════════════


struct MutationInterpreter(Movable):
    """Interprets binary mutation opcodes and applies them to the Blitz DOM.

    This is the Mojo-side port of the JS `Interpreter` class. It reads
    opcodes from a byte buffer and translates each one into Blitz C FFI
    calls via the `Blitz` wrapper.

    The interpreter maintains:
      - A stack of Blitz node IDs (for PUSH_ROOT / APPEND_CHILDREN / etc.)
      - A reference to the Blitz FFI wrapper

    Lifetime: The interpreter borrows the Blitz instance. It must not
    outlive the Blitz context.
    """

    var _blitz: Pointer[Blitz, MutUntrackedOrigin]
    var _stack: List[UInt32]

    def __init__(out self, mut blitz: Blitz):
        """Create a mutation interpreter backed by the given Blitz instance.

        Args:
            blitz: Reference to the Blitz FFI wrapper. The interpreter
                   borrows this; the caller must keep it alive.
        """
        # Store a pointer to the Blitz instance. This is an unsafe borrow —
        # the caller guarantees the Blitz instance outlives the interpreter.
        self._blitz = Pointer(to=blitz).unsafe_origin_cast[MutUntrackedOrigin]()
        self._stack = List[UInt32](capacity=64)

    def __init__(out self, *, deinit move: Self):
        self._blitz = move._blitz
        self._stack = move._stack^

    # ── Public API ───────────────────────────────────────────────────────

    def apply(mut self, buf: Pointer[UInt8, MutUntrackedOrigin], length: Int):
        """Apply all mutations in the given buffer to the Blitz DOM.

        Reads opcodes sequentially from the buffer until OP_END is
        encountered or the buffer is exhausted.

        Args:
            buf: Pointer to the binary mutation buffer.
            length: Number of valid bytes in the buffer.
        """
        var reader = BufReader(buf, length)

        while reader.has_remaining():
            var op = reader.read_u8()

            if op == OP_END:
                break
            elif op == OP_APPEND_CHILDREN:
                self._op_append_children(reader)
            elif op == OP_ASSIGN_ID:
                self._op_assign_id(reader)
            elif op == OP_CREATE_PLACEHOLDER:
                self._op_create_placeholder(reader)
            elif op == OP_CREATE_TEXT_NODE:
                self._op_create_text_node(reader)
            elif op == OP_LOAD_TEMPLATE:
                self._op_load_template(reader)
            elif op == OP_REPLACE_WITH:
                self._op_replace_with(reader)
            elif op == OP_REPLACE_PLACEHOLDER:
                self._op_replace_placeholder(reader)
            elif op == OP_INSERT_AFTER:
                self._op_insert_after(reader)
            elif op == OP_INSERT_BEFORE:
                self._op_insert_before(reader)
            elif op == OP_SET_ATTRIBUTE:
                self._op_set_attribute(reader)
            elif op == OP_SET_TEXT:
                self._op_set_text(reader)
            elif op == OP_NEW_EVENT_LISTENER:
                self._op_new_event_listener(reader)
            elif op == OP_REMOVE_EVENT_LISTENER:
                self._op_remove_event_listener(reader)
            elif op == OP_REMOVE:
                self._op_remove(reader)
            elif op == OP_PUSH_ROOT:
                self._op_push_root(reader)
            elif op == OP_REGISTER_TEMPLATE:
                self._op_register_template(reader)
            elif op == OP_REMOVE_ATTRIBUTE:
                self._op_remove_attribute(reader)
            else:
                # Unknown opcode — skip remaining buffer to avoid corruption.
                # In debug builds, this could print a warning.
                print(
                    "MutationInterpreter: unknown opcode 0x"
                    + hex(Int(op))
                    + " at offset "
                    + String(reader.offset - 1)
                )
                break

    # ── Stack helpers ────────────────────────────────────────────────────

    def _push(mut self, node_id: UInt32):
        """Push a node ID onto the interpreter stack."""
        self._stack.append(node_id)

    def _pop(mut self) -> UInt32:
        """Pop a node ID from the interpreter stack.

        Returns 0 if the stack is empty (should not happen in valid
        mutation streams).
        """
        if len(self._stack) == 0:
            return 0
        return self._stack.pop()

    def _pop_n(mut self, n: Int) -> List[UInt32]:
        """Pop N node IDs from the stack (in LIFO order, reversed to
        give insertion order).

        Returns a list of node IDs in the order they should be inserted
        (first pushed = first in list).
        """
        var result = List[UInt32](capacity=n)
        var start = len(self._stack) - n
        if start < 0:
            start = 0
        # Collect from start to end (preserving push order)
        for i in range(start, len(self._stack)):
            result.append(self._stack[i])
        # Remove from stack
        while len(self._stack) > start:
            _ = self._stack.pop()
        return result^

    # ── Opcode handlers ──────────────────────────────────────────────────

    def _op_append_children(mut self, mut reader: BufReader):
        """OP_APPEND_CHILDREN: Pop m nodes, append as children of id.

        Wire: | id (u32) | m (u32) |
        """
        var id = reader.read_u32_le()
        var m = reader.read_u32_le()

        var children = self._pop_n(Int(m))
        # Use the individual append_children call for each child via
        # the Blitz stack operations. We pass children directly.
        if len(children) > 0:
            # Allocate a temporary buffer for the child ID array
            var child_buf = alloc[UInt32](len(children))
            for i in range(len(children)):
                child_buf[i] = children[i]
            self._blitz[].append_children(id, child_buf, UInt32(len(children)))
            child_buf.free()

    def _op_assign_id(mut self, mut reader: BufReader):
        """OP_ASSIGN_ID: Assign element ID to node at path in current template.

        Wire: | path_len (u8) | path ([u8]) | id (u32) |

        The path navigates from the top of the stack (the last loaded
        template root) to the target node within the template.
        """
        var path_len = reader.read_path_len()
        var path_ptr = reader.read_path_bytes(path_len)
        var mojo_id = reader.read_u32_le()

        # The top of the stack should be the template root
        if len(self._stack) == 0:
            return

        var root_id = self._stack[len(self._stack) - 1]

        # Navigate the path to find the target node
        var target_id: UInt32 = 0
        if path_len == 0:
            target_id = root_id
        else:
            target_id = self._blitz[].node_at_path(
                root_id, path_ptr, UInt32(path_len)
            )

        if target_id != 0:
            self._blitz[].assign_id(mojo_id, target_id)

    def _op_create_placeholder(mut self, mut reader: BufReader):
        """OP_CREATE_PLACEHOLDER: Create a placeholder and push to stack.

        Wire: | id (u32) |
        """
        var mojo_id = reader.read_u32_le()

        var blitz_id = self._blitz[].create_placeholder()
        self._blitz[].assign_id(mojo_id, blitz_id)
        self._push(blitz_id)

    def _op_create_text_node(mut self, mut reader: BufReader):
        """OP_CREATE_TEXT_NODE: Create a text node and push to stack.

        Wire: | id (u32) | len (u32) | text ([u8]) |
        """
        var mojo_id = reader.read_u32_le()
        var text = reader.read_str()

        var blitz_id = self._blitz[].create_text_node(text)
        self._blitz[].assign_id(mojo_id, blitz_id)
        self._push(blitz_id)

    def _op_load_template(mut self, mut reader: BufReader):
        """OP_LOAD_TEMPLATE: Clone template, assign id, push to stack.

        Wire: | tmpl_id (u32) | index (u32) | id (u32) |

        `index` selects which root of a multi-root template to clone.
        For single-root templates (the common case), index is always 0.
        """
        var tmpl_id = reader.read_u32_le()
        var index = reader.read_u32_le()
        var mojo_id = reader.read_u32_le()

        var blitz_id = self._blitz[].clone_template(tmpl_id)
        if blitz_id != 0:
            self._blitz[].assign_id(mojo_id, blitz_id)
            self._push(blitz_id)

    def _op_replace_with(mut self, mut reader: BufReader):
        """OP_REPLACE_WITH: Replace node id with m stack nodes.

        Wire: | id (u32) | m (u32) |
        """
        var id = reader.read_u32_le()
        var m = reader.read_u32_le()

        var replacements = self._pop_n(Int(m))
        if len(replacements) > 0:
            var replace_buf = alloc[UInt32](len(replacements))
            for i in range(len(replacements)):
                replace_buf[i] = replacements[i]
            self._blitz[].replace_with(
                id, replace_buf, UInt32(len(replacements))
            )
            replace_buf.free()

    def _op_replace_placeholder(mut self, mut reader: BufReader):
        """OP_REPLACE_PLACEHOLDER: Replace placeholder at path with m stack nodes.

        Wire: | path_len (u8) | path ([u8]) | m (u32) |

        The path navigates from the template root (top of stack after the
        m replacement nodes) to the placeholder node.
        """
        var path_len = reader.read_path_len()
        var path_ptr = reader.read_path_bytes(path_len)
        var m = reader.read_u32_le()

        var replacements = self._pop_n(Int(m))

        # The node to replace is at path from the current template root.
        # After popping m nodes, the template root should be on top of stack.
        if len(self._stack) == 0:
            return

        var root_id = self._stack[len(self._stack) - 1]
        var target_id: UInt32 = 0
        if path_len == 0:
            target_id = root_id
        else:
            target_id = self._blitz[].node_at_path(
                root_id, path_ptr, UInt32(path_len)
            )

        if target_id != 0 and len(replacements) > 0:
            var replace_buf = alloc[UInt32](len(replacements))
            for i in range(len(replacements)):
                replace_buf[i] = replacements[i]
            self._blitz[].replace_with(
                target_id, replace_buf, UInt32(len(replacements))
            )
            replace_buf.free()

    def _op_insert_after(mut self, mut reader: BufReader):
        """OP_INSERT_AFTER: Insert m stack nodes after node id.

        Wire: | id (u32) | m (u32) |
        """
        var id = reader.read_u32_le()
        var m = reader.read_u32_le()

        var nodes = self._pop_n(Int(m))
        if len(nodes) > 0:
            var node_buf = alloc[UInt32](len(nodes))
            for i in range(len(nodes)):
                node_buf[i] = nodes[i]
            self._blitz[].insert_after(id, node_buf, UInt32(len(nodes)))
            node_buf.free()

    def _op_insert_before(mut self, mut reader: BufReader):
        """OP_INSERT_BEFORE: Insert m stack nodes before node id.

        Wire: | id (u32) | m (u32) |
        """
        var id = reader.read_u32_le()
        var m = reader.read_u32_le()

        var nodes = self._pop_n(Int(m))
        if len(nodes) > 0:
            var node_buf = alloc[UInt32](len(nodes))
            for i in range(len(nodes)):
                node_buf[i] = nodes[i]
            self._blitz[].insert_before(id, node_buf, UInt32(len(nodes)))
            node_buf.free()

    def _op_set_attribute(mut self, mut reader: BufReader):
        """OP_SET_ATTRIBUTE: Set an attribute on element id.

        Wire: | id (u32) | ns (u8) | name_len (u16) | name | val_len (u32) | val |

        The ns byte is a namespace tag (0 = no namespace). Currently we
        ignore namespaces and pass the attribute name directly to Blitz.
        """
        var id = reader.read_u32_le()
        var ns = reader.read_u8()  # namespace tag (ignored for now)
        var name = reader.read_short_str()
        var value = reader.read_str()

        self._blitz[].set_attribute(id, name, value)

    def _op_set_text(mut self, mut reader: BufReader):
        """OP_SET_TEXT: Update the text content of node id.

        Wire: | id (u32) | len (u32) | text ([u8]) |
        """
        var id = reader.read_u32_le()
        var text = reader.read_str()

        self._blitz[].set_text_content(id, text)

    def _op_new_event_listener(mut self, mut reader: BufReader):
        """OP_NEW_EVENT_LISTENER: Attach an event listener to element id.

        Wire: | id (u32) | handler_id (u32) | name_len (u16) | name ([u8]) |
        """
        var id = reader.read_u32_le()
        var handler_id = reader.read_u32_le()
        var event_name = reader.read_short_str()

        self._blitz[].add_event_listener(id, handler_id, event_name)

    def _op_remove_event_listener(mut self, mut reader: BufReader):
        """OP_REMOVE_EVENT_LISTENER: Remove an event listener from element id.

        Wire: | id (u32) | name_len (u16) | name ([u8]) |
        """
        var id = reader.read_u32_le()
        var event_name = reader.read_short_str()

        self._blitz[].remove_event_listener(id, event_name)

    def _op_remove(mut self, mut reader: BufReader):
        """OP_REMOVE: Remove node id from the DOM.

        Wire: | id (u32) |
        """
        var id = reader.read_u32_le()
        self._blitz[].remove_node(id)

    def _op_push_root(mut self, mut reader: BufReader):
        """OP_PUSH_ROOT: Push node id onto the interpreter stack.

        Wire: | id (u32) |
        """
        var id = reader.read_u32_le()
        self._push(id)

    def _op_register_template(mut self, mut reader: BufReader):
        """OP_REGISTER_TEMPLATE: Build and register a template from its definition.

        Wire format:
          | tmpl_id (u32)
          | name_len (u16) | name ([u8])
          | root_count (u16)
          | node_count (u16)
          | attr_count (u16)
          | [nodes × node_count]
          | [attrs × attr_count]
          | [root_indices × root_count as u16]

        Node wire format (kind-tagged):
          Element:     | 0x00 | tag (u8) | child_count (u16) | [child_indices as u16…] | attr_first (u16) | attr_count (u16) |
          Text:        | 0x01 | text_len (u32) | text ([u8]) |
          Dynamic:     | 0x02 | dynamic_index (u32) |
          DynamicText: | 0x03 | dynamic_index (u32) |

        Attribute wire format:
          Static:  | 0x00 | name_len (u16) | name | value_len (u32) | value |
          Dynamic: | 0x01 | dynamic_index (u32) |

        This is the most complex opcode. We build the template's static
        structure as real Blitz DOM nodes (detached from the document tree),
        then register the root for efficient deep-cloning.

        Dynamic nodes/attributes/text are placeholders — they will be
        filled in by ASSIGN_ID + SET_TEXT / SET_ATTRIBUTE after cloning.
        """
        var tmpl_id = reader.read_u32_le()
        var tmpl_name = reader.read_short_str()
        var root_count = Int(reader.read_u16_le())
        var node_count = Int(reader.read_u16_le())
        var attr_count = Int(reader.read_u16_le())

        # ── Phase 1: Read all nodes into a flat array ────────────────────
        # We build the template bottom-up: first create all nodes, then
        # wire up parent-child relationships.

        # Store the Blitz node IDs created for each template node index.
        var node_ids = List[UInt32](capacity=node_count)

        # Store child indices and attribute ranges for element nodes.
        # We need to defer child attachment until all nodes are created.
        var element_children = List[List[Int]](capacity=node_count)
        var element_attr_first = List[Int](capacity=node_count)
        var element_attr_count = List[Int](capacity=node_count)

        for _i in range(node_count):
            var kind = reader.read_u8()

            if kind == TNODE_ELEMENT:
                var html_tag = reader.read_u8()
                var child_count = Int(reader.read_u16_le())
                var children = List[Int](capacity=child_count)
                for _c in range(child_count):
                    children.append(Int(reader.read_u16_le()))
                var af = Int(reader.read_u16_le())
                var ac = Int(reader.read_u16_le())

                # Create the element in Blitz
                var tag_str = tag_name(html_tag)
                var blitz_id = self._blitz[].create_element(tag_str)
                node_ids.append(blitz_id)
                element_children.append(children^)
                element_attr_first.append(af)
                element_attr_count.append(ac)

            elif kind == TNODE_TEXT:
                var text = reader.read_str()
                var blitz_id = self._blitz[].create_text_node(text)
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

            elif kind == TNODE_DYNAMIC:
                var _dynamic_index = reader.read_u32_le()
                # Dynamic nodes become placeholders in the template.
                # They will be replaced after cloning via ASSIGN_ID + PUSH_ROOT etc.
                var blitz_id = self._blitz[].create_placeholder()
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

            elif kind == TNODE_DYNAMIC_TEXT:
                var _dynamic_index = reader.read_u32_le()
                # Dynamic text nodes become empty text nodes in the template.
                # Their content will be set after cloning via SET_TEXT.
                var blitz_id = self._blitz[].create_text_node(String(""))
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

            else:
                # Unknown node kind — create a placeholder as fallback
                var blitz_id = self._blitz[].create_placeholder()
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

        # ── Phase 2: Read all attributes ─────────────────────────────────

        var attr_names = List[String](capacity=attr_count)
        var attr_values = List[String](capacity=attr_count)
        var attr_kinds = List[UInt8](capacity=attr_count)

        for _a in range(attr_count):
            var attr_kind = reader.read_u8()
            attr_kinds.append(attr_kind)

            if attr_kind == TATTR_STATIC:
                var name = reader.read_short_str()
                var value = reader.read_str()
                attr_names.append(name)
                attr_values.append(value)
            elif attr_kind == TATTR_DYNAMIC:
                var _dynamic_index = reader.read_u32_le()
                # Dynamic attributes are placeholders — they will be set
                # after cloning via SET_ATTRIBUTE with a specific element ID.
                attr_names.append(String(""))
                attr_values.append(String(""))
            else:
                attr_names.append(String(""))
                attr_values.append(String(""))

        # ── Phase 3: Read root indices ───────────────────────────────────

        var root_indices = List[Int](capacity=root_count)
        for _r in range(root_count):
            root_indices.append(Int(reader.read_u16_le()))

        # ── Phase 4: Apply static attributes to element nodes ────────────

        for i in range(node_count):
            var af = element_attr_first[i]
            var ac = element_attr_count[i]
            if ac > 0:
                var el_id = node_ids[i]
                for a in range(af, af + ac):
                    if a < attr_count and attr_kinds[a] == TATTR_STATIC:
                        if (attr_names[a]).byte_length() > 0:
                            self._blitz[].set_attribute(
                                el_id, attr_names[a], attr_values[a]
                            )

        # ── Phase 5: Wire up parent-child relationships ──────────────────

        for i in range(node_count):
            var children = element_children[i].copy()
            if len(children) > 0:
                var parent_id = node_ids[i]
                var child_buf = alloc[UInt32](len(children))
                for c in range(len(children)):
                    var child_idx = children[c]
                    if child_idx < len(node_ids):
                        child_buf[c] = node_ids[child_idx]
                    else:
                        child_buf[c] = 0
                self._blitz[].append_children(
                    parent_id, child_buf, UInt32(len(children))
                )
                child_buf.free()

        # ── Phase 6: Register the template root(s) ───────────────────────
        #
        # For single-root templates (the common case), we register the
        # root node's Blitz ID. For multi-root templates, we'd need a
        # container — for now, we only support single-root.

        if root_count > 0 and len(root_indices) > 0:
            var root_idx = root_indices[0]
            if root_idx < len(node_ids):
                self._blitz[].register_template(tmpl_id, node_ids[root_idx])

    def _op_remove_attribute(mut self, mut reader: BufReader):
        """OP_REMOVE_ATTRIBUTE: Remove an attribute from element id.

        Wire: | id (u32) | ns (u8) | name_len (u16) | name ([u8]) |
        """
        var id = reader.read_u32_le()
        var ns = reader.read_u8()  # namespace tag (ignored for now)
        var name = reader.read_short_str()

        self._blitz[].remove_attribute(id, name)
