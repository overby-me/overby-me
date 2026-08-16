# Template Definition — Static UI structure representation.
#
# A Template defines the static structure of a piece of UI.  It is compiled
# once and instantiated many times.  In Tier 1 (runtime), templates are
# created by the builder and cached in a TemplateRegistry.  In Tier 2
# (compile-time), templates will be `comptime` constants.
#
# Templates use a flat arena layout: all nodes are stored in a single
# List[TemplateNode] within the Template, and parent-child relationships
# are expressed via index references.  This avoids recursive struct issues
# and makes serialisation straightforward.
#
# Template nodes come in four kinds:
#   - Element:     An HTML element with a tag, attributes, and children.
#   - Text:        A static text node.
#   - Dynamic:     A placeholder for a dynamic node (index into VNode's
#                  dynamic_nodes array at render time).
#   - DynamicText: A placeholder for dynamic text content (index into
#                  VNode's dynamic_texts array at render time).
#
# Template attributes come in two kinds:
#   - Static:  A name/value pair known at template definition time.
#   - Dynamic: A placeholder (index into VNode's dynamic_attrs array).

from memory import UnsafePointer

# TAG_UNKNOWN sentinel — defined locally to avoid circular import with html.tags
# (tags.mojo was moved from vdom/ to html/ during the mojo-gui separation).
comptime TAG_UNKNOWN: UInt8 = 255


# ── TemplateNode kind tags ───────────────────────────────────────────────────

comptime TNODE_ELEMENT: UInt8 = 0
comptime TNODE_TEXT: UInt8 = 1
comptime TNODE_DYNAMIC: UInt8 = 2
comptime TNODE_DYNAMIC_TEXT: UInt8 = 3


# ── TemplateAttribute kind tags ──────────────────────────────────────────────

comptime TATTR_STATIC: UInt8 = 0
comptime TATTR_DYNAMIC: UInt8 = 1


# ── TemplateAttribute ────────────────────────────────────────────────────────


struct TemplateAttribute(Copyable):
    """An attribute on a template element node.

    Static attributes have a name and value known at template definition time.
    Dynamic attributes are placeholders filled in at render time.
    """

    var kind: UInt8  # TATTR_STATIC or TATTR_DYNAMIC
    var name: String  # attribute name (meaningful for static)
    var value: String  # attribute value (meaningful for static)
    var dynamic_index: UInt32  # index into VNode.dynamic_attrs (for dynamic)

    # ── Static constructor ───────────────────────────────────────────

    @staticmethod
    fn static_attr(name: String, value: String) -> Self:
        """Create a static attribute with a known name and value."""
        return Self(
            kind=TATTR_STATIC,
            name=name,
            value=value,
            dynamic_index=0,
        )

    # ── Dynamic constructor ──────────────────────────────────────────

    @staticmethod
    fn dynamic_attr(index: UInt32) -> Self:
        """Create a dynamic attribute placeholder."""
        return Self(
            kind=TATTR_DYNAMIC,
            name=String(""),
            value=String(""),
            dynamic_index=index,
        )

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        kind: UInt8,
        name: String,
        value: String,
        dynamic_index: UInt32,
    ):
        self.kind = kind
        self.name = name
        self.value = value
        self.dynamic_index = dynamic_index

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.name = other.name
        self.value = other.value
        self.dynamic_index = other.dynamic_index

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.name = other.name^
        self.value = other.value^
        self.dynamic_index = other.dynamic_index

    # ── Queries ──────────────────────────────────────────────────────

    fn is_static(self) -> Bool:
        """Check whether this is a static attribute."""
        return self.kind == TATTR_STATIC

    fn is_dynamic(self) -> Bool:
        """Check whether this is a dynamic attribute placeholder."""
        return self.kind == TATTR_DYNAMIC


# ── TemplateNode ─────────────────────────────────────────────────────────────


struct TemplateNode(Copyable):
    """A node in a template's static structure.

    Uses a tagged-struct pattern to represent the four node kinds.
    All fields are present regardless of kind; unused fields hold
    default/zero values.

    For Element nodes:
      - html_tag identifies the HTML tag (TAG_DIV, TAG_SPAN, etc.)
      - children stores indices into the owning Template's nodes list
      - first_attr / num_attrs delimit this node's attributes in
        the owning Template's attrs list

    For Text nodes:
      - text holds the static text content

    For Dynamic / DynamicText nodes:
      - dynamic_index is the index into the VNode's dynamic arrays
    """

    var kind: UInt8  # TNODE_ELEMENT / TEXT / DYNAMIC / DYNAMIC_TEXT
    var html_tag: UInt8  # TAG_* constant (for Element)
    var children: List[UInt32]  # child node indices (for Element)
    var first_attr: UInt32  # first attribute index in Template.attrs
    var num_attrs: UInt32  # number of attributes for this node
    var text: String  # static text content (for Text)
    var dynamic_index: UInt32  # dynamic slot index (for Dynamic/DynamicText)

    # ── Named constructors ───────────────────────────────────────────

    @staticmethod
    fn element(html_tag: UInt8) -> Self:
        """Create an Element node with the given HTML tag."""
        return Self(
            kind=TNODE_ELEMENT,
            html_tag=html_tag,
            children=List[UInt32](),
            first_attr=0,
            num_attrs=0,
            text=String(""),
            dynamic_index=0,
        )

    @staticmethod
    fn static_text(text: String) -> Self:
        """Create a static Text node."""
        return Self(
            kind=TNODE_TEXT,
            html_tag=TAG_UNKNOWN,
            children=List[UInt32](),
            first_attr=0,
            num_attrs=0,
            text=text,
            dynamic_index=0,
        )

    @staticmethod
    fn dynamic(index: UInt32) -> Self:
        """Create a Dynamic node placeholder."""
        return Self(
            kind=TNODE_DYNAMIC,
            html_tag=TAG_UNKNOWN,
            children=List[UInt32](),
            first_attr=0,
            num_attrs=0,
            text=String(""),
            dynamic_index=index,
        )

    @staticmethod
    fn dynamic_text(index: UInt32) -> Self:
        """Create a DynamicText node placeholder."""
        return Self(
            kind=TNODE_DYNAMIC_TEXT,
            html_tag=TAG_UNKNOWN,
            children=List[UInt32](),
            first_attr=0,
            num_attrs=0,
            text=String(""),
            dynamic_index=index,
        )

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        kind: UInt8,
        html_tag: UInt8,
        var children: List[UInt32],
        first_attr: UInt32,
        num_attrs: UInt32,
        text: String,
        dynamic_index: UInt32,
    ):
        self.kind = kind
        self.html_tag = html_tag
        self.children = children^
        self.first_attr = first_attr
        self.num_attrs = num_attrs
        self.text = text
        self.dynamic_index = dynamic_index

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.html_tag = other.html_tag
        self.children = other.children.copy()
        self.first_attr = other.first_attr
        self.num_attrs = other.num_attrs
        self.text = other.text
        self.dynamic_index = other.dynamic_index

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.html_tag = other.html_tag
        self.children = other.children^
        self.first_attr = other.first_attr
        self.num_attrs = other.num_attrs
        self.text = other.text^
        self.dynamic_index = other.dynamic_index

    # ── Kind queries ─────────────────────────────────────────────────

    fn is_element(self) -> Bool:
        """Check whether this is an Element node."""
        return self.kind == TNODE_ELEMENT

    fn is_text(self) -> Bool:
        """Check whether this is a static Text node."""
        return self.kind == TNODE_TEXT

    fn is_dynamic(self) -> Bool:
        """Check whether this is a Dynamic node placeholder."""
        return self.kind == TNODE_DYNAMIC

    fn is_dynamic_text(self) -> Bool:
        """Check whether this is a DynamicText node placeholder."""
        return self.kind == TNODE_DYNAMIC_TEXT

    # ── Child management (for Element nodes) ─────────────────────────

    fn child_count(self) -> Int:
        """Return the number of child nodes."""
        return len(self.children)

    fn child_at(self, index: Int) -> UInt32:
        """Return the node index of the child at position `index`."""
        return self.children[index]

    fn add_child(mut self, child_index: UInt32):
        """Append a child node index."""
        self.children.append(child_index)

    # ── Attribute range (for Element nodes) ──────────────────────────

    fn attr_count(self) -> Int:
        """Return the number of attributes on this node."""
        return Int(self.num_attrs)

    fn set_attr_range(mut self, first: UInt32, count: UInt32):
        """Set the attribute index range for this node.

        Attributes are stored contiguously in the owning Template's
        attrs list starting at `first` with `count` entries.
        """
        self.first_attr = first
        self.num_attrs = count


# ── Template ─────────────────────────────────────────────────────────────────


struct Template(Copyable):
    """A static UI template — the blueprint for a piece of UI.

    Contains a flat list of all nodes and attributes, plus a list of
    root node indices.  The template is identified by both a numeric ID
    (assigned by the registry) and a string name (for deduplication).

    Nodes reference children and attributes by index into this template's
    `nodes` and `attrs` lists respectively.
    """

    var id: UInt32  # assigned by TemplateRegistry
    var name: String  # unique name (e.g. "counter", "todo-item")
    var nodes: List[TemplateNode]  # flat array of all nodes
    var attrs: List[TemplateAttribute]  # flat array of all attributes
    var root_indices: List[UInt32]  # indices into nodes[] for root-level nodes

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self, name: String):
        """Create an empty template with the given name."""
        self.id = 0
        self.name = name
        self.nodes = List[TemplateNode]()
        self.attrs = List[TemplateAttribute]()
        self.root_indices = List[UInt32]()

    fn __init__(
        out self,
        id: UInt32,
        name: String,
        var nodes: List[TemplateNode],
        var attrs: List[TemplateAttribute],
        var root_indices: List[UInt32],
    ):
        """Create a fully specified template."""
        self.id = id
        self.name = name
        self.nodes = nodes^
        self.attrs = attrs^
        self.root_indices = root_indices^

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.name = other.name
        self.nodes = other.nodes.copy()
        self.attrs = other.attrs.copy()
        self.root_indices = other.root_indices.copy()

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.name = other.name^
        self.nodes = other.nodes^
        self.attrs = other.attrs^
        self.root_indices = other.root_indices^

    # ── Node access ──────────────────────────────────────────────────

    fn node_count(self) -> Int:
        """Return the total number of nodes in this template."""
        return len(self.nodes)

    fn root_count(self) -> Int:
        """Return the number of root-level nodes."""
        return len(self.root_indices)

    fn attr_total_count(self) -> Int:
        """Return the total number of attributes across all nodes."""
        return len(self.attrs)

    fn get_node_ptr(
        self, index: Int
    ) -> UnsafePointer[TemplateNode, MutExternalOrigin]:
        """Return a pointer to the node at `index`.

        The pointer is valid until the next mutation of the template.
        """
        var ptr = self.nodes.unsafe_ptr() + index
        return UnsafePointer[TemplateNode, MutExternalOrigin](
            unsafe_from_address=Int(ptr)
        )

    fn get_root_index(self, i: Int) -> UInt32:
        """Return the node index of the i-th root node."""
        return self.root_indices[i]

    fn get_attr(self, index: Int) -> TemplateAttribute:
        """Return a copy of the attribute at the given index."""
        return self.attrs[index].copy()

    # ── Node queries (convenience) ───────────────────────────────────

    fn node_kind(self, index: Int) -> UInt8:
        """Return the kind tag of the node at `index`."""
        return self.nodes[index].kind

    fn node_html_tag(self, index: Int) -> UInt8:
        """Return the HTML tag of the Element node at `index`."""
        return self.nodes[index].html_tag

    fn node_child_count(self, index: Int) -> Int:
        """Return the number of children of the node at `index`."""
        return self.nodes[index].child_count()

    fn node_child_at(self, node_index: Int, child_pos: Int) -> UInt32:
        """Return the node index of the child at position `child_pos`
        within the node at `node_index`."""
        return self.nodes[node_index].child_at(child_pos)

    fn node_dynamic_index(self, index: Int) -> UInt32:
        """Return the dynamic slot index of the node at `index`."""
        return self.nodes[index].dynamic_index

    fn node_attr_count(self, index: Int) -> Int:
        """Return the number of attributes on the node at `index`."""
        return self.nodes[index].attr_count()

    fn node_first_attr(self, index: Int) -> UInt32:
        """Return the first attribute index of the node at `index`."""
        return self.nodes[index].first_attr

    # ── Mutation (used by builder) ───────────────────────────────────

    fn push_node(mut self, node: TemplateNode) -> UInt32:
        """Append a node to the template and return its index."""
        var idx = UInt32(len(self.nodes))
        self.nodes.append(node^)
        return idx

    fn push_attr(mut self, attr: TemplateAttribute) -> UInt32:
        """Append an attribute to the template and return its index."""
        var idx = UInt32(len(self.attrs))
        self.attrs.append(attr^)
        return idx

    fn push_root(mut self, node_index: UInt32):
        """Mark a node as a root-level node."""
        self.root_indices.append(node_index)

    fn add_child_to_node(mut self, parent_index: UInt32, child_index: UInt32):
        """Add a child index to the parent node's children list."""
        self.nodes[Int(parent_index)].add_child(child_index)

    fn set_node_attr_range(
        mut self, node_index: UInt32, first_attr: UInt32, count: UInt32
    ):
        """Set the attribute range for the node at `node_index`."""
        self.nodes[Int(node_index)].set_attr_range(first_attr, count)

    # ── Dynamic slot counting ────────────────────────────────────────

    fn dynamic_node_count(self) -> Int:
        """Count the number of Dynamic node slots in this template."""
        var count = 0
        for i in range(len(self.nodes)):
            if self.nodes[i].kind == TNODE_DYNAMIC:
                count += 1
        return count

    fn dynamic_text_count(self) -> Int:
        """Count the number of DynamicText node slots in this template."""
        var count = 0
        for i in range(len(self.nodes)):
            if self.nodes[i].kind == TNODE_DYNAMIC_TEXT:
                count += 1
        return count

    fn dynamic_attr_count(self) -> Int:
        """Count the number of dynamic attribute slots in this template."""
        var count = 0
        for i in range(len(self.attrs)):
            if self.attrs[i].kind == TATTR_DYNAMIC:
                count += 1
        return count

    fn static_attr_count(self) -> Int:
        """Count the number of static attributes in this template."""
        var count = 0
        for i in range(len(self.attrs)):
            if self.attrs[i].kind == TATTR_STATIC:
                count += 1
        return count
