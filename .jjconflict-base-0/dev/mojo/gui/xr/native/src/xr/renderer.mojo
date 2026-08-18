# XR Mutation Interpreter — Reads binary opcodes and translates to XR Blitz FFI calls.
#
# This module is the XR-panel equivalent of the desktop `MutationInterpreter`
# (desktop/src/desktop/renderer.mojo). It reads the binary mutation buffer
# produced by mojo-gui/core's MutationWriter and translates each opcode into
# the corresponding XR Blitz C FFI call via the `XRBlitz` wrapper.
#
# The key difference from the desktop interpreter is that every DOM operation
# is scoped to a specific panel_id, since the XR shim manages multiple
# independent Blitz documents — one per XR panel.
#
# Architecture:
#
#   MutationWriter (core)
#     │ writes binary opcodes to heap buffer (per-panel)
#     ▼
#   XRMutationInterpreter (this module)
#     │ reads opcodes, decodes payloads
#     │ calls XR Blitz FFI via XRBlitz wrapper (per-panel)
#     ▼
#   libmojo_xr (Rust cdylib)
#     │ manipulates per-panel blitz-dom
#     ▼
#   Blitz (Stylo + Taffy + Vello) → offscreen texture per panel
#
# The interpreter maintains a stack of Blitz node IDs per panel, mirroring
# the JS interpreter's node stack. Opcodes like PUSH_ROOT push nodes onto
# the stack; opcodes like APPEND_CHILDREN pop from it.
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
# Usage (from xr/native/src/xr/xr_launcher.mojo — future Step 5.5):
#
#   var interpreter = XRMutationInterpreter(xr_blitz, panel_id)
#   var mount_len = app.mount(writer_ptr)
#   if mount_len > 0:
#       xr_blitz.panel_begin_mutations(panel_id)
#       interpreter.apply(buf_ptr, Int(mount_len))
#       xr_blitz.panel_end_mutations(panel_id)

from std.memory import UnsafePointer, alloc
from .xr_blitz import XRBlitz, _event_type_from_name
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

    var buf: UnsafePointer[UInt8, MutAnyOrigin]
    var offset: Int
    var length: Int

    fn __init__(out self, buf: UnsafePointer[UInt8, MutAnyOrigin], length: Int):
        self.buf = buf
        self.offset = 0
        self.length = length

    fn has_remaining(self) -> Bool:
        """Check if there are more bytes to read."""
        return self.offset < self.length

    @always_inline
    fn read_u8(mut self) -> UInt8:
        """Read a single byte."""
        var val = self.buf[self.offset]
        self.offset += 1
        return val

    @always_inline
    fn read_u16_le(mut self) -> UInt16:
        """Read a 16-bit little-endian unsigned integer."""
        var lo = UInt16(self.buf[self.offset])
        var hi = UInt16(self.buf[self.offset + 1])
        self.offset += 2
        return lo | (hi << 8)

    @always_inline
    fn read_u32_le(mut self) -> UInt32:
        """Read a 32-bit little-endian unsigned integer."""
        var b0 = UInt32(self.buf[self.offset])
        var b1 = UInt32(self.buf[self.offset + 1])
        var b2 = UInt32(self.buf[self.offset + 2])
        var b3 = UInt32(self.buf[self.offset + 3])
        self.offset += 4
        return b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)

    fn read_str(mut self) -> String:
        """Read a u32-length-prefixed UTF-8 string."""
        var str_len = Int(self.read_u32_le())
        if str_len == 0:
            return String("")
        var result = String("")
        for i in range(str_len):
            var byte = self.buf[self.offset + i]
            result += chr(Int(byte))
        self.offset += str_len
        return result

    fn read_short_str(mut self) -> String:
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

    fn read_path_len(mut self) -> Int:
        """Read a u8 path length."""
        return Int(self.read_u8())

    fn read_path_bytes(
        mut self, path_len: Int
    ) -> UnsafePointer[UInt8, MutAnyOrigin]:
        """Read path_len bytes and return a pointer to the start.

        The pointer points directly into the buffer. The caller must not
        free it and must use it before the buffer is freed/overwritten.

        Returns:
            Pointer to the start of the path bytes in the buffer.
        """
        var ptr = self.buf + self.offset
        self.offset += path_len
        return ptr

    fn skip(mut self, count: Int):
        """Skip forward by count bytes."""
        self.offset += count


# ══════════════════════════════════════════════════════════════════════════════
# XRMutationInterpreter — Per-panel opcode interpreter for XR
# ══════════════════════════════════════════════════════════════════════════════


struct XRMutationInterpreter(Movable):
    """Interprets binary mutation opcodes and applies them to an XR panel's
    Blitz DOM.

    This is the XR-panel port of the desktop `MutationInterpreter`. It reads
    opcodes from a byte buffer and translates each one into XR Blitz C FFI
    calls via the `XRBlitz` wrapper, scoped to a specific panel_id.

    The interpreter maintains:
      - A stack of Blitz node IDs (for PUSH_ROOT / APPEND_CHILDREN / etc.)
      - A reference to the XRBlitz FFI wrapper
      - The panel_id targeting all DOM operations

    Lifetime: The interpreter borrows the XRBlitz instance. It must not
    outlive the XRBlitz session.
    """

    var _xr: UnsafePointer[XRBlitz, MutAnyOrigin]
    var _panel_id: UInt32
    var _stack: List[UInt32]

    fn __init__(out self, ref[MutAnyOrigin] xr: XRBlitz, panel_id: UInt32):
        """Create a mutation interpreter for a specific XR panel.

        Args:
            xr: Reference to the XRBlitz FFI wrapper. The interpreter
                borrows this; the caller must keep it alive.
            panel_id: Panel ID targeting all DOM operations.
        """
        self._xr = UnsafePointer(to=xr)
        self._panel_id = panel_id
        self._stack = List[UInt32](capacity=64)

    fn __moveinit__(out self, deinit take: Self):
        self._xr = take._xr
        self._panel_id = take._panel_id
        self._stack = take._stack^

    # ── Public API ───────────────────────────────────────────────────────

    fn apply(mut self, buf: UnsafePointer[UInt8, MutAnyOrigin], length: Int):
        """Apply all mutations in the given buffer to the panel's Blitz DOM.

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
                print(
                    "XRMutationInterpreter: unknown opcode 0x"
                    + hex(Int(op))
                    + " at offset "
                    + String(reader.offset - 1)
                    + " (panel "
                    + String(self._panel_id)
                    + ")"
                )
                break

    # ── Stack helpers ────────────────────────────────────────────────────

    fn _push(mut self, node_id: UInt32):
        """Push a node ID onto the interpreter stack."""
        self._stack.append(node_id)

    fn _pop(mut self) -> UInt32:
        """Pop a node ID from the interpreter stack.

        Returns 0 if the stack is empty (should not happen in valid
        mutation streams).
        """
        if len(self._stack) == 0:
            return 0
        return self._stack.pop()

    fn _pop_n(mut self, n: Int) -> List[UInt32]:
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

    fn _op_append_children(mut self, mut reader: BufReader):
        """OP_APPEND_CHILDREN: Pop m nodes, append as children of id.

        Wire: | id (u32) | m (u32) |
        """
        var id = reader.read_u32_le()
        var m = reader.read_u32_le()

        var children = self._pop_n(Int(m))
        if len(children) > 0:
            var child_buf = alloc[UInt32](len(children))
            for i in range(len(children)):
                child_buf[i] = children[i]
            self._xr[].panel_append_children(
                self._panel_id, id, child_buf, UInt32(len(children))
            )
            child_buf.free()

    fn _op_assign_id(mut self, mut reader: BufReader):
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
            # Convert u8 path bytes to u32 indices for the XR shim API.
            var path_u32 = alloc[UInt32](path_len)
            for i in range(path_len):
                path_u32[i] = UInt32(path_ptr[i])
            target_id = self._xr[].panel_node_at_path(
                self._panel_id, root_id, path_u32, UInt32(path_len)
            )
            path_u32.free()

        if target_id != 0:
            self._xr[].panel_assign_id(self._panel_id, mojo_id, target_id)

    fn _op_create_placeholder(mut self, mut reader: BufReader):
        """OP_CREATE_PLACEHOLDER: Create a placeholder and push to stack.

        Wire: | id (u32) |
        """
        var mojo_id = reader.read_u32_le()

        var blitz_id = self._xr[].panel_create_placeholder(self._panel_id)
        self._xr[].panel_assign_id(self._panel_id, mojo_id, blitz_id)
        self._push(blitz_id)

    fn _op_create_text_node(mut self, mut reader: BufReader):
        """OP_CREATE_TEXT_NODE: Create a text node and push to stack.

        Wire: | id (u32) | len (u32) | text ([u8]) |
        """
        var mojo_id = reader.read_u32_le()
        var text = reader.read_str()

        var blitz_id = self._xr[].panel_create_text_node(self._panel_id, text)
        self._xr[].panel_assign_id(self._panel_id, mojo_id, blitz_id)
        self._push(blitz_id)

    fn _op_load_template(mut self, mut reader: BufReader):
        """OP_LOAD_TEMPLATE: Clone template, assign id, push to stack.

        Wire: | tmpl_id (u32) | index (u32) | id (u32) |

        `index` selects which root of a multi-root template to clone.
        For single-root templates (the common case), index is always 0.
        """
        var tmpl_id = reader.read_u32_le()
        var index = reader.read_u32_le()
        var mojo_id = reader.read_u32_le()

        var blitz_id = self._xr[].panel_clone_template(self._panel_id, tmpl_id)
        if blitz_id != 0:
            self._xr[].panel_assign_id(self._panel_id, mojo_id, blitz_id)
            self._push(blitz_id)

    fn _op_replace_with(mut self, mut reader: BufReader):
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
            self._xr[].panel_replace_with(
                self._panel_id, id, replace_buf, UInt32(len(replacements))
            )
            replace_buf.free()

    fn _op_replace_placeholder(mut self, mut reader: BufReader):
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
            var path_u32 = alloc[UInt32](path_len)
            for i in range(path_len):
                path_u32[i] = UInt32(path_ptr[i])
            target_id = self._xr[].panel_node_at_path(
                self._panel_id, root_id, path_u32, UInt32(path_len)
            )
            path_u32.free()

        if target_id != 0 and len(replacements) > 0:
            var replace_buf = alloc[UInt32](len(replacements))
            for i in range(len(replacements)):
                replace_buf[i] = replacements[i]
            self._xr[].panel_replace_with(
                self._panel_id,
                target_id,
                replace_buf,
                UInt32(len(replacements)),
            )
            replace_buf.free()

    fn _op_insert_after(mut self, mut reader: BufReader):
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
            self._xr[].panel_insert_after(
                self._panel_id, id, node_buf, UInt32(len(nodes))
            )
            node_buf.free()

    fn _op_insert_before(mut self, mut reader: BufReader):
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
            self._xr[].panel_insert_before(
                self._panel_id, id, node_buf, UInt32(len(nodes))
            )
            node_buf.free()

    fn _op_set_attribute(mut self, mut reader: BufReader):
        """OP_SET_ATTRIBUTE: Set an attribute on element id.

        Wire: | id (u32) | ns (u8) | name_len (u16) | name | val_len (u32) | val |

        The ns byte is a namespace tag (0 = no namespace). Currently we
        ignore namespaces and pass the attribute name directly to Blitz.
        """
        var id = reader.read_u32_le()
        var ns = reader.read_u8()  # namespace tag (ignored for now)
        var name = reader.read_short_str()
        var value = reader.read_str()

        self._xr[].panel_set_attribute(self._panel_id, id, name, value)

    fn _op_set_text(mut self, mut reader: BufReader):
        """OP_SET_TEXT: Update the text content of node id.

        Wire: | id (u32) | len (u32) | text ([u8]) |
        """
        var id = reader.read_u32_le()
        var text = reader.read_str()

        self._xr[].panel_set_text_content(self._panel_id, id, text)

    fn _op_new_event_listener(mut self, mut reader: BufReader):
        """OP_NEW_EVENT_LISTENER: Attach an event listener to element id.

        Wire: | id (u32) | handler_id (u32) | name_len (u16) | name ([u8]) |
        """
        var id = reader.read_u32_le()
        var handler_id = reader.read_u32_le()
        var event_name = reader.read_short_str()

        self._xr[].panel_add_event_listener_by_name(
            self._panel_id, id, handler_id, event_name
        )

    fn _op_remove_event_listener(mut self, mut reader: BufReader):
        """OP_REMOVE_EVENT_LISTENER: Remove an event listener from element id.

        Wire: | id (u32) | name_len (u16) | name ([u8]) |

        Note: The mutation protocol sends the event name and the node ID,
        but the XR shim's remove_event_listener also requires handler_id.
        We pass handler_id=0 as a sentinel — the shim matches on
        (node_id, event_type) for removal, ignoring handler_id.
        """
        var id = reader.read_u32_le()
        var event_name = reader.read_short_str()

        self._xr[].panel_remove_event_listener_by_name(
            self._panel_id, id, UInt32(0), event_name
        )

    fn _op_remove(mut self, mut reader: BufReader):
        """OP_REMOVE: Remove node id from the DOM.

        Wire: | id (u32) |
        """
        var id = reader.read_u32_le()
        self._xr[].panel_remove_node(self._panel_id, id)

    fn _op_push_root(mut self, mut reader: BufReader):
        """OP_PUSH_ROOT: Push node id onto the interpreter stack.

        Wire: | id (u32) |
        """
        var id = reader.read_u32_le()
        self._push(id)

    fn _op_register_template(mut self, mut reader: BufReader):
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

        var node_ids = List[UInt32](capacity=node_count)
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

                var tag_str = tag_name(html_tag)
                var blitz_id = self._xr[].panel_create_element(
                    self._panel_id, tag_str
                )
                node_ids.append(blitz_id)
                element_children.append(children^)
                element_attr_first.append(af)
                element_attr_count.append(ac)

            elif kind == TNODE_TEXT:
                var text = reader.read_str()
                var blitz_id = self._xr[].panel_create_text_node(
                    self._panel_id, text
                )
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

            elif kind == TNODE_DYNAMIC:
                var _dynamic_index = reader.read_u32_le()
                var blitz_id = self._xr[].panel_create_placeholder(
                    self._panel_id
                )
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

            elif kind == TNODE_DYNAMIC_TEXT:
                var _dynamic_index = reader.read_u32_le()
                var blitz_id = self._xr[].panel_create_text_node(
                    self._panel_id, String("")
                )
                node_ids.append(blitz_id)
                element_children.append(List[Int]())
                element_attr_first.append(0)
                element_attr_count.append(0)

            else:
                # Unknown node kind — create a placeholder as fallback
                var blitz_id = self._xr[].panel_create_placeholder(
                    self._panel_id
                )
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
                        if len(attr_names[a]) > 0:
                            self._xr[].panel_set_attribute(
                                self._panel_id,
                                el_id,
                                attr_names[a],
                                attr_values[a],
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
                self._xr[].panel_append_children(
                    self._panel_id,
                    parent_id,
                    child_buf,
                    UInt32(len(children)),
                )
                child_buf.free()

        # ── Phase 6: Register the template root(s) ───────────────────────
        #
        # For single-root templates (the common case), we register the
        # root node's Blitz ID. For multi-root templates, we'd need a
        # container — for now, we only support single-root.

        # The XR shim's panel_register_template expects a serialized
        # template buffer, but we've already built the DOM nodes above.
        # We use the same approach as the desktop interpreter: build the
        # template as real Blitz nodes, then register via the template
        # clone infrastructure.
        #
        # Since the shim's mxr_panel_register_template expects a raw
        # buffer (for the Rust-side interpreter path), and we're using
        # the Mojo-side interpreter path (building nodes via individual
        # FFI calls), we need to register the built root node as a
        # template. The shim should have a register-by-node-id function.
        #
        # For now, we use the same approach as the desktop interpreter:
        # store the template root ID internally and handle cloning on the
        # Mojo side. The template registration via raw buffer is for the
        # Rust-side interpreter only.
        #
        # TODO: Add mxr_panel_register_template_node() to the shim for
        # Mojo-side template registration by node ID (matching desktop's
        # mblitz_register_template pattern). For now, templates built
        # via Mojo-side interpretation work through the node_at_path +
        # clone machinery.

        # For the Mojo-side interpreter, we skip the template registration
        # since we've already built the template tree as live DOM nodes.
        # The LOAD_TEMPLATE opcode will clone from the shim's template
        # store, which is populated by the Rust-side interpreter or by
        # explicit registration. When using the Mojo-side interpreter,
        # templates are built in-place and don't need the clone path.
        #
        # However, to support LOAD_TEMPLATE (clone-based instantiation),
        # we need to register. Let's use the raw buffer approach: pass
        # the original template bytes to the shim for Rust-side parsing.
        #
        # Actually, the cleanest approach is to track the reader position
        # at the start of this opcode and pass the entire opcode payload
        # to mxr_panel_register_template. But we've already consumed the
        # bytes. For now, we note this as a known limitation.
        #
        # Workaround: The Mojo-side interpreter builds templates as live
        # nodes. When LOAD_TEMPLATE is encountered, it clones from the
        # shim's template store. For this to work, templates must be
        # registered on the shim side. Since we can't easily re-serialize,
        # we use the live root node approach: the template IS the built
        # subtree, and cloning deep-clones from it.
        #
        # This matches how the desktop shim works: register_template(id,
        # root_node_id) stores the root for deep cloning.

        # We need a way to register by root node ID. The XR shim has
        # mxr_panel_register_template which takes a raw buffer. We'd
        # need an additional FFI function. For now, skip registration
        # and document the limitation — templates work when the Rust-side
        # interpreter is used (via panel_apply_mutations), not when using
        # the Mojo-side interpreter for template-heavy apps.
        #
        # TODO(Step 5.5): Add mxr_panel_register_template_by_node() to
        # allow Mojo-side interpreter to register templates by root node ID.
        pass

    fn _op_remove_attribute(mut self, mut reader: BufReader):
        """OP_REMOVE_ATTRIBUTE: Remove an attribute from element id.

        Wire: | id (u32) | ns (u8) | name_len (u16) | name ([u8]) |
        """
        var id = reader.read_u32_le()
        var ns = reader.read_u8()  # namespace tag (ignored for now)
        var name = reader.read_short_str()

        self._xr[].panel_remove_attribute(self._panel_id, id, name)
