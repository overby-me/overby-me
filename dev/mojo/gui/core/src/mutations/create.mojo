# CreateEngine — Initial render (mount) mutation emitter.
#
# The CreateEngine walks a VNode tree and emits stack-based DOM mutations
# that instruct the JS interpreter to build the corresponding DOM nodes.
# This is the "no previous tree" path — the first time a component is
# rendered, all its VNodes are created from scratch.
#
# The engine populates each VNode's mount state (root_ids, dyn_node_ids,
# dyn_attr_ids) so the diff engine can later target mutations to the
# correct DOM elements.
#
# Mutation sequence for each VNode kind:
#
#   TemplateRef:
#     1. LoadTemplate for each template root → assigns ElementIds
#     2. For each dynamic attribute:
#        - AssignId to the target element (path from root)
#        - SetAttribute / NewEventListener
#     3. For each dynamic node:
#        - Create the dynamic content (CreateTextNode / CreatePlaceholder)
#        - ReplacePlaceholder at the template path
#     Result: template root(s) on the stack
#
#   Text:
#     1. CreateTextNode with a new ElementId
#     Result: text node on the stack
#
#   Placeholder:
#     1. CreatePlaceholder with the VNode's element_id
#     Result: placeholder on the stack
#
#   Fragment:
#     1. Recursively create each child
#     Result: all children's roots on the stack

from memory import UnsafePointer
from bridge import MutationWriter
from arena import ElementId, ElementIdAllocator
from signals import Runtime
from vdom import (
    Template,
    TemplateNode,
    TemplateRegistry,
    VNode,
    VNodeStore,
    DynamicNode,
    DynamicAttr,
    AttributeValue,
    TNODE_ELEMENT,
    TNODE_TEXT,
    TNODE_DYNAMIC,
    TNODE_DYNAMIC_TEXT,
    TATTR_STATIC,
    TATTR_DYNAMIC,
    VNODE_TEMPLATE_REF,
    VNODE_TEXT,
    VNODE_PLACEHOLDER,
    VNODE_FRAGMENT,
    AVAL_TEXT,
    AVAL_INT,
    AVAL_FLOAT,
    AVAL_BOOL,
    AVAL_EVENT,
    AVAL_NONE,
    DNODE_TEXT,
    DNODE_PLACEHOLDER,
)


# ── Path computation helpers ─────────────────────────────────────────────────
#
# Templates use a flat arena with index-based parent-child relationships.
# To emit AssignId and ReplacePlaceholder mutations, we need the "path"
# from a template root to a descendant node.  The path is a sequence of
# child indices: e.g. [1, 0] means "take child 1 of root, then child 0
# of that node".
#
# We compute paths by building a parent map, then walking from the target
# up to the root and reversing.


fn _build_parent_map(
    tmpl_ptr: UnsafePointer[Template, MutExternalOrigin]
) -> List[Int]:
    """Build parent[i] = parent node index of node i, or -1 for roots."""
    var n = tmpl_ptr[0].node_count()
    var parents = List[Int](capacity=n)
    for _ in range(n):
        parents.append(-1)
    for i in range(n):
        var node_ptr = tmpl_ptr[0].get_node_ptr(i)
        for j in range(node_ptr[0].child_count()):
            var child = Int(node_ptr[0].child_at(j))
            parents[child] = i
    return parents^


fn _path_from_root(
    tmpl_ptr: UnsafePointer[Template, MutExternalOrigin],
    parents: List[Int],
    target: Int,
) -> List[UInt8]:
    """Compute the child-index path from the target's root to target.

    Walks up the parent chain collecting child indices, then reverses.
    """
    var rev_path = List[UInt8]()
    var current = target
    while parents[current] != -1:
        var parent_idx = parents[current]
        var parent_node_ptr = tmpl_ptr[0].get_node_ptr(parent_idx)
        # Find which child slot `current` occupies under parent
        for i in range(parent_node_ptr[0].child_count()):
            if Int(parent_node_ptr[0].child_at(i)) == current:
                rev_path.append(UInt8(i))
                break
        current = parent_idx
    # Reverse
    var path = List[UInt8](capacity=len(rev_path))
    for i in range(len(rev_path) - 1, -1, -1):
        path.append(rev_path[i])
    return path^


fn _find_root_for_node(parents: List[Int], node_index: Int) -> Int:
    """Find the root node index that `node_index` is a descendant of."""
    var current = node_index
    while parents[current] != -1:
        current = parents[current]
    return current


# ── Attribute value to string conversion ─────────────────────────────────────


fn _attr_value_to_string(value: AttributeValue) -> String:
    """Convert an AttributeValue to its string representation for SetAttribute.
    """
    if value.kind == AVAL_TEXT:
        return value.text_value
    elif value.kind == AVAL_INT:
        return String(value.int_value)
    elif value.kind == AVAL_FLOAT:
        return String(value.float_value)
    elif value.kind == AVAL_BOOL:
        if value.bool_value:
            return String("true")
        else:
            return String("false")
    elif value.kind == AVAL_NONE:
        return String("")
    else:
        # AVAL_EVENT — handled separately
        return String("")


# ── CreateEngine ─────────────────────────────────────────────────────────────


struct CreateEngine:
    """Emits mutations to create DOM nodes from a VNode tree.

    The engine holds references to the shared resources needed during
    creation: the mutation writer, element ID allocator, runtime (for
    template registry access), and VNode store.

    After creating a VNode, its mount state (root_ids, dyn_node_ids,
    dyn_attr_ids) is populated so the diff engine can target those
    elements later.

    Usage:
        var engine = CreateEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
        var num_roots = engine.create_node(vnode_index)
        # The VNode at vnode_index now has mount state populated
        # The writer contains the mutations to create the DOM
    """

    var writer: UnsafePointer[MutationWriter, MutExternalOrigin]
    var eid_alloc: UnsafePointer[ElementIdAllocator, MutExternalOrigin]
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]
    var store: UnsafePointer[VNodeStore, MutExternalOrigin]

    fn __init__(
        out self,
        writer: UnsafePointer[MutationWriter, MutExternalOrigin],
        eid_alloc: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
        store: UnsafePointer[VNodeStore, MutExternalOrigin],
    ):
        self.writer = writer
        self.eid_alloc = eid_alloc
        self.runtime = runtime
        self.store = store

    fn create_node(mut self, vnode_index: UInt32) -> UInt32:
        """Create mutations for the VNode at `vnode_index`.

        Emits the mutations and populates the VNode's mount state.
        Returns the number of root-level nodes placed on the stack.
        """
        var node_ptr = self.store[0].get_ptr(vnode_index)
        var kind = node_ptr[0].kind

        if kind == VNODE_TEMPLATE_REF:
            return self._create_template_ref(vnode_index)
        elif kind == VNODE_TEXT:
            return self._create_text(vnode_index)
        elif kind == VNODE_PLACEHOLDER:
            return self._create_placeholder(vnode_index)
        elif kind == VNODE_FRAGMENT:
            return self._create_fragment(vnode_index)
        else:
            return 0

    fn _create_template_ref(mut self, vnode_index: UInt32) -> UInt32:
        """Create a TemplateRef VNode.  Returns number of roots (on stack)."""
        var node_ptr = self.store[0].get_ptr(vnode_index)
        var tmpl_id = node_ptr[0].template_id
        var tmpl_ptr = self.runtime[0].templates.get_ptr(tmpl_id)

        var root_count = tmpl_ptr[0].root_count()

        # Build parent map for path computation
        var parents = _build_parent_map(tmpl_ptr)

        # 1. LoadTemplate for each root — assigns ElementIds, pushes to stack
        for i in range(root_count):
            var _ = tmpl_ptr[0].get_root_index(i)
            var eid = self.eid_alloc[0].alloc()
            self.writer[0].load_template(tmpl_id, UInt32(i), eid.as_u32())
            # Store root ElementId on the VNode
            # Re-read pointer in case store was mutated
            self.store[0].get_ptr(vnode_index)[0].push_root_id(eid.as_u32())

        # 2. Process dynamic attributes
        #    Walk all nodes in the template looking for dynamic attributes.
        #    For each, compute path to the owning element and assign an ID.
        #
        #    IMPORTANT: dyn_attr_ids must be indexed by dynamic_index (dyn_idx)
        #    so that dyn_attr_ids[i] corresponds to dynamic_attrs[i].
        #    Template traversal order may differ from dynamic_index order,
        #    so we pre-allocate the array and assign by dyn_idx.
        var num_dyn_attrs = (
            self.store[0].get_ptr(vnode_index)[0].dynamic_attr_count()
        )
        # Pre-allocate dyn_attr_ids with zeros so we can index by dyn_idx
        for _ in range(num_dyn_attrs):
            self.store[0].get_ptr(vnode_index)[0].push_dyn_attr_id(0)

        # Build a mapping: dynamic_attr_index → (node_index in template)
        # by scanning template nodes for dynamic attrs
        for node_i in range(tmpl_ptr[0].node_count()):
            var tnode_ptr = tmpl_ptr[0].get_node_ptr(node_i)
            if tnode_ptr[0].kind != TNODE_ELEMENT:
                continue
            var first_attr = Int(tnode_ptr[0].first_attr)
            var n_attrs = tnode_ptr[0].attr_count()
            for attr_j in range(n_attrs):
                var attr_idx = first_attr + attr_j
                var attr = tmpl_ptr[0].get_attr(attr_idx)
                if attr.kind == TATTR_DYNAMIC:
                    var dyn_idx = Int(attr.dynamic_index)
                    if dyn_idx < num_dyn_attrs:
                        # Compute path from root to this element
                        var path = _path_from_root(tmpl_ptr, parents, node_i)
                        # Assign an ElementId to this element
                        var elem_eid = self.eid_alloc[0].alloc()
                        var path_ptr = path.unsafe_ptr()
                        self.writer[0].assign_id(
                            path_ptr, len(path), elem_eid.as_u32()
                        )
                        # Store the element ID indexed by dyn_idx (not push order)
                        self.store[0].get_ptr(vnode_index)[0].dyn_attr_ids[
                            dyn_idx
                        ] = elem_eid.as_u32()
                        # Now emit the attribute mutation
                        var vnode_ptr2 = self.store[0].get_ptr(vnode_index)
                        var dyn_attr = (
                            vnode_ptr2[0].dynamic_attrs[dyn_idx].copy()
                        )
                        if dyn_attr.value.kind == AVAL_EVENT:
                            self.writer[0].new_event_listener(
                                elem_eid.as_u32(),
                                dyn_attr.value.handler_id,
                                dyn_attr.name,
                            )
                        elif dyn_attr.value.kind != AVAL_NONE:
                            # Determine namespace byte
                            var ns_byte: UInt8 = 0
                            if dyn_attr.has_namespace():
                                ns_byte = 1  # simplified: 1 = has namespace
                            var val_str = _attr_value_to_string(dyn_attr.value)
                            self.writer[0].set_attribute(
                                elem_eid.as_u32(),
                                ns_byte,
                                dyn_attr.name,
                                val_str,
                            )

        # 3. Process dynamic nodes
        #    Walk template nodes looking for Dynamic and DynamicText nodes.
        for node_i in range(tmpl_ptr[0].node_count()):
            var tnode_ptr = tmpl_ptr[0].get_node_ptr(node_i)
            if tnode_ptr[0].kind == TNODE_DYNAMIC:
                var dyn_idx = Int(tnode_ptr[0].dynamic_index)
                var vnode_ptr3 = self.store[0].get_ptr(vnode_index)
                if dyn_idx < len(vnode_ptr3[0].dynamic_nodes):
                    var dyn_node = vnode_ptr3[0].dynamic_nodes[dyn_idx].copy()
                    var path = _path_from_root(tmpl_ptr, parents, node_i)
                    if dyn_node.kind == DNODE_TEXT:
                        # Create a text node
                        var text_eid = self.eid_alloc[0].alloc()
                        self.writer[0].create_text_node(
                            text_eid.as_u32(), dyn_node.text
                        )
                        # Replace the placeholder in the template clone
                        var path_ptr = path.unsafe_ptr()
                        self.writer[0].replace_placeholder(
                            path_ptr, len(path), 1
                        )
                        self.store[0].get_ptr(vnode_index)[0].push_dyn_node_id(
                            text_eid.as_u32()
                        )
                    elif dyn_node.kind == DNODE_PLACEHOLDER:
                        # Create a placeholder node
                        var ph_eid = self.eid_alloc[0].alloc()
                        self.writer[0].create_placeholder(ph_eid.as_u32())
                        var path_ptr = path.unsafe_ptr()
                        self.writer[0].replace_placeholder(
                            path_ptr, len(path), 1
                        )
                        self.store[0].get_ptr(vnode_index)[0].push_dyn_node_id(
                            ph_eid.as_u32()
                        )

            elif tnode_ptr[0].kind == TNODE_DYNAMIC_TEXT:
                var dyn_idx = Int(tnode_ptr[0].dynamic_index)
                var vnode_ptr4 = self.store[0].get_ptr(vnode_index)
                if dyn_idx < len(vnode_ptr4[0].dynamic_nodes):
                    var dyn_node = vnode_ptr4[0].dynamic_nodes[dyn_idx].copy()
                    if dyn_node.kind == DNODE_TEXT:
                        # DynamicText is text that replaces inline —
                        # assign an ID so we can update it later via SetText
                        var path = _path_from_root(tmpl_ptr, parents, node_i)
                        var text_eid = self.eid_alloc[0].alloc()
                        # Assign the ID to the text node position in template
                        var path_ptr = path.unsafe_ptr()
                        self.writer[0].assign_id(
                            path_ptr, len(path), text_eid.as_u32()
                        )
                        # Set its text content
                        self.writer[0].set_text(
                            text_eid.as_u32(), dyn_node.text
                        )
                        self.store[0].get_ptr(vnode_index)[0].push_dyn_node_id(
                            text_eid.as_u32()
                        )

        return UInt32(root_count)

    fn _create_text(mut self, vnode_index: UInt32) -> UInt32:
        """Create a Text VNode.  Returns 1 (one root on stack)."""
        var node_ptr = self.store[0].get_ptr(vnode_index)
        var eid = self.eid_alloc[0].alloc()
        self.writer[0].create_text_node(eid.as_u32(), node_ptr[0].text)
        # Store the ElementId on the VNode
        node_ptr[0].element_id = eid.as_u32()
        node_ptr[0].push_root_id(eid.as_u32())
        return 1

    fn _create_placeholder(mut self, vnode_index: UInt32) -> UInt32:
        """Create a Placeholder VNode.  Returns 1 (one root on stack)."""
        var node_ptr = self.store[0].get_ptr(vnode_index)
        var eid: ElementId
        if node_ptr[0].element_id != 0:
            # Placeholder already has an assigned ID (e.g. pre-allocated)
            eid = ElementId(node_ptr[0].element_id)
        else:
            eid = self.eid_alloc[0].alloc()
            node_ptr[0].element_id = eid.as_u32()
        self.writer[0].create_placeholder(eid.as_u32())
        node_ptr[0].push_root_id(eid.as_u32())
        return 1

    fn _create_fragment(mut self, vnode_index: UInt32) -> UInt32:
        """Create a Fragment VNode.  Returns total roots from all children."""
        var node_ptr = self.store[0].get_ptr(vnode_index)
        var child_count = node_ptr[0].fragment_child_count()
        var total_roots: UInt32 = 0
        for i in range(child_count):
            var child_idx = (
                self.store[0].get_ptr(vnode_index)[0].get_fragment_child(i)
            )
            total_roots += self.create_node(child_idx)
        return total_roots
