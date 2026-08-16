# ElementId — Lightweight DOM node identifier.
#
# An ElementId is a u32 handle that uniquely identifies a node in the
# virtual DOM.  The JS interpreter uses these IDs to map mutations to
# real DOM nodes.
#
# The ElementIdAllocator manages a pool of IDs backed by a List with
# an intrusive free list, providing O(1) alloc/free with automatic
# reuse of freed IDs.
#
# ElementId(0) is reserved as the root node / invalid sentinel.


# ── ElementId ────────────────────────────────────────────────────────────────


@fieldwise_init
struct ElementId(Copyable, Equatable, Hashable, Stringable, Writable):
    """A lightweight handle identifying a DOM node.

    Internally just a `UInt32`.  ElementId(0) is reserved for the root
    node and should not be allocated by the user.

    `Equatable`, `Hashable`, and `Writable` are auto-derived from the
    single `id` field.
    """

    var id: UInt32

    # ── Additional constructors ──────────────────────────────────────

    fn __init__(out self, id: Int):
        self.id = UInt32(id)

    # ── Queries ──────────────────────────────────────────────────────

    @always_inline
    fn is_root(self) -> Bool:
        """Check whether this is the root element (id == 0)."""
        return self.id == 0

    @always_inline
    fn is_valid(self) -> Bool:
        """Check whether this is a non-root, potentially valid ID."""
        return self.id != 0

    @always_inline
    fn as_u32(self) -> UInt32:
        """Return the raw u32 value."""
        return self.id

    @always_inline
    fn as_int(self) -> Int:
        """Return the raw value as Int."""
        return Int(self.id)

    # ── Trait implementations ────────────────────────────────────────

    fn __str__(self) -> String:
        return String("ElementId(") + String(self.id) + String(")")


# ── Sentinels ────────────────────────────────────────────────────────────────

comptime ROOT_ELEMENT_ID = ElementId(UInt32(0))
comptime INVALID_ELEMENT_ID = ElementId(UInt32(0))


# ── Slot state for the allocator ─────────────────────────────────────────────


@fieldwise_init
struct _SlotState(Copyable, Equatable, Writable):
    """Tracks whether an ID slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only meaningful when not occupied; -1 = end of free list.


# ── ElementIdAllocator ───────────────────────────────────────────────────────


struct ElementIdAllocator(Movable):
    """Allocates and recycles ElementIds using a List-backed free list.

    ID 0 is reserved (root / invalid).  The first user-allocated ID is 1.

    Usage:
        var alloc = ElementIdAllocator()
        var id1 = alloc.alloc()    # ElementId(1)
        var id2 = alloc.alloc()    # ElementId(2)
        alloc.free(id1)            # id1 available for reuse
        var id3 = alloc.alloc()    # ElementId(1) — reused!
    """

    var _slots: List[_SlotState]
    var _free_head: Int  # Index of first free slot, or -1 if none
    var _count: Int  # Number of occupied slots (including reserved root)

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create an allocator.  Slot 0 is auto-reserved."""
        self._slots = List[_SlotState]()
        self._free_head = -1
        self._count = 0
        # Reserve slot 0 so user IDs start at 1
        self._reserve_root()

    fn __init__(out self, *, capacity: Int):
        """Create an allocator with pre-allocated capacity."""
        self._slots = List[_SlotState](capacity=capacity)
        self._free_head = -1
        self._count = 0
        self._reserve_root()

    fn __moveinit__(out self, deinit other: Self):
        self._slots = other._slots^
        self._free_head = other._free_head
        self._count = other._count

    fn _reserve_root(mut self):
        """Reserve slot 0 for the root element."""
        # Append an occupied slot at index 0
        self._slots.append(_SlotState(occupied=True, next_free=-1))
        self._count = 1

    # ── Core API ─────────────────────────────────────────────────────

    fn alloc(mut self) -> ElementId:
        """Allocate a new ElementId.  O(1).

        Freed IDs are reused.  The returned ID is guaranteed to be > 0.
        """
        if self._free_head != -1:
            # Reuse a freed slot
            var idx = self._free_head
            self._free_head = self._slots[idx].next_free
            self._slots[idx] = _SlotState(occupied=True, next_free=-1)
            self._count += 1
            return ElementId(UInt32(idx))
        else:
            # Append a new slot
            var idx = len(self._slots)
            self._slots.append(_SlotState(occupied=True, next_free=-1))
            self._count += 1
            return ElementId(UInt32(idx))

    fn free(mut self, id: ElementId):
        """Free an ElementId for reuse.  O(1).

        Freeing the root ID (0) is a no-op.
        Freeing an already-freed ID is a no-op (no double-free crash).
        """
        if id.is_root():
            return
        var idx = id.as_int()
        if idx < 0 or idx >= len(self._slots):
            return  # Out of bounds
        if not self._slots[idx].occupied:
            return  # Already vacant
        self._slots[idx] = _SlotState(occupied=False, next_free=self._free_head)
        self._free_head = idx
        self._count -= 1

    fn is_alive(self, id: ElementId) -> Bool:
        """Check whether the given ID is currently allocated."""
        var idx = id.as_int()
        if idx < 0 or idx >= len(self._slots):
            return False
        return self._slots[idx].occupied

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of allocated IDs (including the reserved root)."""
        return self._count

    fn user_count(self) -> Int:
        """Return the number of user-allocated IDs (excluding root)."""
        return self._count - 1

    fn next_id(self) -> ElementId:
        """Return the ID that the next `alloc()` will return.

        Useful for pre-computing IDs before allocation.
        """
        if self._free_head != -1:
            return ElementId(UInt32(self._free_head))
        return ElementId(UInt32(len(self._slots)))

    fn clear(mut self):
        """Free all IDs.  Slot 0 is re-reserved."""
        self._slots.clear()
        self._free_head = -1
        self._count = 0
        self._reserve_root()
