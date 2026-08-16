# TemplateBuilder — Tier 1 runtime template construction.
#
# The builder provides a step-by-step API for constructing templates at
# runtime.  Nodes are added one at a time, with parent-child relationships
# established via index references.  Once complete, the builder produces
# a Template that can be registered in the TemplateRegistry.
#
# This is the Tier 1 approach: fully dynamic, simple, correct.  Tier 2
# will use `comptime` constants for zero-runtime-cost templates.
#
# Usage:
#     var b = TemplateBuilder("counter")
#     var div_idx = b.push_element(TAG_DIV, -1)       # root element
#     var h1_idx = b.push_element(TAG_H1, div_idx)    # child of div
#     var txt_idx = b.push_text("Count: ", h1_idx)    # child of h1
#     var dyn_idx = b.push_dynamic_text(0, h1_idx)    # dynamic text slot
#     var btn_idx = b.push_element(TAG_BUTTON, div_idx)
#     b.push_text("Increment", btn_idx)
#     b.push_static_attr(btn_idx, "class", "btn")
#     b.push_dynamic_attr(btn_idx, 0)                 # dynamic onclick
#     var template = b.build()
#
# The builder can also be used from WASM exports, where it is heap-allocated
# and manipulated via pointer handles (Int64).

from memory import UnsafePointer, alloc
from .template import (
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
from html.tags import TAG_UNKNOWN


# ── TemplateBuilder ──────────────────────────────────────────────────────────


struct TemplateBuilder(Movable):
    """Step-by-step builder for constructing Templates at runtime.

    Nodes are added via `push_element`, `push_text`, `push_dynamic`, and
    `push_dynamic_text`.  Each push returns the index of the new node in
    the template's flat node array.  A `parent` argument of -1 marks the
    node as a root; otherwise the node is appended as a child of the
    specified parent index.

    Attributes are added via `push_static_attr` and `push_dynamic_attr`,
    which attach to a specific node by index.

    Call `build()` to finalise and produce the Template.  The builder is
    consumed (moved) by `build()`.
    """

    var _name: String
    var _nodes: List[TemplateNode]
    var _attrs: List[TemplateAttribute]
    var _root_indices: List[UInt32]
    # Per-node attribute tracking: parallel to _nodes.
    # _node_attr_start[i] = index into _attrs where node i's attrs begin
    # _node_attr_count[i] = number of attrs for node i
    # These are set lazily when attributes are pushed.
    var _node_attr_start: List[Int]
    var _node_attr_count: List[Int]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self, name: String):
        """Create a builder for a template with the given name."""
        self._name = name
        self._nodes = List[TemplateNode]()
        self._attrs = List[TemplateAttribute]()
        self._root_indices = List[UInt32]()
        self._node_attr_start = List[Int]()
        self._node_attr_count = List[Int]()

    fn __moveinit__(out self, deinit other: Self):
        self._name = other._name^
        self._nodes = other._nodes^
        self._attrs = other._attrs^
        self._root_indices = other._root_indices^
        self._node_attr_start = other._node_attr_start^
        self._node_attr_count = other._node_attr_count^

    # ── Node insertion ───────────────────────────────────────────────

    fn push_element(mut self, html_tag: UInt8, parent: Int) -> Int:
        """Add an Element node.

        Args:
            html_tag: The HTML tag constant (TAG_DIV, TAG_SPAN, etc.).
            parent: Index of the parent node, or -1 for a root node.

        Returns:
            The index of the new node in the template's node list.
        """
        var node = TemplateNode.element(html_tag)
        var idx = len(self._nodes)
        self._nodes.append(node^)
        self._node_attr_start.append(-1)
        self._node_attr_count.append(0)

        if parent == -1:
            self._root_indices.append(UInt32(idx))
        else:
            self._nodes[parent].add_child(UInt32(idx))
        return idx

    fn push_text(mut self, text: String, parent: Int) -> Int:
        """Add a static Text node.

        Args:
            text: The static text content.
            parent: Index of the parent node, or -1 for a root node.

        Returns:
            The index of the new node in the template's node list.
        """
        var node = TemplateNode.static_text(text)
        var idx = len(self._nodes)
        self._nodes.append(node^)
        self._node_attr_start.append(-1)
        self._node_attr_count.append(0)

        if parent == -1:
            self._root_indices.append(UInt32(idx))
        else:
            self._nodes[parent].add_child(UInt32(idx))
        return idx

    fn push_dynamic(mut self, dynamic_index: UInt32, parent: Int) -> Int:
        """Add a Dynamic node placeholder.

        A Dynamic node is a slot for a full dynamic node (component,
        fragment, etc.) that will be filled in at render time from the
        VNode's dynamic_nodes array.

        Args:
            dynamic_index: Index into the VNode's dynamic_nodes array.
            parent: Index of the parent node, or -1 for a root node.

        Returns:
            The index of the new node in the template's node list.
        """
        var node = TemplateNode.dynamic(dynamic_index)
        var idx = len(self._nodes)
        self._nodes.append(node^)
        self._node_attr_start.append(-1)
        self._node_attr_count.append(0)

        if parent == -1:
            self._root_indices.append(UInt32(idx))
        else:
            self._nodes[parent].add_child(UInt32(idx))
        return idx

    fn push_dynamic_text(mut self, dynamic_index: UInt32, parent: Int) -> Int:
        """Add a DynamicText node placeholder.

        A DynamicText node is a slot for dynamic text content that will
        be filled in at render time from the VNode's dynamic_texts array.

        Args:
            dynamic_index: Index into the VNode's dynamic_texts array.
            parent: Index of the parent node, or -1 for a root node.

        Returns:
            The index of the new node in the template's node list.
        """
        var node = TemplateNode.dynamic_text(dynamic_index)
        var idx = len(self._nodes)
        self._nodes.append(node^)
        self._node_attr_start.append(-1)
        self._node_attr_count.append(0)

        if parent == -1:
            self._root_indices.append(UInt32(idx))
        else:
            self._nodes[parent].add_child(UInt32(idx))
        return idx

    # ── Attribute insertion ──────────────────────────────────────────

    fn push_static_attr(mut self, node_index: Int, name: String, value: String):
        """Add a static attribute to the node at `node_index`.

        Args:
            node_index: The index of the node to attach the attribute to.
            name: The attribute name (e.g. "class", "id").
            value: The attribute value.
        """
        var attr_idx = len(self._attrs)
        self._attrs.append(TemplateAttribute.static_attr(name, value))

        if self._node_attr_count[node_index] == 0:
            # First attribute for this node — record start position
            self._node_attr_start[node_index] = attr_idx
        self._node_attr_count[node_index] += 1

    fn push_dynamic_attr(mut self, node_index: Int, dynamic_index: UInt32):
        """Add a dynamic attribute placeholder to the node at `node_index`.

        Args:
            node_index: The index of the node to attach the attribute to.
            dynamic_index: Index into the VNode's dynamic_attrs array.
        """
        var attr_idx = len(self._attrs)
        self._attrs.append(TemplateAttribute.dynamic_attr(dynamic_index))

        if self._node_attr_count[node_index] == 0:
            self._node_attr_start[node_index] = attr_idx
        self._node_attr_count[node_index] += 1

    # ── Queries (pre-build introspection) ────────────────────────────

    fn node_count(self) -> Int:
        """Return the number of nodes added so far."""
        return len(self._nodes)

    fn root_count(self) -> Int:
        """Return the number of root nodes added so far."""
        return len(self._root_indices)

    fn attr_count(self) -> Int:
        """Return the total number of attributes added so far."""
        return len(self._attrs)

    fn node_kind(self, index: Int) -> UInt8:
        """Return the kind of the node at `index`."""
        return self._nodes[index].kind

    fn node_html_tag(self, index: Int) -> UInt8:
        """Return the HTML tag of the node at `index`."""
        return self._nodes[index].html_tag

    fn node_child_count(self, index: Int) -> Int:
        """Return the number of children of the node at `index`."""
        return self._nodes[index].child_count()

    fn node_attr_count_at(self, index: Int) -> Int:
        """Return the number of attributes on the node at `index`."""
        return self._node_attr_count[index]

    fn node_dynamic_index(self, index: Int) -> UInt32:
        """Return the dynamic index of the node at `index`."""
        return self._nodes[index].dynamic_index

    # ── Build ────────────────────────────────────────────────────────

    fn build(mut self) -> Template:
        """Finalise the builder and produce a Template.

        Transfers all nodes, attributes, and root indices into the new
        Template.  Attribute ranges on each node are set based on the
        accumulated push_*_attr calls.

        After calling build(), the builder is left in an empty state.
        """
        # Set attribute ranges on each node before transfer
        for i in range(len(self._nodes)):
            var start = self._node_attr_start[i]
            var count = self._node_attr_count[i]
            if count > 0:
                self._nodes[i].set_attr_range(UInt32(start), UInt32(count))

        # Move data into the template
        var nodes = self._nodes^
        self._nodes = List[TemplateNode]()

        var attrs = self._attrs^
        self._attrs = List[TemplateAttribute]()

        var roots = self._root_indices^
        self._root_indices = List[UInt32]()

        var name = self._name

        # Reset attr tracking
        self._node_attr_start.clear()
        self._node_attr_count.clear()

        return Template(
            id=UInt32(0),
            name=name,
            nodes=nodes^,
            attrs=attrs^,
            root_indices=roots^,
        )


# ── Heap-allocated builder handle ────────────────────────────────────────────
#
# For WASM interop, the builder is heap-allocated and accessed via Int64
# pointer handles.  These helpers manage the lifecycle.


fn create_builder(
    name: String,
) -> UnsafePointer[TemplateBuilder, MutExternalOrigin]:
    """Allocate a TemplateBuilder on the heap and return a pointer."""
    var ptr = alloc[TemplateBuilder](1)
    ptr.init_pointee_move(TemplateBuilder(name))
    return ptr


fn destroy_builder(ptr: UnsafePointer[TemplateBuilder, MutExternalOrigin]):
    """Destroy and free a heap-allocated TemplateBuilder."""
    ptr.destroy_pointee()
    ptr.free()
