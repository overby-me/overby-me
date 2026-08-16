# Scheduler — Height-ordered dirty scope queue with render prioritization.
#
# When signals change, the scopes that subscribe to them are marked dirty.
# The Scheduler collects these dirty scopes and processes them in
# height-first order (lowest height = shallowest = closest to root).
#
# This ensures parent components re-render before their children,
# which avoids redundant child re-renders when a parent's output
# changes the child tree entirely.
#
# Design:
#   - `collect()` drains the runtime's dirty queue into the scheduler.
#   - `next()` returns the next scope to render (lowest height first).
#   - `is_empty()` checks if there are pending scopes.
#   - Duplicate scope IDs are deduplicated automatically.
#
# The scheduler does NOT own the Runtime — it borrows a pointer to it
# for querying scope heights and draining the dirty queue.

from memory import UnsafePointer
from signals import Runtime


@fieldwise_init
struct SchedulerEntry(Copyable, Equatable, Writable):
    """A dirty scope entry with its height for sorting."""

    var scope_id: UInt32
    var height: UInt32

    fn __copyinit__(out self, other: Self):
        self.scope_id = other.scope_id
        self.height = other.height

    fn __moveinit__(out self, deinit other: Self):
        self.scope_id = other.scope_id
        self.height = other.height


struct Scheduler(Movable):
    """Height-ordered dirty scope queue.

    Collects dirty scopes from a Runtime and yields them in height-first
    order (shallowest first) for rendering.  Deduplicates scope IDs so
    each scope is rendered at most once per flush cycle.

    Usage:
        var sched = Scheduler()
        sched.collect(runtime_ptr)
        while not sched.is_empty():
            var scope_id = sched.next()
            # ... re-render scope_id ...
    """

    var _queue: List[SchedulerEntry]
    var _sorted: Bool

    fn __init__(out self):
        self._queue = List[SchedulerEntry]()
        self._sorted = False

    fn __moveinit__(out self, deinit other: Self):
        self._queue = other._queue^
        self._sorted = other._sorted

    fn collect(mut self, rt: UnsafePointer[Runtime, MutExternalOrigin]):
        """Drain the runtime's dirty queue into the scheduler.

        Deduplicates against any entries already in the queue.
        Marks the queue as unsorted so the next `next()` call will sort.
        """
        var dirty = rt[0].drain_dirty()
        for i in range(len(dirty)):
            var sid = dirty[i]
            # Deduplicate: skip if already queued
            if not self._contains(sid):
                var h = rt[0].scopes.height(sid)
                self._queue.append(SchedulerEntry(sid, h))
        if len(dirty) > 0:
            self._sorted = False

    fn collect_one(
        mut self,
        rt: UnsafePointer[Runtime, MutExternalOrigin],
        scope_id: UInt32,
    ):
        """Add a single scope to the queue (if not already present).

        Useful when you know exactly which scope is dirty without
        draining the full runtime queue.
        """
        if not self._contains(scope_id):
            var h = rt[0].scopes.height(scope_id)
            self._queue.append(SchedulerEntry(scope_id, h))
            self._sorted = False

    fn next(mut self) -> UInt32:
        """Return and remove the next scope to render (lowest height first).

        Precondition: `not self.is_empty()`.

        After sorting (if needed), pops the front entry (lowest height).
        """
        if not self._sorted:
            self._sort()
            self._sorted = True

        # Pop from the front (lowest height)
        var entry = self._queue[0].copy()

        # Shift remaining entries forward
        var new_queue = List[SchedulerEntry]()
        for i in range(1, len(self._queue)):
            new_queue.append(self._queue[i].copy())
        self._queue = new_queue^

        return entry.scope_id

    fn peek(self) -> UInt32:
        """Return the next scope ID without removing it.

        Precondition: `not self.is_empty()`.
        Note: May not reflect sorted order if queue is unsorted.
        """
        return self._queue[0].scope_id

    fn is_empty(self) -> Bool:
        """Check if there are no pending dirty scopes."""
        return len(self._queue) == 0

    fn count(self) -> Int:
        """Return the number of pending dirty scopes."""
        return len(self._queue)

    fn clear(mut self):
        """Discard all pending dirty scopes."""
        self._queue = List[SchedulerEntry]()
        self._sorted = False

    fn has_scope(self, scope_id: UInt32) -> Bool:
        """Check if a specific scope is already in the queue."""
        return self._contains(scope_id)

    # ── Internals ────────────────────────────────────────────────────

    fn _contains(self, scope_id: UInt32) -> Bool:
        """Check if scope_id is already in the queue."""
        for i in range(len(self._queue)):
            if self._queue[i].scope_id == scope_id:
                return True
        return False

    fn _sort(mut self):
        """Sort the queue by height (ascending) using insertion sort.

        Insertion sort is optimal here because:
          1. The queue is typically very small (< 20 entries).
          2. It's often nearly sorted already.
          3. It's stable (preserves insertion order for equal heights).
        """
        for i in range(1, len(self._queue)):
            var key_scope = self._queue[i].scope_id
            var key_height = self._queue[i].height
            var j = i - 1
            while j >= 0 and self._queue[j].height > key_height:
                self._queue[j + 1] = SchedulerEntry(
                    self._queue[j].scope_id, self._queue[j].height
                )
                j -= 1
            self._queue[j + 1] = SchedulerEntry(key_scope, key_height)
