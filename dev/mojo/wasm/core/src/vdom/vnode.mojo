# VNode — Rendered output representation.
#
# A VNode is the output of a component's render function.  It references
# a Template and fills in the dynamic parts (text, attributes, child
# nodes).  VNodes form the virtual DOM tree that the diff algorithm
# compares to produce mutations.
#
# Since Mojo does not have discriminated unions (variants), we use a
# tagged-struct pattern: a `kind` field discriminates the variant, and
# all possible fields are present (unused fields hold default values).
#
# VNode kinds:
#   - TemplateRef:  A template instantiation with dynamic values filled in.
#   - Text:         A plain text node.
#   - Placeholder:  An empty slot for conditional/suspended content.
#   - Fragment:     Multiple adjacent nodes (no wrapper element).
#
# Supporting types:
#   - DynamicNode:  A dynamic child node within a TemplateRef.
#   - DynamicAttr:  A dynamic attribute within a TemplateRef.
#   - AttributeValue: The value of a dynamic attribute (tagged union).

from memory import UnsafePointer


# ── VNode kind tags ──────────────────────────────────────────────────────────

comptime VNODE_TEMPLATE_REF: UInt8 = 0
comptime VNODE_TEXT: UInt8 = 1
comptime VNODE_PLACEHOLDER: UInt8 = 2
comptime VNODE_FRAGMENT: UInt8 = 3


# ── AttributeValue kind tags ─────────────────────────────────────────────────

comptime AVAL_TEXT: UInt8 = 0
comptime AVAL_INT: UInt8 = 1
comptime AVAL_FLOAT: UInt8 = 2
comptime AVAL_BOOL: UInt8 = 3
comptime AVAL_EVENT: UInt8 = 4
comptime AVAL_NONE: UInt8 = 5


# ── DynamicNode kind tags ────────────────────────────────────────────────────

comptime DNODE_TEXT: UInt8 = 0
comptime DNODE_PLACEHOLDER: UInt8 = 1


# ── AttributeValue ───────────────────────────────────────────────────────────


struct AttributeValue(Copyable, Equatable, Writable):
    """The value of a dynamic attribute.

    Tagged union supporting text, integer, float, boolean, event handler
    reference, or none (attribute removal).

    `Equatable` and `Writable` are auto-derived from all fields.  Since
    named constructors always set inactive fields to defaults (empty
    string, 0, False), field-by-field equality is semantically correct.
    """

    var kind: UInt8  # AVAL_*
    var text_value: String  # for AVAL_TEXT
    var int_value: Int64  # for AVAL_INT
    var float_value: Float64  # for AVAL_FLOAT
    var bool_value: Bool  # for AVAL_BOOL
    var handler_id: UInt32  # for AVAL_EVENT (event handler registry key)

    # ── Named constructors ───────────────────────────────────────────

    @staticmethod
    fn text(value: String) -> Self:
        """Create a text attribute value."""
        return Self(
            kind=AVAL_TEXT,
            text_value=value,
            int_value=0,
            float_value=0.0,
            bool_value=False,
            handler_id=0,
        )

    @staticmethod
    fn integer(value: Int64) -> Self:
        """Create an integer attribute value."""
        return Self(
            kind=AVAL_INT,
            text_value=String(""),
            int_value=value,
            float_value=0.0,
            bool_value=False,
            handler_id=0,
        )

    @staticmethod
    fn floating(value: Float64) -> Self:
        """Create a float attribute value."""
        return Self(
            kind=AVAL_FLOAT,
            text_value=String(""),
            int_value=0,
            float_value=value,
            bool_value=False,
            handler_id=0,
        )

    @staticmethod
    fn boolean(value: Bool) -> Self:
        """Create a boolean attribute value."""
        return Self(
            kind=AVAL_BOOL,
            text_value=String(""),
            int_value=0,
            float_value=0.0,
            bool_value=value,
            handler_id=0,
        )

    @staticmethod
    fn event(handler_id: UInt32) -> Self:
        """Create an event handler attribute value."""
        return Self(
            kind=AVAL_EVENT,
            text_value=String(""),
            int_value=0,
            float_value=0.0,
            bool_value=False,
            handler_id=handler_id,
        )

    @staticmethod
    fn none() -> Self:
        """Create a none/empty attribute value (for attribute removal)."""
        return Self(
            kind=AVAL_NONE,
            text_value=String(""),
            int_value=0,
            float_value=0.0,
            bool_value=False,
            handler_id=0,
        )

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        kind: UInt8,
        text_value: String,
        int_value: Int64,
        float_value: Float64,
        bool_value: Bool,
        handler_id: UInt32,
    ):
        self.kind = kind
        self.text_value = text_value
        self.int_value = int_value
        self.float_value = float_value
        self.bool_value = bool_value
        self.handler_id = handler_id

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.text_value = other.text_value
        self.int_value = other.int_value
        self.float_value = other.float_value
        self.bool_value = other.bool_value
        self.handler_id = other.handler_id

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.text_value = other.text_value^
        self.int_value = other.int_value
        self.float_value = other.float_value
        self.bool_value = other.bool_value
        self.handler_id = other.handler_id

    # ── Queries ──────────────────────────────────────────────────────

    fn is_text(self) -> Bool:
        """Check whether this is a text attribute value."""
        return self.kind == AVAL_TEXT

    fn is_int(self) -> Bool:
        """Check whether this is an integer attribute value."""
        return self.kind == AVAL_INT

    fn is_float(self) -> Bool:
        """Check whether this is a float attribute value."""
        return self.kind == AVAL_FLOAT

    fn is_bool(self) -> Bool:
        """Check whether this is a boolean attribute value."""
        return self.kind == AVAL_BOOL

    fn is_event(self) -> Bool:
        """Check whether this is an event handler attribute value."""
        return self.kind == AVAL_EVENT

    fn is_none(self) -> Bool:
        """Check whether this is a none/empty attribute value."""
        return self.kind == AVAL_NONE


# ── DynamicAttr ──────────────────────────────────────────────────────────────


struct DynamicAttr(Copyable, Equatable, Writable):
    """A dynamic attribute on a VNode's template instantiation.

    Each dynamic attribute specifies which element in the template it
    belongs to (via element_id), the attribute name, an optional namespace,
    and the current value.

    `Equatable` and `Writable` are auto-derived from all fields.
    """

    var name: String
    var namespace: String  # empty string = no namespace
    var value: AttributeValue
    var element_id: UInt32  # which element in the template this attr targets

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        name: String,
        var value: AttributeValue,
        element_id: UInt32,
    ):
        """Create a dynamic attribute with no namespace."""
        self.name = name
        self.namespace = String("")
        self.value = value^
        self.element_id = element_id

    fn __init__(
        out self,
        name: String,
        namespace: String,
        var value: AttributeValue,
        element_id: UInt32,
    ):
        """Create a dynamic attribute with a namespace."""
        self.name = name
        self.namespace = namespace
        self.value = value^
        self.element_id = element_id

    fn __copyinit__(out self, other: Self):
        self.name = other.name
        self.namespace = other.namespace
        self.value = other.value.copy()
        self.element_id = other.element_id

    fn __moveinit__(out self, deinit other: Self):
        self.name = other.name^
        self.namespace = other.namespace^
        self.value = other.value^
        self.element_id = other.element_id

    # ── Queries ──────────────────────────────────────────────────────

    fn has_namespace(self) -> Bool:
        """Check whether this attribute has a namespace."""
        return len(self.namespace) > 0


# ── DynamicNode ──────────────────────────────────────────────────────────────


struct DynamicNode(Copyable, Equatable, Writable):
    """A dynamic child node within a TemplateRef VNode.

    Dynamic nodes fill the slots identified by TemplateNode.Dynamic entries
    in the template.  They can be text nodes or placeholders.  More complex
    dynamic content (components, fragments) is handled by nesting VNodes.

    `Equatable` and `Writable` are auto-derived from all fields.
    """

    var kind: UInt8  # DNODE_TEXT or DNODE_PLACEHOLDER
    var text: String  # for DNODE_TEXT

    # ── Named constructors ───────────────────────────────────────────

    @staticmethod
    fn text_node(text: String) -> Self:
        """Create a dynamic text node."""
        return Self(kind=DNODE_TEXT, text=text)

    @staticmethod
    fn placeholder() -> Self:
        """Create a dynamic placeholder node."""
        return Self(kind=DNODE_PLACEHOLDER, text=String(""))

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self, kind: UInt8, text: String):
        self.kind = kind
        self.text = text

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.text = other.text

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.text = other.text^

    # ── Queries ──────────────────────────────────────────────────────

    fn is_text(self) -> Bool:
        """Check whether this is a text node."""
        return self.kind == DNODE_TEXT

    fn is_placeholder(self) -> Bool:
        """Check whether this is a placeholder."""
        return self.kind == DNODE_PLACEHOLDER


# ── VNode ────────────────────────────────────────────────────────────────────


struct VNode(Copyable):
    """A virtual DOM node — the output of a component's render function.

    VNodes form the virtual DOM tree.  The diff algorithm compares old and
    new VNode trees to produce a minimal set of DOM mutations.

    Kinds:
      - TemplateRef: References a registered template and provides dynamic
        values (nodes, attributes, text) to fill in the template's slots.
      - Text: A plain text node with string content.
      - Placeholder: An empty node used for conditional/suspended content
        that has no current output.
      - Fragment: A list of adjacent child VNodes with no wrapper element.

    For TemplateRef nodes:
      - template_id identifies the template in the TemplateRegistry.
      - dynamic_nodes fill Dynamic slots in the template.
      - dynamic_attrs fill dynamic attribute slots.
      - key is an optional string for keyed diffing (list reconciliation).

    For Text nodes:
      - text holds the string content.

    For Placeholder nodes:
      - element_id identifies the placeholder in the DOM (for replacement).

    For Fragment nodes:
      - fragment_children holds the list of child VNode indices into the
        owning VNodeStore (to avoid recursive ownership issues).
    """

    var kind: UInt8  # VNODE_*
    var template_id: UInt32  # for TemplateRef
    var dynamic_nodes: List[DynamicNode]  # for TemplateRef
    var dynamic_attrs: List[DynamicAttr]  # for TemplateRef
    var key: String  # optional key (empty = no key)
    var text: String  # for Text
    var element_id: UInt32  # for Placeholder (also used for Text after mount)
    var fragment_children: List[UInt32]  # child VNode indices (for Fragment)

    # ── Mount state (populated by create, consumed by diff) ──────────
    #
    # These fields are empty after construction and are populated by the
    # create engine when a VNode is first mounted to the DOM.  The diff
    # engine reads them from the old VNode to target mutations and
    # transfers/updates them on the new VNode.
    #
    # root_ids:       ElementIds assigned to this VNode's root elements.
    #                 For TemplateRef: one per template root.
    #                 For Text/Placeholder: single entry (mirrors element_id).
    #                 For Fragment: empty (children have their own).
    #
    # dyn_node_ids:   ElementIds for dynamic node slots within a TemplateRef.
    #                 Parallel to dynamic_nodes — dyn_node_ids[i] is the
    #                 ElementId assigned to dynamic_nodes[i]'s DOM node.
    #
    # dyn_attr_ids:   ElementIds for elements that own dynamic attributes
    #                 within a TemplateRef.  Parallel to dynamic_attrs —
    #                 dyn_attr_ids[i] is the ElementId of the template
    #                 element that hosts dynamic_attrs[i].

    var root_ids: List[UInt32]
    var dyn_node_ids: List[UInt32]
    var dyn_attr_ids: List[UInt32]

    # ── Named constructors ───────────────────────────────────────────

    @staticmethod
    fn template_ref(template_id: UInt32) -> Self:
        """Create a TemplateRef VNode for the given template.

        Dynamic nodes and attributes can be added after construction
        via push_dynamic_node and push_dynamic_attr.
        """
        return Self(
            kind=VNODE_TEMPLATE_REF,
            template_id=template_id,
            dynamic_nodes=List[DynamicNode](),
            dynamic_attrs=List[DynamicAttr](),
            key=String(""),
            text=String(""),
            element_id=0,
            fragment_children=List[UInt32](),
            root_ids=List[UInt32](),
            dyn_node_ids=List[UInt32](),
            dyn_attr_ids=List[UInt32](),
        )

    @staticmethod
    fn template_ref_keyed(template_id: UInt32, key: String) -> Self:
        """Create a keyed TemplateRef VNode."""
        return Self(
            kind=VNODE_TEMPLATE_REF,
            template_id=template_id,
            dynamic_nodes=List[DynamicNode](),
            dynamic_attrs=List[DynamicAttr](),
            key=key,
            text=String(""),
            element_id=0,
            fragment_children=List[UInt32](),
            root_ids=List[UInt32](),
            dyn_node_ids=List[UInt32](),
            dyn_attr_ids=List[UInt32](),
        )

    @staticmethod
    fn text_node(text: String) -> Self:
        """Create a Text VNode."""
        return Self(
            kind=VNODE_TEXT,
            template_id=0,
            dynamic_nodes=List[DynamicNode](),
            dynamic_attrs=List[DynamicAttr](),
            key=String(""),
            text=text,
            element_id=0,
            fragment_children=List[UInt32](),
            root_ids=List[UInt32](),
            dyn_node_ids=List[UInt32](),
            dyn_attr_ids=List[UInt32](),
        )

    @staticmethod
    fn placeholder(element_id: UInt32) -> Self:
        """Create a Placeholder VNode."""
        return Self(
            kind=VNODE_PLACEHOLDER,
            template_id=0,
            dynamic_nodes=List[DynamicNode](),
            dynamic_attrs=List[DynamicAttr](),
            key=String(""),
            text=String(""),
            element_id=element_id,
            fragment_children=List[UInt32](),
            root_ids=List[UInt32](),
            dyn_node_ids=List[UInt32](),
            dyn_attr_ids=List[UInt32](),
        )

    @staticmethod
    fn fragment() -> Self:
        """Create an empty Fragment VNode.

        Children are added via push_fragment_child.
        """
        return Self(
            kind=VNODE_FRAGMENT,
            template_id=0,
            dynamic_nodes=List[DynamicNode](),
            dynamic_attrs=List[DynamicAttr](),
            key=String(""),
            text=String(""),
            element_id=0,
            fragment_children=List[UInt32](),
            root_ids=List[UInt32](),
            dyn_node_ids=List[UInt32](),
            dyn_attr_ids=List[UInt32](),
        )

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        kind: UInt8,
        template_id: UInt32,
        var dynamic_nodes: List[DynamicNode],
        var dynamic_attrs: List[DynamicAttr],
        key: String,
        text: String,
        element_id: UInt32,
        var fragment_children: List[UInt32],
        var root_ids: List[UInt32],
        var dyn_node_ids: List[UInt32],
        var dyn_attr_ids: List[UInt32],
    ):
        self.kind = kind
        self.template_id = template_id
        self.dynamic_nodes = dynamic_nodes^
        self.dynamic_attrs = dynamic_attrs^
        self.key = key
        self.text = text
        self.element_id = element_id
        self.fragment_children = fragment_children^
        self.root_ids = root_ids^
        self.dyn_node_ids = dyn_node_ids^
        self.dyn_attr_ids = dyn_attr_ids^

    fn __copyinit__(out self, other: Self):
        self.kind = other.kind
        self.template_id = other.template_id
        self.dynamic_nodes = other.dynamic_nodes.copy()
        self.dynamic_attrs = other.dynamic_attrs.copy()
        self.key = other.key
        self.text = other.text
        self.element_id = other.element_id
        self.fragment_children = other.fragment_children.copy()
        self.root_ids = other.root_ids.copy()
        self.dyn_node_ids = other.dyn_node_ids.copy()
        self.dyn_attr_ids = other.dyn_attr_ids.copy()

    fn __moveinit__(out self, deinit other: Self):
        self.kind = other.kind
        self.template_id = other.template_id
        self.dynamic_nodes = other.dynamic_nodes^
        self.dynamic_attrs = other.dynamic_attrs^
        self.key = other.key^
        self.text = other.text^
        self.element_id = other.element_id
        self.fragment_children = other.fragment_children^
        self.root_ids = other.root_ids^
        self.dyn_node_ids = other.dyn_node_ids^
        self.dyn_attr_ids = other.dyn_attr_ids^

    # ── Kind queries ─────────────────────────────────────────────────

    fn is_template_ref(self) -> Bool:
        """Check whether this is a TemplateRef VNode."""
        return self.kind == VNODE_TEMPLATE_REF

    fn is_text(self) -> Bool:
        """Check whether this is a Text VNode."""
        return self.kind == VNODE_TEXT

    fn is_placeholder(self) -> Bool:
        """Check whether this is a Placeholder VNode."""
        return self.kind == VNODE_PLACEHOLDER

    fn is_fragment(self) -> Bool:
        """Check whether this is a Fragment VNode."""
        return self.kind == VNODE_FRAGMENT

    # ── Key ──────────────────────────────────────────────────────────

    fn has_key(self) -> Bool:
        """Check whether this VNode has a key."""
        return len(self.key) > 0

    # ── Dynamic content (for TemplateRef) ────────────────────────────

    fn dynamic_node_count(self) -> Int:
        """Return the number of dynamic nodes."""
        return len(self.dynamic_nodes)

    fn dynamic_attr_count(self) -> Int:
        """Return the number of dynamic attributes."""
        return len(self.dynamic_attrs)

    fn push_dynamic_node(mut self, var node: DynamicNode):
        """Append a dynamic node to this TemplateRef VNode."""
        self.dynamic_nodes.append(node^)

    fn push_dynamic_attr(mut self, var attr: DynamicAttr):
        """Append a dynamic attribute to this TemplateRef VNode."""
        self.dynamic_attrs.append(attr^)

    fn get_dynamic_node_kind(self, index: Int) -> UInt8:
        """Return the kind of the dynamic node at `index`."""
        return self.dynamic_nodes[index].kind

    fn get_dynamic_attr_kind(self, index: Int) -> UInt8:
        """Return the attribute value kind of the dynamic attr at `index`."""
        return self.dynamic_attrs[index].value.kind

    fn get_dynamic_attr_element_id(self, index: Int) -> UInt32:
        """Return the element_id of the dynamic attr at `index`."""
        return self.dynamic_attrs[index].element_id

    # ── Fragment children ────────────────────────────────────────────

    fn fragment_child_count(self) -> Int:
        """Return the number of children in this Fragment VNode."""
        return len(self.fragment_children)

    fn push_fragment_child(mut self, child_index: UInt32):
        """Append a child VNode index to this Fragment VNode.

        The child_index refers to a VNode stored externally (e.g. in a
        VNodeStore or a List[VNode] managed by the caller).
        """
        self.fragment_children.append(child_index)

    fn get_fragment_child(self, index: Int) -> UInt32:
        """Return the VNode index of the child at position `index`."""
        return self.fragment_children[index]

    # ── Mount state (populated by create engine) ─────────────────────

    fn is_mounted(self) -> Bool:
        """Check whether this VNode has been mounted (has assigned ElementIds).
        """
        return len(self.root_ids) > 0 or self.element_id != 0

    fn root_id_count(self) -> Int:
        """Return the number of root ElementIds assigned to this VNode."""
        return len(self.root_ids)

    fn get_root_id(self, index: Int) -> UInt32:
        """Return the root ElementId at position `index`."""
        return self.root_ids[index]

    fn push_root_id(mut self, id: UInt32):
        """Append a root ElementId (called by create engine)."""
        self.root_ids.append(id)

    fn dyn_node_id_count(self) -> Int:
        """Return the number of dynamic node ElementIds."""
        return len(self.dyn_node_ids)

    fn get_dyn_node_id(self, index: Int) -> UInt32:
        """Return the dynamic node ElementId at position `index`."""
        return self.dyn_node_ids[index]

    fn push_dyn_node_id(mut self, id: UInt32):
        """Append a dynamic node ElementId (called by create engine)."""
        self.dyn_node_ids.append(id)

    fn dyn_attr_id_count(self) -> Int:
        """Return the number of dynamic attribute target ElementIds."""
        return len(self.dyn_attr_ids)

    fn get_dyn_attr_id(self, index: Int) -> UInt32:
        """Return the dynamic attribute target ElementId at position `index`."""
        return self.dyn_attr_ids[index]

    fn push_dyn_attr_id(mut self, id: UInt32):
        """Append a dynamic attribute target ElementId (called by create engine).
        """
        self.dyn_attr_ids.append(id)

    fn clear_mount_state(mut self):
        """Clear all mount state (for recycling/replacement)."""
        self.root_ids.clear()
        self.dyn_node_ids.clear()
        self.dyn_attr_ids.clear()

    fn transfer_mount_state_to(self, mut target: VNode):
        """Copy this VNode's mount state to `target`.

        Used by the diff engine when the old and new VNodes share the
        same template — the ElementIds remain valid and are transferred.
        """
        target.root_ids = self.root_ids.copy()
        target.dyn_node_ids = self.dyn_node_ids.copy()
        target.dyn_attr_ids = self.dyn_attr_ids.copy()
        target.element_id = self.element_id


# ── VNodeStore ───────────────────────────────────────────────────────────────


struct VNodeStore(Movable):
    """Flat storage for VNode instances.

    VNodes are stored in a flat list and referenced by index (UInt32).
    This avoids recursive ownership issues with Fragment children and
    allows the diff algorithm to work with indices rather than pointers.

    The store supports creation and retrieval but not deletion of individual
    nodes.  Entire stores are discarded and rebuilt on each render cycle.
    """

    var _nodes: List[VNode]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        self._nodes = List[VNode]()

    fn __init__(out self, *, capacity: Int):
        """Create a store with pre-allocated capacity."""
        self._nodes = List[VNode](capacity=capacity)

    fn __moveinit__(out self, deinit other: Self):
        self._nodes = other._nodes^

    # ── Push ─────────────────────────────────────────────────────────

    fn push(mut self, var node: VNode) -> UInt32:
        """Add a VNode to the store and return its index."""
        var idx = UInt32(len(self._nodes))
        self._nodes.append(node^)
        return idx

    # ── Access ───────────────────────────────────────────────────────

    fn get_ptr(self, index: UInt32) -> UnsafePointer[VNode, MutExternalOrigin]:
        """Return a pointer to the VNode at `index`.

        Valid until the store is mutated.
        """
        var ptr = self._nodes.unsafe_ptr() + Int(index)
        return UnsafePointer[VNode, MutExternalOrigin](
            unsafe_from_address=Int(ptr)
        )

    fn kind(self, index: UInt32) -> UInt8:
        """Return the kind tag of the VNode at `index`."""
        return self._nodes[Int(index)].kind

    fn template_id(self, index: UInt32) -> UInt32:
        """Return the template_id of the VNode at `index`."""
        return self._nodes[Int(index)].template_id

    fn element_id(self, index: UInt32) -> UInt32:
        """Return the element_id of the Placeholder VNode at `index`."""
        return self._nodes[Int(index)].element_id

    fn has_key(self, index: UInt32) -> Bool:
        """Check if the VNode at `index` has a key."""
        return self._nodes[Int(index)].has_key()

    fn dynamic_node_count(self, index: UInt32) -> Int:
        """Return the dynamic node count of the VNode at `index`."""
        return self._nodes[Int(index)].dynamic_node_count()

    fn dynamic_attr_count(self, index: UInt32) -> Int:
        """Return the dynamic attribute count of the VNode at `index`."""
        return self._nodes[Int(index)].dynamic_attr_count()

    fn fragment_child_count(self, index: UInt32) -> Int:
        """Return the fragment child count of the VNode at `index`."""
        return self._nodes[Int(index)].fragment_child_count()

    fn get_fragment_child(self, vnode_index: UInt32, child_pos: Int) -> UInt32:
        """Return the fragment child VNode index at position `child_pos`."""
        return self._nodes[Int(vnode_index)].get_fragment_child(child_pos)

    fn get_dynamic_node_kind(
        self, vnode_index: UInt32, dyn_index: Int
    ) -> UInt8:
        """Return the kind of the dynamic node at position `dyn_index`."""
        return self._nodes[Int(vnode_index)].get_dynamic_node_kind(dyn_index)

    fn get_dynamic_attr_kind(
        self, vnode_index: UInt32, attr_index: Int
    ) -> UInt8:
        """Return the value kind of the dynamic attr at position `attr_index`.
        """
        return self._nodes[Int(vnode_index)].get_dynamic_attr_kind(attr_index)

    fn get_dynamic_attr_element_id(
        self, vnode_index: UInt32, attr_index: Int
    ) -> UInt32:
        """Return the element_id of the dynamic attr at position `attr_index`.
        """
        return self._nodes[Int(vnode_index)].get_dynamic_attr_element_id(
            attr_index
        )

    # ── Mutations on stored VNodes ───────────────────────────────────

    fn push_dynamic_node(mut self, vnode_index: UInt32, var node: DynamicNode):
        """Append a dynamic node to the VNode at `vnode_index`."""
        self._nodes[Int(vnode_index)].push_dynamic_node(node^)

    fn push_dynamic_attr(mut self, vnode_index: UInt32, var attr: DynamicAttr):
        """Append a dynamic attribute to the VNode at `vnode_index`."""
        self._nodes[Int(vnode_index)].push_dynamic_attr(attr^)

    fn push_fragment_child(mut self, vnode_index: UInt32, child_index: UInt32):
        """Append a child VNode index to the Fragment at `vnode_index`."""
        self._nodes[Int(vnode_index)].push_fragment_child(child_index)

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of VNodes in the store."""
        return len(self._nodes)

    # ── Bulk operations ──────────────────────────────────────────────

    fn clear(mut self):
        """Remove all VNodes."""
        self._nodes.clear()
