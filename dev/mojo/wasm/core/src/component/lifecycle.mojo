# Lifecycle — Reusable mount / update / flush orchestration helpers.
#
# These functions encapsulate the common patterns that every app repeats:
#
#   mount_vnode()        — CreateEngine → append to root → finalize
#   diff_and_finalize()  — DiffEngine → finalize
#   flush_one_scope()    — drain dirty → rebuild → diff → finalize
#
# FragmentSlot + flush_fragment() encapsulate the even more common pattern
# of managing a dynamic list of keyed VNodes inside a parent container:
#
#   empty → populated   — create children, ReplaceWith anchor
#   populated → populated — diff old fragment vs new fragment (keyed)
#   populated → empty   — create new anchor, InsertBefore first item, remove all
#
# ConditionalSlot + flush_conditional() manage a single conditional VNode
# in a dynamic node slot:
#
#   empty → branch       — create VNode, ReplaceWith anchor placeholder
#   branch → branch      — diff old VNode vs new VNode
#   branch → empty       — create new anchor, InsertBefore old root, remove old
#
# They operate on raw pointers so they can be called from any app struct
# without requiring trait conformance.  The app is responsible for:
#
#   1. Building the VNode (app-specific logic)
#   2. Tracking the "current vnode" index
#   3. Calling these helpers with the right pointers
#
# Example (counter-style app):
#
#     # Initial mount
#     var vnode_idx = self.build_vnode()
#     var byte_len = mount_vnode(writer, eid, rt, store, vnode_idx)
#
#     # Subsequent flush
#     var new_idx = self.build_vnode()
#     var byte_len = diff_and_finalize(writer, eid, rt, store, old_idx, new_idx)
#
# Example (keyed list app using FragmentSlot):
#
#     # In app struct:
#     var slot: FragmentSlot
#
#     # Initial setup (after mount):
#     self.slot = FragmentSlot(anchor_eid, empty_frag_idx)
#
#     # Flush after list mutation:
#     var new_frag = self.build_items_fragment()
#     flush_fragment(writer, eid, rt, store, self.slot, new_frag)
#     writer[0].finalize()
#
# Example (conditional rendering using ConditionalSlot):
#
#     # In app struct:
#     var cond: ConditionalSlot
#
#     # Initial setup (after mount):
#     self.cond = ConditionalSlot(anchor_eid)
#
#     # Flush — show a branch:
#     var detail_idx = self.build_detail_vnode()
#     self.cond = flush_conditional(writer, eid, rt, store, self.cond, detail_idx)
#
#     # Flush — hide (back to placeholder):
#     self.cond = flush_conditional_empty(writer, eid, rt, store, self.cond)

from memory import UnsafePointer
from bridge import MutationWriter
from arena import ElementIdAllocator
from signals import Runtime
from mutations import CreateEngine, DiffEngine
from vdom import VNodeStore


# ── FragmentSlot — State tracker for a dynamic fragment in the DOM ────────────


struct FragmentSlot(Copyable, Equatable, Writable):
    """Tracks the state of a fragment-based dynamic list in the DOM.

    A FragmentSlot manages the lifecycle of a list of keyed VNodes that
    live inside a parent container element (e.g. a <ul> or <tbody>).
    When the list is empty, a placeholder comment node occupies the
    slot; when populated, the placeholder is replaced by the list items.

    Fields:
        anchor_id: ElementId of the anchor/placeholder node.  Non-zero
            when the list is empty (placeholder is in the DOM).
            Zero when items are present.
        current_frag: VNode index of the current items Fragment (-1
            before first render).
        mounted: True when items are in the DOM (placeholder removed).
    """

    var anchor_id: UInt32
    var current_frag: Int
    var mounted: Bool

    fn __init__(out self):
        """Create an uninitialized slot."""
        self.anchor_id = 0
        self.current_frag = -1
        self.mounted = False

    fn __init__(out self, anchor_id: UInt32, initial_frag: Int):
        """Create a slot with a known anchor and initial fragment.

        Args:
            anchor_id: ElementId of the placeholder/anchor in the DOM.
            initial_frag: VNode index of the initial (typically empty)
                fragment.
        """
        self.anchor_id = anchor_id
        self.current_frag = initial_frag
        self.mounted = False

    fn __copyinit__(out self, other: Self):
        self.anchor_id = other.anchor_id
        self.current_frag = other.current_frag
        self.mounted = other.mounted

    fn __moveinit__(out self, deinit other: Self):
        self.anchor_id = other.anchor_id
        self.current_frag = other.current_frag
        self.mounted = other.mounted


# ── ConditionalSlot — State tracker for a single conditional VNode ────────────


struct ConditionalSlot(Copyable, Equatable, Writable):
    """Tracks the state of a conditional rendering slot in the DOM.

    A ConditionalSlot manages the lifecycle of a single conditional VNode
    in a dynamic node position (created via `dyn_node(idx)` in a
    template).  When no branch is active, a placeholder comment node
    occupies the slot.  When a branch is active, its VNode tree replaces
    the placeholder.

    This is the single-VNode counterpart to FragmentSlot (which manages
    a list of keyed VNodes).  The diff engine handles all transitions
    automatically — ConditionalSlot just tracks the state so the
    component knows what's currently in the DOM.

    Transitions handled by flush_conditional / flush_conditional_empty:

      - **empty → branch**: CreateEngine on new VNode, ReplaceWith
        anchor placeholder.
      - **branch → branch (same or different template)**: DiffEngine
        on old vs new VNode (handles same template → incremental diff,
        different template → full replacement internally).
      - **branch → empty**: Create a new anchor placeholder,
        InsertBefore old VNode's first root, then remove old VNode roots.

    Fields:
        anchor_id: ElementId of the placeholder comment node.  Non-zero
            when no branch is active.  Zero when a branch is mounted.
        current_vnode: VNode index of the currently mounted branch
            (-1 when empty / placeholder is shown).
        mounted: True when a branch VNode is in the DOM (placeholder
            has been replaced).
    """

    var anchor_id: UInt32
    var current_vnode: Int
    var mounted: Bool

    fn __init__(out self):
        """Create an uninitialized slot."""
        self.anchor_id = 0
        self.current_vnode = -1
        self.mounted = False

    fn __init__(out self, anchor_id: UInt32):
        """Create a slot with a known anchor (placeholder is in the DOM).

        Args:
            anchor_id: ElementId of the placeholder comment node that
                occupies the dynamic node slot after initial mount.
        """
        self.anchor_id = anchor_id
        self.current_vnode = -1
        self.mounted = False

    fn __copyinit__(out self, other: Self):
        self.anchor_id = other.anchor_id
        self.current_vnode = other.current_vnode
        self.mounted = other.mounted

    fn __moveinit__(out self, deinit other: Self):
        self.anchor_id = other.anchor_id
        self.current_vnode = other.current_vnode
        self.mounted = other.mounted


# ── Conditional flush helper ──────────────────────────────────────────────────


fn flush_conditional(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    slot: ConditionalSlot,
    new_vnode_idx: UInt32,
) -> ConditionalSlot:
    """Flush a conditional slot: show or update a branch VNode.

    Handles two transitions:
      1. **empty → branch**: Create the new VNode via CreateEngine,
         emit ReplaceWith to replace the anchor placeholder.
      2. **branch → branch**: Diff the old VNode vs new VNode via
         DiffEngine (handles same template → incremental diff,
         different template → full replacement internally).

    To transition from branch → empty (hide the content), use
    `flush_conditional_empty()` instead.

    Does NOT call `writer_ptr[0].finalize()` — the caller must finalize
    the mutation buffer after this returns.  This allows batching
    multiple flush operations into a single mutation buffer.

    Args:
        writer_ptr: Pointer to the MutationWriter for output.
        eid_ptr: Pointer to the ElementIdAllocator.
        rt_ptr: Pointer to the reactive Runtime.
        store_ptr: Pointer to the VNodeStore.
        slot: The ConditionalSlot tracking the current state.
        new_vnode_idx: Index of the new branch VNode in the store.

    Returns:
        Updated ConditionalSlot with new state.
    """
    var result = slot.copy()

    if not slot.mounted:
        # ── Transition: empty → branch ────────────────────────────────
        # Create the new VNode's DOM and ReplaceWith the anchor.
        var create_eng = CreateEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
        var num_roots = create_eng.create_node(new_vnode_idx)

        if slot.anchor_id != 0 and num_roots > 0:
            writer_ptr[0].replace_with(slot.anchor_id, num_roots)
        result.mounted = True

    else:
        # ── Transition: branch → branch ───────────────────────────────
        # Diff old vs new — DiffEngine handles same/different templates.
        var diff_eng = DiffEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
        diff_eng.diff_node(UInt32(slot.current_vnode), new_vnode_idx)

    result.current_vnode = Int(new_vnode_idx)
    return result.copy()


fn flush_conditional_empty(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    slot: ConditionalSlot,
) -> ConditionalSlot:
    """Flush a conditional slot: hide the current branch (back to placeholder).

    Handles the transition:
      - **branch → empty**: Find the old VNode's first root ElementId,
        create a new anchor placeholder, InsertBefore the first old root,
        then remove all old VNode roots.

    If the slot is already empty, this is a no-op.

    Does NOT call `writer_ptr[0].finalize()` — the caller must finalize
    the mutation buffer after this returns.

    Args:
        writer_ptr: Pointer to the MutationWriter for output.
        eid_ptr: Pointer to the ElementIdAllocator.
        rt_ptr: Pointer to the reactive Runtime.
        store_ptr: Pointer to the VNodeStore.
        slot: The ConditionalSlot tracking the current state.

    Returns:
        Updated ConditionalSlot with empty state and new anchor ID.
    """
    var result = slot.copy()

    if not slot.mounted:
        # Already empty — no-op
        return result.copy()

    # ── Transition: branch → empty ────────────────────────────────────
    var old_vnode_idx = UInt32(slot.current_vnode)
    var old_ptr = store_ptr[0].get_ptr(old_vnode_idx)

    # Find the first root ElementId of the old VNode
    var first_old_root_id: UInt32 = 0
    if old_ptr[0].root_id_count() > 0:
        first_old_root_id = old_ptr[0].get_root_id(0)
    elif old_ptr[0].element_id != 0:
        first_old_root_id = old_ptr[0].element_id

    # Create a new anchor placeholder
    var new_anchor = eid_ptr[0].alloc()
    writer_ptr[0].create_placeholder(new_anchor.as_u32())

    # Insert the placeholder before the first old root
    if first_old_root_id != 0:
        writer_ptr[0].insert_before(first_old_root_id, 1)

    # Remove all old VNode roots
    var diff_eng = DiffEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
    diff_eng._remove_node(old_vnode_idx)

    result.anchor_id = new_anchor.as_u32()
    result.current_vnode = -1
    result.mounted = False
    return result.copy()


# ── Fragment flush helper ─────────────────────────────────────────────────────


fn flush_fragment(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    mut slot: FragmentSlot,
    new_frag_idx: UInt32,
) -> FragmentSlot:
    """Flush a fragment slot: diff old vs new fragment and emit mutations.

    Handles three transitions:
      1. **empty → populated**: Create new fragment children via
         CreateEngine, emit ReplaceWith to replace the anchor placeholder.
      2. **populated → populated**: Diff old fragment vs new fragment
         via DiffEngine (supports keyed children).
      3. **populated → empty**: Create a new anchor placeholder,
         InsertBefore the first old item, then remove all old items.

    Does NOT call `writer_ptr[0].finalize()` — the caller must finalize
    the mutation buffer after this returns.  This allows batching
    multiple flush operations into a single mutation buffer.

    Args:
        writer_ptr: Pointer to the MutationWriter for output.
        eid_ptr: Pointer to the ElementIdAllocator.
        rt_ptr: Pointer to the reactive Runtime.
        store_ptr: Pointer to the VNodeStore.
        slot: The FragmentSlot tracking the current state.  Will be
            updated in-place with the new fragment index and mount state.
        new_frag_idx: Index of the new Fragment VNode in the store.

    Returns:
        Updated FragmentSlot with new state.
    """
    var old_frag_idx = UInt32(slot.current_frag)

    var old_frag_ptr = store_ptr[0].get_ptr(old_frag_idx)
    var new_frag_ptr = store_ptr[0].get_ptr(new_frag_idx)
    var old_count = old_frag_ptr[0].fragment_child_count()
    var new_count = new_frag_ptr[0].fragment_child_count()

    if not slot.mounted and new_count > 0:
        # ── Transition: empty → populated ─────────────────────────────
        # Create fragment children and ReplaceWith the anchor placeholder.
        var create_eng = CreateEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
        var total_roots: UInt32 = 0
        for i in range(new_count):
            var child_idx = (
                store_ptr[0].get_ptr(new_frag_idx)[0].get_fragment_child(i)
            )
            total_roots += create_eng.create_node(child_idx)

        if slot.anchor_id != 0 and total_roots > 0:
            writer_ptr[0].replace_with(slot.anchor_id, total_roots)
        slot.mounted = True

    elif slot.mounted and new_count == 0:
        # ── Transition: populated → empty ─────────────────────────────
        # Find the first old item's root ElementId so we can InsertBefore.
        var first_old_root_id: UInt32 = 0
        if old_count > 0:
            var first_child = (
                store_ptr[0].get_ptr(old_frag_idx)[0].get_fragment_child(0)
            )
            var fc_ptr = store_ptr[0].get_ptr(first_child)
            if fc_ptr[0].root_id_count() > 0:
                first_old_root_id = fc_ptr[0].get_root_id(0)
            elif fc_ptr[0].element_id != 0:
                first_old_root_id = fc_ptr[0].element_id

        # Create a new anchor placeholder
        var new_anchor = eid_ptr[0].alloc()
        writer_ptr[0].create_placeholder(new_anchor.as_u32())

        # Insert the placeholder before the first old item
        if first_old_root_id != 0:
            writer_ptr[0].insert_before(first_old_root_id, 1)

        # Remove all old items
        var diff_eng = DiffEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
        for i in range(old_count):
            var old_child = (
                store_ptr[0].get_ptr(old_frag_idx)[0].get_fragment_child(i)
            )
            diff_eng._remove_node(old_child)

        slot.anchor_id = new_anchor.as_u32()
        slot.mounted = False

    elif slot.mounted and new_count > 0:
        # ── Transition: populated → populated ─────────────────────────
        # Diff old fragment vs new fragment (keyed).
        var diff_eng = DiffEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
        diff_eng.diff_node(old_frag_idx, new_frag_idx)

    # else: both empty → no-op

    slot.current_frag = Int(new_frag_idx)
    return slot.copy()


# ── Single-VNode mount / diff helpers ─────────────────────────────────────────


fn mount_vnode(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    vnode_idx: UInt32,
) -> Int32:
    """Initial render: create mutations for a VNode and append to root.

    Runs CreateEngine on the given VNode, emits AppendChildren to the
    root element (id 0), writes the End sentinel, and returns the byte
    length of the mutation data.

    Args:
        writer_ptr: Pointer to a MutationWriter positioned at the start.
        eid_ptr: Pointer to the ElementIdAllocator for assigning IDs.
        rt_ptr: Pointer to the reactive Runtime (for template lookups).
        store_ptr: Pointer to the VNodeStore containing the VNode.
        vnode_idx: Index of the VNode to mount in the store.

    Returns:
        Byte length of the mutation data written to the buffer.
    """
    var engine = CreateEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
    var num_roots = engine.create_node(vnode_idx)

    # Append created nodes to the DOM root (element id 0)
    writer_ptr[0].append_children(0, num_roots)

    # Write End sentinel
    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn mount_vnode_to(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    vnode_idx: UInt32,
    parent_id: UInt32,
) -> Int32:
    """Initial render: create mutations for a VNode and append to a
    specific parent element.

    Same as `mount_vnode` but appends to `parent_id` instead of root (0).

    Args:
        writer_ptr: Pointer to a MutationWriter positioned at the start.
        eid_ptr: Pointer to the ElementIdAllocator for assigning IDs.
        rt_ptr: Pointer to the reactive Runtime (for template lookups).
        store_ptr: Pointer to the VNodeStore containing the VNode.
        vnode_idx: Index of the VNode to mount in the store.
        parent_id: ElementId of the parent to append children to.

    Returns:
        Byte length of the mutation data written to the buffer.
    """
    var engine = CreateEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
    var num_roots = engine.create_node(vnode_idx)

    writer_ptr[0].append_children(parent_id, num_roots)

    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn diff_and_finalize(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    old_idx: UInt32,
    new_idx: UInt32,
) -> Int32:
    """Diff two VNodes, emit mutations, and finalize the buffer.

    Runs DiffEngine on the old and new VNodes, writes the End sentinel,
    and returns the byte length of the mutation data.

    Args:
        writer_ptr: Pointer to a MutationWriter positioned at the start.
        eid_ptr: Pointer to the ElementIdAllocator.
        rt_ptr: Pointer to the reactive Runtime.
        store_ptr: Pointer to the VNodeStore containing both VNodes.
        old_idx: Index of the old (current) VNode in the store.
        new_idx: Index of the new (updated) VNode in the store.

    Returns:
        Byte length of the mutation data written to the buffer.
    """
    var engine = DiffEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
    engine.diff_node(old_idx, new_idx)

    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn diff_no_finalize(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    old_idx: UInt32,
    new_idx: UInt32,
):
    """Diff two VNodes and emit mutations WITHOUT finalizing.

    Useful when multiple diffs need to be batched into a single mutation
    buffer before the End sentinel is written.  The caller must call
    `writer_ptr[0].finalize()` when all diffs are complete.

    Args:
        writer_ptr: Pointer to a MutationWriter.
        eid_ptr: Pointer to the ElementIdAllocator.
        rt_ptr: Pointer to the reactive Runtime.
        store_ptr: Pointer to the VNodeStore containing both VNodes.
        old_idx: Index of the old (current) VNode in the store.
        new_idx: Index of the new (updated) VNode in the store.
    """
    var engine = DiffEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
    engine.diff_node(old_idx, new_idx)


fn create_no_finalize(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    eid_ptr: UnsafePointer[ElementIdAllocator, MutExternalOrigin],
    rt_ptr: UnsafePointer[Runtime, MutExternalOrigin],
    store_ptr: UnsafePointer[VNodeStore, MutExternalOrigin],
    vnode_idx: UInt32,
) -> UInt32:
    """Create mutations for a VNode WITHOUT finalizing or appending.

    Returns the number of root elements created and pushed onto the
    mutation stack.  The caller is responsible for emitting
    AppendChildren / InsertAfter / etc. and calling finalize().

    Useful for composing multiple creates into a single mutation buffer.

    Args:
        writer_ptr: Pointer to a MutationWriter.
        eid_ptr: Pointer to the ElementIdAllocator.
        rt_ptr: Pointer to the reactive Runtime.
        store_ptr: Pointer to the VNodeStore containing the VNode.
        vnode_idx: Index of the VNode to create in the store.

    Returns:
        Number of root nodes created (pushed onto the mutation stack).
    """
    var engine = CreateEngine(writer_ptr, eid_ptr, rt_ptr, store_ptr)
    return engine.create_node(vnode_idx)
