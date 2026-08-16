# TemplateRegistry — Stores and deduplicates templates by name.
#
# The registry owns all Template instances and provides O(1) lookup by
# numeric ID (UInt32) and O(n) lookup/deduplication by name string.
# Template names must be unique; registering a template with a name that
# already exists returns the existing template's ID without replacing it.
#
# This is the Tier 1 (runtime) registry.  In Tier 2, compile-time
# templates will be pre-registered as constants.

from memory import UnsafePointer
from .template import Template


# ── TemplateRegistry ─────────────────────────────────────────────────────────


struct TemplateRegistry(Movable):
    """Stores and deduplicates templates by name.

    Each template is assigned a monotonically increasing UInt32 ID when
    first registered.  Subsequent registrations with the same name return
    the existing ID (the new template is discarded).

    Usage:
        var reg = TemplateRegistry()
        var id1 = reg.register(template1^)  # new → id 0
        var id2 = reg.register(template2^)  # same name → id 0 (deduped)
        var id3 = reg.register(template3^)  # different name → id 1
    """

    var _templates: List[Template]
    var _names: List[
        String
    ]  # parallel to _templates; _names[i] == _templates[i].name
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        self._templates = List[Template]()
        self._names = List[String]()
        self._count = 0

    fn __init__(out self, *, capacity: Int):
        """Create a registry with pre-allocated capacity."""
        self._templates = List[Template](capacity=capacity)
        self._names = List[String](capacity=capacity)
        self._count = 0

    fn __moveinit__(out self, deinit other: Self):
        self._templates = other._templates^
        self._names = other._names^
        self._count = other._count

    # ── Register ─────────────────────────────────────────────────────

    fn register(mut self, var template: Template) -> UInt32:
        """Register a template and return its ID.

        If a template with the same name already exists, the existing
        template's ID is returned and the new template is discarded
        (deduplication).

        Args:
            template: The template to register (moved into the registry).

        Returns:
            The UInt32 template ID.
        """
        # Check for an existing template with the same name
        var name = template.name
        for i in range(self._count):
            if self._names[i] == name:
                # Already registered — return existing ID
                return UInt32(i)

        # New template — assign next ID
        var id = UInt32(self._count)
        template.id = id
        self._names.append(name^)
        self._templates.append(template^)
        self._count += 1
        return id

    # ── Lookup ───────────────────────────────────────────────────────

    fn get_ptr(self, id: UInt32) -> UnsafePointer[Template, MutExternalOrigin]:
        """Return a pointer to the template at `id`.

        The pointer is valid until the next mutation of the registry.
        Precondition: `id` < `count()`.
        """
        var ptr = self._templates.unsafe_ptr() + Int(id)
        return UnsafePointer[Template, MutExternalOrigin](
            unsafe_from_address=Int(ptr)
        )

    fn find_by_name(self, name: String) -> Int:
        """Find a template by name.  Returns its ID as Int, or -1 if not found.
        """
        for i in range(self._count):
            if self._names[i] == name:
                return i
        return -1

    fn contains_name(self, name: String) -> Bool:
        """Check whether a template with the given name is registered."""
        return self.find_by_name(name) != -1

    fn contains_id(self, id: UInt32) -> Bool:
        """Check whether a template with the given ID exists."""
        return Int(id) < self._count

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of registered templates."""
        return self._count

    fn name(self, id: UInt32) -> String:
        """Return the name of the template at `id`."""
        return self._names[Int(id)]

    # ── Template property queries (convenience delegates) ────────────

    fn node_count(self, id: UInt32) -> Int:
        """Return the total node count of the template at `id`."""
        return self._templates[Int(id)].node_count()

    fn root_count(self, id: UInt32) -> Int:
        """Return the root node count of the template at `id`."""
        return self._templates[Int(id)].root_count()

    fn attr_total_count(self, id: UInt32) -> Int:
        """Return the total attribute count of the template at `id`."""
        return self._templates[Int(id)].attr_total_count()

    fn node_kind(self, id: UInt32, node_index: Int) -> UInt8:
        """Return the kind tag of a node in the template at `id`."""
        return self._templates[Int(id)].node_kind(node_index)

    fn node_html_tag(self, id: UInt32, node_index: Int) -> UInt8:
        """Return the HTML tag of a node in the template at `id`."""
        return self._templates[Int(id)].node_html_tag(node_index)

    fn node_child_count(self, id: UInt32, node_index: Int) -> Int:
        """Return the child count of a node in the template at `id`."""
        return self._templates[Int(id)].node_child_count(node_index)

    fn node_child_at(
        self, id: UInt32, node_index: Int, child_pos: Int
    ) -> UInt32:
        """Return a child index of a node in the template at `id`."""
        return self._templates[Int(id)].node_child_at(node_index, child_pos)

    fn node_dynamic_index(self, id: UInt32, node_index: Int) -> UInt32:
        """Return the dynamic slot index of a node in the template at `id`."""
        return self._templates[Int(id)].node_dynamic_index(node_index)

    fn node_attr_count(self, id: UInt32, node_index: Int) -> Int:
        """Return the attribute count of a node in the template at `id`."""
        return self._templates[Int(id)].node_attr_count(node_index)

    fn node_first_attr(self, id: UInt32, node_index: Int) -> UInt32:
        """Return the first attribute index of a node in the template at `id`.
        """
        return self._templates[Int(id)].node_first_attr(node_index)

    fn get_root_index(self, id: UInt32, i: Int) -> UInt32:
        """Return the node index of the i-th root of the template at `id`."""
        return self._templates[Int(id)].get_root_index(i)

    fn get_attr_kind(self, id: UInt32, attr_index: Int) -> UInt8:
        """Return the kind of the attribute at `attr_index` in template `id`."""
        return self._templates[Int(id)].attrs[attr_index].kind

    fn get_attr_dynamic_index(self, id: UInt32, attr_index: Int) -> UInt32:
        """Return the dynamic index of the attribute at `attr_index` in template `id`.
        """
        return self._templates[Int(id)].attrs[attr_index].dynamic_index

    fn dynamic_node_count(self, id: UInt32) -> Int:
        """Return the number of Dynamic node slots in the template at `id`."""
        return self._templates[Int(id)].dynamic_node_count()

    fn dynamic_text_count(self, id: UInt32) -> Int:
        """Return the number of DynamicText slots in the template at `id`."""
        return self._templates[Int(id)].dynamic_text_count()

    fn dynamic_attr_count(self, id: UInt32) -> Int:
        """Return the number of dynamic attribute slots in template `id`."""
        return self._templates[Int(id)].dynamic_attr_count()

    fn static_attr_count(self, id: UInt32) -> Int:
        """Return the number of static attributes in template `id`."""
        return self._templates[Int(id)].static_attr_count()

    # ── Bulk operations ──────────────────────────────────────────────

    fn clear(mut self):
        """Remove all registered templates."""
        self._templates.clear()
        self._names.clear()
        self._count = 0
