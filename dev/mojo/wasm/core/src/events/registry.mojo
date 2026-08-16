# Event Handler Registry — Maps handler IDs to event actions.
#
# The registry stores event handlers that are triggered when DOM events
# fire.  Since Mojo WASM cannot store closures or function pointers in
# the traditional sense, handlers are encoded as *actions*: a tag byte
# describing what to do, plus a signal key and an operand value.
#
# Supported actions:
#   - SIGNAL_SET_I32:  signal.set(operand)
#   - SIGNAL_ADD_I32:  signal += operand
#   - SIGNAL_SUB_I32:  signal -= operand
#   - SIGNAL_TOGGLE:   signal.set(!signal.get())  (for Bool signals stored as i32)
#   - SIGNAL_SET_INPUT: signal.set(input_value)   (value comes from event data)
#   - SIGNAL_SET_STRING: string_signal.set(string_value)  (Phase 20: string from event)
#   - CUSTOM:          no Mojo-side action; JS is responsible for the side effect
#
# Each handler also records the owning scope_id so the runtime can mark
# the correct scope dirty after the action executes.
#
# The registry uses a slab allocator (free-list) identical to SignalStore
# and ScopeArena, so handler IDs are stable and reusable.
#
# Integration:
#   - When a VNode has a dynamic event attribute (AVAL_EVENT), the
#     handler_id stored in the AttributeValue refers to a slot in this
#     registry.
#   - The JS EventBridge captures DOM events, looks up the element's
#     handler_id, and calls `dispatch_event` on the WASM side.
#   - `dispatch_event` executes the action, which writes to a signal,
#     which in turn marks subscribing scopes dirty.

from memory import UnsafePointer


# ── Event type tags ──────────────────────────────────────────────────────────
#
# These match the event names that the JS EventBridge listens for.
# They are sent as part of the dispatch call so Mojo knows which kind
# of event data to expect.

comptime EVT_CLICK: UInt8 = 0
comptime EVT_INPUT: UInt8 = 1
comptime EVT_KEY_DOWN: UInt8 = 2
comptime EVT_KEY_UP: UInt8 = 3
comptime EVT_MOUSE_MOVE: UInt8 = 4
comptime EVT_FOCUS: UInt8 = 5
comptime EVT_BLUR: UInt8 = 6
comptime EVT_SUBMIT: UInt8 = 7
comptime EVT_CHANGE: UInt8 = 8
comptime EVT_MOUSE_DOWN: UInt8 = 9
comptime EVT_MOUSE_UP: UInt8 = 10
comptime EVT_MOUSE_ENTER: UInt8 = 11
comptime EVT_MOUSE_LEAVE: UInt8 = 12
comptime EVT_CUSTOM: UInt8 = 255


# ── Handler action tags ──────────────────────────────────────────────────────
#
# Describe the Mojo-side effect of an event handler.

comptime ACTION_NONE: UInt8 = 0
comptime ACTION_SIGNAL_SET_I32: UInt8 = 1
comptime ACTION_SIGNAL_ADD_I32: UInt8 = 2
comptime ACTION_SIGNAL_SUB_I32: UInt8 = 3
comptime ACTION_SIGNAL_TOGGLE: UInt8 = 4
comptime ACTION_SIGNAL_SET_INPUT: UInt8 = 5
comptime ACTION_SIGNAL_SET_STRING: UInt8 = 6
comptime ACTION_KEY_ENTER_CUSTOM: UInt8 = 7
comptime ACTION_CUSTOM: UInt8 = 255


# ── HandlerEntry ─────────────────────────────────────────────────────────────


struct HandlerEntry(Copyable, Equatable, Writable):
    """A single event handler entry in the registry.

    Fields:
        scope_id:   The scope that owns this handler. When the handler
                    fires, this scope (and any signal subscribers) will
                    be marked dirty.
        action:     One of the ACTION_* tags describing what to do.
        signal_key: The signal to modify (ignored for ACTION_NONE/CUSTOM).
        operand:    The value to use in the action (e.g. the amount to add).
        event_name: The DOM event name (e.g. "click", "input") for JS lookup.
    """

    var scope_id: UInt32
    var action: UInt8
    var signal_key: UInt32
    var operand: Int32
    var event_name: String

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create an empty (default) handler entry."""
        self.scope_id = 0
        self.action = ACTION_NONE
        self.signal_key = 0
        self.operand = 0
        self.event_name = String("")

    fn __init__(
        out self,
        scope_id: UInt32,
        action: UInt8,
        signal_key: UInt32,
        operand: Int32,
        event_name: String,
    ):
        """Create a handler entry with all fields specified."""
        self.scope_id = scope_id
        self.action = action
        self.signal_key = signal_key
        self.operand = operand
        self.event_name = event_name

    fn __copyinit__(out self, other: Self):
        self.scope_id = other.scope_id
        self.action = other.action
        self.signal_key = other.signal_key
        self.operand = other.operand
        self.event_name = other.event_name

    fn __moveinit__(out self, deinit other: Self):
        self.scope_id = other.scope_id
        self.action = other.action
        self.signal_key = other.signal_key
        self.operand = other.operand
        self.event_name = other.event_name^

    # ── Convenience constructors ─────────────────────────────────────

    @staticmethod
    fn signal_set(
        scope_id: UInt32,
        signal_key: UInt32,
        value: Int32,
        event_name: String,
    ) -> HandlerEntry:
        """Create a handler that sets a signal to a fixed value."""
        return HandlerEntry(
            scope_id, ACTION_SIGNAL_SET_I32, signal_key, value, event_name
        )

    @staticmethod
    fn signal_add(
        scope_id: UInt32,
        signal_key: UInt32,
        delta: Int32,
        event_name: String,
    ) -> HandlerEntry:
        """Create a handler that adds `delta` to a signal."""
        return HandlerEntry(
            scope_id, ACTION_SIGNAL_ADD_I32, signal_key, delta, event_name
        )

    @staticmethod
    fn signal_sub(
        scope_id: UInt32,
        signal_key: UInt32,
        delta: Int32,
        event_name: String,
    ) -> HandlerEntry:
        """Create a handler that subtracts `delta` from a signal."""
        return HandlerEntry(
            scope_id, ACTION_SIGNAL_SUB_I32, signal_key, delta, event_name
        )

    @staticmethod
    fn signal_toggle(
        scope_id: UInt32,
        signal_key: UInt32,
        event_name: String,
    ) -> HandlerEntry:
        """Create a handler that toggles a boolean signal (0 ↔ 1)."""
        return HandlerEntry(
            scope_id, ACTION_SIGNAL_TOGGLE, signal_key, 0, event_name
        )

    @staticmethod
    fn signal_set_input(
        scope_id: UInt32,
        signal_key: UInt32,
        event_name: String,
    ) -> HandlerEntry:
        """Create a handler that sets a signal from the event's input value.

        The actual string value comes from JS via the event data buffer.
        For now this is a placeholder — the operand is unused.
        """
        return HandlerEntry(
            scope_id, ACTION_SIGNAL_SET_INPUT, signal_key, 0, event_name
        )

    @staticmethod
    fn signal_set_string(
        scope_id: UInt32,
        string_key: UInt32,
        version_key: UInt32,
        event_name: String,
    ) -> HandlerEntry:
        """Create a handler that sets a SignalString from a string event value.

        Phase 20: The string value is passed from JS via
        `dispatch_event_with_string()`.  The handler stores the
        string_key in signal_key and the version_key in operand
        (cast to Int32), allowing the runtime to call
        `write_signal_string(string_key, version_key, value)`.

        Args:
            scope_id: The owning scope.
            string_key: The key in the Runtime's StringStore.
            version_key: The companion version signal key in SignalStore.
            event_name: The DOM event name (e.g. "input").
        """
        return HandlerEntry(
            scope_id,
            ACTION_SIGNAL_SET_STRING,
            string_key,
            Int32(version_key),
            event_name,
        )

    @staticmethod
    fn key_enter_custom(scope_id: UInt32) -> HandlerEntry:
        """Create a handler that fires only when the key is "Enter".

        Phase 22: Used by `onkeydown_enter_custom()` DSL helper.
        Dispatched via `dispatch_event_with_string()` — the runtime
        checks the string payload against "Enter" and marks the scope
        dirty only on match.  The app's `handle_event()` then performs
        custom routing based on the handler ID (same as ACTION_CUSTOM).

        Args:
            scope_id: The owning scope.
        """
        return HandlerEntry(
            scope_id, ACTION_KEY_ENTER_CUSTOM, 0, 0, String("keydown")
        )

    @staticmethod
    fn custom(scope_id: UInt32, event_name: String) -> HandlerEntry:
        """Create a handler with no Mojo-side action (JS handles it)."""
        return HandlerEntry(scope_id, ACTION_CUSTOM, 0, 0, event_name)

    @staticmethod
    fn noop(scope_id: UInt32, event_name: String) -> HandlerEntry:
        """Create a no-op handler (marks scope dirty but does nothing else)."""
        return HandlerEntry(scope_id, ACTION_NONE, 0, 0, event_name)


# ── Slot state ───────────────────────────────────────────────────────────────


@fieldwise_init
struct HandlerSlotState(Copyable, Equatable, Writable):
    """Tracks whether a handler slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only valid when not occupied; -1 = end of free list.


# ── HandlerRegistry ──────────────────────────────────────────────────────────


struct HandlerRegistry(Movable):
    """Slab-allocated registry of event handlers.

    Handlers are identified by UInt32 IDs that remain stable across
    register/remove cycles (freed slots are reused via a free list).

    The registry does NOT own the Runtime — the caller must pass a
    Runtime pointer when calling `dispatch` so the registry can read
    and write signals.
    """

    var _entries: List[HandlerEntry]
    var _states: List[HandlerSlotState]
    var _free_head: Int
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        self._entries = List[HandlerEntry]()
        self._states = List[HandlerSlotState]()
        self._free_head = -1
        self._count = 0

    fn __moveinit__(out self, deinit other: Self):
        self._entries = other._entries^
        self._states = other._states^
        self._free_head = other._free_head
        self._count = other._count

    # ── Register / Remove ────────────────────────────────────────────

    fn register(mut self, entry: HandlerEntry) -> UInt32:
        """Register a new handler.  Returns its stable ID."""
        if self._free_head != -1:
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._entries[idx] = entry.copy()
            self._states[idx] = HandlerSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            var idx = len(self._entries)
            self._entries.append(entry.copy())
            self._states.append(HandlerSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    fn remove(mut self, id: UInt32):
        """Remove the handler at `id`, freeing its slot for reuse."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        self._entries[idx] = HandlerEntry()
        self._states[idx] = HandlerSlotState(
            occupied=False, next_free=self._free_head
        )
        self._free_head = idx
        self._count -= 1

    # ── Queries ──────────────────────────────────────────────────────

    fn count(self) -> Int:
        """Return the number of live handlers."""
        return self._count

    fn contains(self, id: UInt32) -> Bool:
        """Check whether `id` is a live handler."""
        var idx = Int(id)
        if idx < 0 or idx >= len(self._states):
            return False
        return self._states[idx].occupied

    fn get(self, id: UInt32) -> HandlerEntry:
        """Return a copy of the handler entry at `id`.

        Precondition: `contains(id)` is True.
        """
        return self._entries[Int(id)].copy()

    fn get_ptr(
        self, id: UInt32
    ) -> UnsafePointer[HandlerEntry, MutExternalOrigin]:
        """Return a pointer to the handler entry at `id`.

        Precondition: `contains(id)` is True.
        """
        return UnsafePointer.address_of(self._entries[Int(id)])

    fn scope_id(self, id: UInt32) -> UInt32:
        """Return the scope_id of the handler at `id`."""
        return self._entries[Int(id)].scope_id

    fn action(self, id: UInt32) -> UInt8:
        """Return the action tag of the handler at `id`."""
        return self._entries[Int(id)].action

    fn signal_key(self, id: UInt32) -> UInt32:
        """Return the signal_key of the handler at `id`."""
        return self._entries[Int(id)].signal_key

    fn operand(self, id: UInt32) -> Int32:
        """Return the operand of the handler at `id`."""
        return self._entries[Int(id)].operand

    fn event_name(self, id: UInt32) -> String:
        """Return the event_name of the handler at `id`."""
        return self._entries[Int(id)].event_name

    # ── Bulk operations ──────────────────────────────────────────────

    fn remove_for_scope(mut self, scope_id: UInt32):
        """Remove all handlers belonging to the given scope.

        This is called when a scope is destroyed to clean up its handlers.
        """
        for i in range(len(self._entries)):
            if self._states[i].occupied:
                if self._entries[i].scope_id == scope_id:
                    self.remove(UInt32(i))

    fn handlers_for_scope(self, scope_id: UInt32) -> List[UInt32]:
        """Return a list of handler IDs belonging to the given scope."""
        var result = List[UInt32]()
        for i in range(len(self._entries)):
            if self._states[i].occupied:
                if self._entries[i].scope_id == scope_id:
                    result.append(UInt32(i))
        return result^

    fn clear(mut self):
        """Remove all handlers."""
        self._entries.clear()
        self._states.clear()
        self._free_head = -1
        self._count = 0
