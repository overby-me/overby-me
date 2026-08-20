# Reactive Runtime — Global signal storage and context tracking.
#
# This module provides the core reactive infrastructure, including scope
# management for components:
#
#   - `SignalStore`  — Type-erased storage for all signal values, backed
#     by raw memory. Each signal is identified by a `UInt32` key.
#
#   - `Runtime`      — Top-level runtime state: the signal store, the
#     "current reactive context" pointer, subscriber bookkeeping, and
#     the dirty-scope queue.
#
# WASM is single-threaded, so there are no synchronisation concerns.
# However, Mojo does not support module-level mutable variables, so
# we heap-allocate a single `Runtime` instance and pass its pointer
# through exported functions.
#
# Subscriber model (mirrors Dioxus):
#   - A **reactive context** is any entity (scope, memo, effect) that
#     reads signals and wants to be notified when they change.
#   - When a signal is read while a context is active, the context's ID
#     is recorded in the signal's subscriber set.
#   - When a signal is written, all subscribers are marked dirty.
#
# Memory layout per signal slot:
#   - value_ptr    : Pointer[UInt8, MutUntrackedOrigin]  — type-erased value storage
#   - value_size   : Int                   — byte size of the stored value
#   - subscribers  : List[UInt32]          — context IDs subscribed to this signal
#   - version      : UInt32               — monotonic write counter (for staleness checks)

from std.sys import size_of
from std.memory import Pointer, unsafe_memcpy
from std.memory.alloc import unsafe_alloc
from scope import ScopeArena, ScopeState, HOOK_SIGNAL, HOOK_MEMO, HOOK_EFFECT
from vdom import TemplateRegistry, VNodeStore
from .memo import MemoStore, MemoEntry, MEMO_NO_STRING
from .effect import EffectStore, EffectEntry

# Tag bit used to distinguish scope reactive contexts from signal keys.
# Scope IDs (from ScopeArena) and signal keys (from SignalStore) share
# the UInt32 namespace.  When a scope is used as a reactive context
# (i.e. subscribed to signals during rendering), its scope_id is ORed
# with this tag.  This prevents false matches against memo/effect
# context IDs (which are bare signal keys) in write_signal's
# subscriber classification.
comptime SCOPE_CONTEXT_TAG: UInt32 = UInt32(0x80000000)
from events import (
    HandlerRegistry,
    HandlerEntry,
    ACTION_NONE,
    ACTION_SIGNAL_SET_I32,
    ACTION_SIGNAL_ADD_I32,
    ACTION_SIGNAL_SUB_I32,
    ACTION_SIGNAL_TOGGLE,
    ACTION_SIGNAL_SET_INPUT,
    ACTION_SIGNAL_SET_STRING,
    ACTION_KEY_ENTER_CUSTOM,
    ACTION_CUSTOM,
)


# ── SignalEntry ──────────────────────────────────────────────────────────────


struct SignalEntry(Copyable):
    """Type-erased storage for a single signal's value + subscribers."""

    var value_ptr: Pointer[UInt8, MutUntrackedOrigin]
    var value_size: Int
    var subscribers: List[UInt32]
    var version: UInt32

    # ── Construction ─────────────────────────────────────────────────

    def __init__(out self):
        """Create an empty (uninitialised) entry."""
        self.value_ptr = Pointer[UInt8, MutUntrackedOrigin].unsafe_dangling()
        self.value_size = 0
        self.subscribers = List[UInt32]()
        self.version = 0

    def __init__(out self, ptr: Pointer[UInt8, MutUntrackedOrigin], size: Int):
        """Create an entry that owns `size` bytes at `ptr`."""
        self.value_ptr = ptr
        self.value_size = size
        self.subscribers = List[UInt32]()
        self.version = 0

    def __init__(out self, *, copy: Self):
        self.value_size = copy.value_size
        self.subscribers = copy.subscribers.copy()
        self.version = copy.version
        if copy.value_size > 0:
            self.value_ptr = unsafe_alloc[UInt8](copy.value_size)
            unsafe_memcpy(
                dest=self.value_ptr, src=copy.value_ptr, count=copy.value_size
            )
        else:
            self.value_ptr = Pointer[
                UInt8, MutUntrackedOrigin
            ].unsafe_dangling()

    def __init__(out self, *, deinit move: Self):
        self.value_ptr = move.value_ptr
        self.value_size = move.value_size
        self.subscribers = move.subscribers^
        self.version = move.version

    def __deinit__(deinit self):
        """Destroy the entry, freeing value storage."""
        if self.value_size > 0:
            self.value_ptr.unsafe_free()

    # ── Value access ─────────────────────────────────────────────────

    @always_inline
    def read_value[T: Copyable ](self) -> T:
        """Reinterpret the raw bytes as T and return a copy."""
        return self.value_ptr.unsafe_bitcast[T]()[unsafe_offset=0].copy()

    @always_inline
    def write_value[T: Copyable & Deinitable ](mut self, value: T):
        """Overwrite the stored bytes with `value` and bump version."""
        var tmp = unsafe_alloc[T](1)
        tmp.unsafe_write(value.copy())
        unsafe_memcpy(
            dest=self.value_ptr, src=tmp.unsafe_bitcast[UInt8](), count=self.value_size
        )
        tmp.unsafe_deinit_pointee()
        tmp.unsafe_free()
        self.version += 1

    # ── Subscriber management ────────────────────────────────────────

    def subscribe(mut self, context_id: UInt32):
        """Add `context_id` to the subscriber set (idempotent)."""
        for i in range(len(self.subscribers)):
            if self.subscribers[i] == context_id:
                return  # already subscribed
        self.subscribers.append(context_id)

    def unsubscribe(mut self, context_id: UInt32):
        """Remove `context_id` from the subscriber set."""
        for i in range(len(self.subscribers)):
            if self.subscribers[i] == context_id:
                # Swap-remove for O(1)
                var last_idx = len(self.subscribers) - 1
                if i != last_idx:
                    self.subscribers[i] = self.subscribers[last_idx]
                _ = self.subscribers.pop()
                return

    def subscriber_count(self) -> Int:
        """Return the number of subscribed contexts."""
        return len(self.subscribers)


# ── Slot state for the signal store ──────────────────────────────────────────


@fieldwise_init
struct SignalSlotState(Copyable, Equatable, Writable):
    """Tracks whether a signal slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only valid when not occupied; -1 = end of free list.


# ── SignalStore ──────────────────────────────────────────────────────────────


struct SignalStore(Movable):
    """Type-erased storage for all signal values.

    Each signal is identified by a `UInt32` key.  Values are stored as
    raw byte blobs so the store does not need to be parameterised on T.
    """

    var _entries: List[SignalEntry]
    var _states: List[SignalSlotState]
    var _free_head: Int
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    def __init__(out self):
        self._entries = List[SignalEntry]()
        self._states = List[SignalSlotState]()
        self._free_head = -1
        self._count = 0

    def __init__(out self, *, deinit move: Self):
        self._entries = move._entries^
        self._states = move._states^
        self._free_head = move._free_head
        self._count = move._count

    # ── Create / Destroy ─────────────────────────────────────────────

    def create[
        T: Copyable & Deinitable 
    ](mut self, initial: T) -> UInt32:
        """Create a new signal with `initial` value.  Returns its key."""
        var sz = size_of[T]()

        # Allocate value storage and copy initial value into it
        var ptr = unsafe_alloc[UInt8](sz)
        var tmp = unsafe_alloc[T](1)
        tmp.unsafe_write(initial.copy())
        unsafe_memcpy(dest=ptr, src=tmp.unsafe_bitcast[UInt8](), count=sz)
        tmp.unsafe_deinit_pointee()
        tmp.unsafe_free()

        var entry = SignalEntry(ptr, sz)

        if self._free_head != -1:
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._entries[idx] = entry^
            self._states[idx] = SignalSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            var idx = len(self._entries)
            self._entries.append(entry^)
            self._states.append(SignalSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    def destroy(mut self, key: UInt32):
        """Remove the signal at `key`, freeing its storage."""
        var idx = Int(key)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        # Replace with an empty entry — the old entry's __del__ frees storage
        self._entries[idx] = SignalEntry()
        self._states[idx] = SignalSlotState(
            occupied=False, next_free=self._free_head
        )
        self._free_head = idx
        self._count -= 1

    # ── Read / Write ─────────────────────────────────────────────────

    @always_inline
    def read[T: Copyable ](self, key: UInt32) -> T:
        """Read the signal value at `key` as type T.

        This does NOT perform subscriber tracking — call `read_tracked`
        if you want the current context to be subscribed.
        """
        return self._entries[Int(key)].read_value[T]()

    def read_tracked[
        T: Copyable 
    ](mut self, key: UInt32, context_id: UInt32) -> T:
        """Read the signal value and subscribe `context_id`.

        This is the "normal" read path during component rendering:
        the reading context is automatically subscribed so it will be
        notified when the signal changes.
        """
        self._entries[Int(key)].subscribe(context_id)
        return self._entries[Int(key)].read_value[T]()

    def write[
        T: Copyable & Deinitable 
    ](mut self, key: UInt32, value: T):
        """Write a new value to the signal at `key`.

        Returns without notifying subscribers — call `get_subscribers`
        afterwards to retrieve the dirty context list.
        """
        self._entries[Int(key)].write_value[T](value)

    def peek[T: Copyable ](self, key: UInt32) -> T:
        """Read without subscribing.  Alias for `read`."""
        return self.read[T](key)

    # ── Subscriber queries ───────────────────────────────────────────

    def subscribe(mut self, key: UInt32, context_id: UInt32):
        """Manually subscribe `context_id` to the signal at `key`."""
        self._entries[Int(key)].subscribe(context_id)

    def unsubscribe(mut self, key: UInt32, context_id: UInt32):
        """Remove `context_id` from the signal's subscriber set."""
        self._entries[Int(key)].unsubscribe(context_id)

    def subscriber_count(self, key: UInt32) -> Int:
        """Return how many contexts are subscribed to signal `key`."""
        return self._entries[Int(key)].subscriber_count()

    def get_subscribers(self, key: UInt32) -> List[UInt32]:
        """Return a copy of the subscriber list for signal `key`.

        The caller typically iterates this to mark scopes dirty after
        a write.
        """
        return self._entries[Int(key)].subscribers.copy()

    def version(self, key: UInt32) -> UInt32:
        """Return the write-version counter for signal `key`."""
        return self._entries[Int(key)].version

    # ── Queries ──────────────────────────────────────────────────────

    def signal_count(self) -> Int:
        """Number of live signals."""
        return self._count

    def contains(self, key: UInt32) -> Bool:
        """Check whether `key` is a live signal."""
        var idx = Int(key)
        if idx < 0 or idx >= len(self._states):
            return False
        return self._states[idx].occupied


# ── StringStore ──────────────────────────────────────────────────────────────
#
# Safe storage for heap-allocated String signal values.
#
# The SignalStore uses type-erased raw-byte storage (memcpy), which is
# correct for fixed-size value types like Int32 but unsafe for types
# with ownership semantics like String (double-dangling pointers).
#
# StringStore solves this by storing Strings in a proper List[String]
# with slab-style free-list reuse.  Each SignalString gets:
#   - A string_key  — index into StringStore for the actual String value
#   - A version_key — an Int32 signal in SignalStore for reactivity
#     (subscriptions, dirty-marking, version tracking)
#
# When a SignalString is written, we update the StringStore entry and
# bump the version signal, which triggers the existing subscriber/dirty
# mechanism.  This keeps the reactive plumbing in one place (SignalStore)
# while safely managing heap strings.


@fieldwise_init
struct _StringSlotState(Copyable, Equatable, Writable):
    """Tracks whether a string slot is occupied or vacant."""

    var occupied: Bool
    var next_free: Int  # Only valid when not occupied; -1 = end of free list.


struct StringStore(Movable):
    """Safe storage for String signal values.

    Each string is identified by a `UInt32` key (index into the store).
    Freed slots are recycled via a free list for O(1) allocation.

    This store does NOT handle reactivity — that is managed by a
    companion Int32 "version signal" in the SignalStore.
    """

    var _entries: List[String]
    var _states: List[_StringSlotState]
    var _free_head: Int
    var _count: Int

    # ── Construction ─────────────────────────────────────────────────

    def __init__(out self):
        self._entries = List[String]()
        self._states = List[_StringSlotState]()
        self._free_head = -1
        self._count = 0

    def __init__(out self, *, deinit move: Self):
        self._entries = move._entries^
        self._states = move._states^
        self._free_head = move._free_head
        self._count = move._count

    # ── Create / Destroy ─────────────────────────────────────────────

    def create(mut self, initial: String) -> UInt32:
        """Store a new string and return its key."""
        if self._free_head != -1:
            var idx = self._free_head
            self._free_head = self._states[idx].next_free
            self._entries[idx] = initial
            self._states[idx] = _StringSlotState(occupied=True, next_free=-1)
            self._count += 1
            return UInt32(idx)
        else:
            var idx = len(self._entries)
            self._entries.append(initial)
            self._states.append(_StringSlotState(occupied=True, next_free=-1))
            self._count += 1
            return UInt32(idx)

    def destroy(mut self, key: UInt32):
        """Remove the string at `key`, freeing its slot for reuse."""
        var idx = Int(key)
        if idx < 0 or idx >= len(self._entries):
            return
        if not self._states[idx].occupied:
            return
        # Replace with empty string to release heap memory
        self._entries[idx] = String("")
        self._states[idx] = _StringSlotState(
            occupied=False, next_free=self._free_head
        )
        self._free_head = idx
        self._count -= 1

    # ── Read / Write ─────────────────────────────────────────────────

    def read(self, key: UInt32) -> String:
        """Read the string at `key`.  Returns a copy."""
        return self._entries[Int(key)]

    def write(mut self, key: UInt32, value: String):
        """Write a new string value at `key`.

        Does NOT handle reactivity — the caller must bump the companion
        version signal after calling this.
        """
        self._entries[Int(key)] = value

    # ── Queries ──────────────────────────────────────────────────────

    def count(self) -> Int:
        """Number of live string entries."""
        return self._count

    def contains(self, key: UInt32) -> Bool:
        """Check whether `key` is a live string slot."""
        var idx = Int(key)
        if idx < 0 or idx >= len(self._states):
            return False
        return self._states[idx].occupied


# ── Runtime ──────────────────────────────────────────────────────────────────


struct Runtime(Movable):
    """Top-level reactive runtime state.

    Owns the signal store, the string store, the current reactive context
    pointer, and the dirty-scope queue.  A single instance is
    heap-allocated at framework init and its pointer is threaded through
    all exports.
    """

    var signals: SignalStore
    var strings: StringStore
    var scopes: ScopeArena
    var templates: TemplateRegistry
    var vnodes: VNodeStore
    var handlers: HandlerRegistry
    var memos: MemoStore
    var effects: EffectStore
    var current_context: Int  # -1 = no active context
    var current_scope: Int  # -1 = no active scope (for hooks)
    var dirty_scopes: List[UInt32]
    # Context-to-memo mapping: parallel arrays.
    # When a reactive context is notified (i.e. appears in a signal's
    # subscriber list after a write), the runtime checks whether that
    # context belongs to a memo.  If so, the memo is marked dirty and
    # the memo's output signal's subscribers are also notified.
    var _memo_ctx_ids: List[UInt32]  # context IDs that belong to memos
    var _memo_ids: List[UInt32]  # corresponding memo IDs
    # Context-to-effect mapping: parallel arrays.
    # When a reactive context is notified after a signal write, the
    # runtime checks whether that context belongs to an effect.  If so,
    # the effect is marked pending (but NOT added to dirty_scopes —
    # effects run after rendering, not during).
    var _effect_ctx_ids: List[UInt32]  # context IDs that belong to effects
    var _effect_ids: List[UInt32]  # corresponding effect IDs
    # Changed-signals tracking for equality-gated memo propagation (Phase 37).
    # Accumulates signal keys whose values actually changed during a flush
    # cycle.  Populated by write_signal (source signals always change) and
    # memo_end_compute_* (only when new != old).  Used by settle_scopes()
    # to remove scopes whose subscribed signals are all value-stable.
    var _changed_signals: List[UInt32]
    # Batch signal writes (Phase 38).
    # When _batch_depth > 0, write_signal stores values immediately but
    # defers subscriber scanning and worklist propagation until the
    # outermost end_batch().  _batch_keys collects the signal keys
    # written during the batch (deduplicated).
    var _batch_depth: Int
    var _batch_keys: List[UInt32]

    # ── Construction ─────────────────────────────────────────────────

    def __init__(out self):
        self.signals = SignalStore()
        self.strings = StringStore()
        self.scopes = ScopeArena()
        self.templates = TemplateRegistry()
        self.vnodes = VNodeStore()
        self.handlers = HandlerRegistry()
        self.memos = MemoStore()
        self.effects = EffectStore()
        self.current_context = -1
        self.current_scope = -1
        self.dirty_scopes = List[UInt32]()
        self._memo_ctx_ids = List[UInt32]()
        self._memo_ids = List[UInt32]()
        self._effect_ctx_ids = List[UInt32]()
        self._effect_ids = List[UInt32]()
        self._changed_signals = List[UInt32]()
        self._batch_depth = 0
        self._batch_keys = List[UInt32]()

    def __init__(out self, *, deinit move: Self):
        self.signals = move.signals^
        self.strings = move.strings^
        self.scopes = move.scopes^
        self.templates = move.templates^
        self.vnodes = move.vnodes^
        self.handlers = move.handlers^
        self.memos = move.memos^
        self.effects = move.effects^
        self.current_context = move.current_context
        self.current_scope = move.current_scope
        self.dirty_scopes = move.dirty_scopes^
        self._memo_ctx_ids = move._memo_ctx_ids^
        self._memo_ids = move._memo_ids^
        self._effect_ctx_ids = move._effect_ctx_ids^
        self._effect_ids = move._effect_ids^
        self._changed_signals = move._changed_signals^
        self._batch_depth = move._batch_depth
        self._batch_keys = move._batch_keys^

    # ── Context management ───────────────────────────────────────────

    @always_inline
    def has_context(self) -> Bool:
        """Check whether a reactive context is currently active."""
        return self.current_context != -1

    @always_inline
    def get_context(self) -> UInt32:
        """Return the current reactive context ID.

        Precondition: `has_context()` is True.
        """
        return UInt32(self.current_context)

    @always_inline
    def set_context(mut self, context_id: UInt32):
        """Set the current reactive context."""
        self.current_context = Int(context_id)

    @always_inline
    def clear_context(mut self):
        """Clear the current reactive context."""
        self.current_context = -1

    def push_context(mut self, context_id: UInt32) -> Int:
        """Push a new context, returning the previous one (for restore).

        Usage:
            var prev = runtime.push_context(my_ctx)
            # ... reads subscribe to my_ctx ...
            runtime.restore_context(prev)
        """
        var prev = self.current_context
        self.current_context = Int(context_id)
        return prev

    def restore_context(mut self, prev: Int):
        """Restore a previously saved context."""
        self.current_context = prev

    # ── Signal operations (convenience wrappers) ─────────────────────

    def create_signal[
        T: Copyable & Deinitable 
    ](mut self, initial: T) -> UInt32:
        """Create a signal and return its key."""
        return self.signals.create[T](initial)

    def read_signal[T: Copyable ](mut self, key: UInt32) -> T:
        """Read a signal, auto-subscribing the current context if any."""
        if self.has_context():
            return self.signals.read_tracked[T](key, self.get_context())
        return self.signals.read[T](key)

    def write_signal[
        T: Copyable & Deinitable 
    ](mut self, key: UInt32, value: T):
        """Write a signal and propagate dirtiness through memo chains.

        If currently inside a batch (Phase 38), the value is stored
        immediately but propagation is deferred until end_batch().

        Two-phase propagation (Phase 36):

        Phase 1 — Scan direct subscribers of the written signal:
          - Memo subscriber → mark dirty, add to worklist.
          - Effect subscriber → mark pending.
          - Scope subscriber → add to dirty_scopes.

        Phase 2 — Drain worklist to propagate through memo → memo
        chains to arbitrary depth:
          - For each memo in the worklist, scan its output signal's
            subscribers and apply the same memo/effect/scope logic.
          - The is_dirty() check serves as a cycle guard: a memo
            already dirty is not re-added to the worklist, guaranteeing
            termination (each memo processed at most once per call).
        """
        self.signals.write[T](key, value)

        if self._batch_depth > 0:
            # Batch mode: track the key, skip propagation
            var already = False
            for i in range(len(self._batch_keys)):
                if self._batch_keys[i] == key:
                    already = True
                    break
            if not already:
                self._batch_keys.append(key)
            return

        # Non-batch: immediate propagation
        self._changed_signals.append(key)

        var memo_worklist = List[UInt32]()
        var subs = self.signals.get_subscribers(key)

        # Phase 1: direct subscribers of the written signal
        for i in range(len(subs)):
            var ctx = subs[i]
            # 1. Check for scope tag (SCOPE_CONTEXT_TAG) first — scope
            #    IDs are tagged to avoid collisions with signal keys
            #    used as memo/effect context IDs.
            if (ctx & SCOPE_CONTEXT_TAG) != 0:
                var scope_id = ctx & ~SCOPE_CONTEXT_TAG
                var found = False
                for j in range(len(self.dirty_scopes)):
                    if self.dirty_scopes[j] == scope_id:
                        found = True
                        break
                if not found:
                    self.dirty_scopes.append(scope_id)
                continue
            # 2. Check if this subscriber is a memo's reactive context
            var is_memo = False
            for m in range(len(self._memo_ctx_ids)):
                if self._memo_ctx_ids[m] == ctx:
                    var memo_id = self._memo_ids[m]
                    if self.memos.contains(memo_id):
                        if not self.memos.is_dirty(memo_id):
                            self.memos.mark_dirty(memo_id)
                            memo_worklist.append(memo_id)
                    is_memo = True
                    break
            if is_memo:
                continue
            # 3. Check if this subscriber is an effect's reactive context
            var is_effect = False
            for e in range(len(self._effect_ctx_ids)):
                if self._effect_ctx_ids[e] == ctx:
                    var effect_id = self._effect_ids[e]
                    if self.effects.contains(effect_id):
                        self.effects.mark_pending(effect_id)
                    is_effect = True
                    break
            if is_effect:
                continue
            # 4. Untagged, unknown subscriber — treat as scope (legacy)
            var found = False
            for j in range(len(self.dirty_scopes)):
                if self.dirty_scopes[j] == ctx:
                    found = True
                    break
            if not found:
                self.dirty_scopes.append(ctx)

        # Phase 2: drain worklist — propagate through memo → memo chains
        # Use index-based iteration instead of pop() to avoid potential
        # issues with List[UInt32].pop() semantics.  The list may grow
        # during iteration as downstream memos are discovered.
        var wl_idx = 0
        while wl_idx < len(memo_worklist):
            var mid = memo_worklist[wl_idx]
            wl_idx += 1
            if not self.memos.contains(mid):
                continue
            var out_key = self.memos.output_key(mid)
            var out_subs = self.signals.get_subscribers(out_key)
            for k in range(len(out_subs)):
                var sub_ctx = out_subs[k]
                # Check for scope tag first
                if (sub_ctx & SCOPE_CONTEXT_TAG) != 0:
                    var scope_id = sub_ctx & ~SCOPE_CONTEXT_TAG
                    var found2 = False
                    for j2 in range(len(self.dirty_scopes)):
                        if self.dirty_scopes[j2] == scope_id:
                            found2 = True
                            break
                    if not found2:
                        self.dirty_scopes.append(scope_id)
                    continue
                # Check if this subscriber is a memo's reactive context
                var is_memo2 = False
                for m2 in range(len(self._memo_ctx_ids)):
                    if self._memo_ctx_ids[m2] == sub_ctx:
                        var downstream_id = self._memo_ids[m2]
                        if self.memos.contains(downstream_id):
                            if not self.memos.is_dirty(downstream_id):
                                self.memos.mark_dirty(downstream_id)
                                memo_worklist.append(downstream_id)
                        is_memo2 = True
                        break
                if is_memo2:
                    continue
                # Check if this subscriber is an effect's reactive context
                var is_eff = False
                for e2 in range(len(self._effect_ctx_ids)):
                    if self._effect_ctx_ids[e2] == sub_ctx:
                        var eff_id = self._effect_ids[e2]
                        if self.effects.contains(eff_id):
                            self.effects.mark_pending(eff_id)
                        is_eff = True
                        break
                if is_eff:
                    continue
                # Untagged, unknown subscriber — treat as scope (legacy)
                var found2 = False
                for j2 in range(len(self.dirty_scopes)):
                    if self.dirty_scopes[j2] == sub_ctx:
                        found2 = True
                        break
                if not found2:
                    self.dirty_scopes.append(sub_ctx)

    def peek_signal[T: Copyable ](self, key: UInt32) -> T:
        """Read a signal without subscribing."""
        return self.signals.peek[T](key)

    def destroy_signal(mut self, key: UInt32):
        """Destroy a signal, cleaning up subscribers."""
        self.signals.destroy(key)

    # ── String signal operations ─────────────────────────────────────
    #
    # A "string signal" is a pair: (string_key in StringStore,
    # version_key in SignalStore).  The version signal provides
    # subscriber tracking and dirty-marking; the StringStore provides
    # safe heap-string storage.

    def create_signal_string(
        mut self, initial: String
    ) -> Tuple[UInt32, UInt32]:
        """Create a string signal and return (string_key, version_key).

        The version signal starts at 0; it is bumped on every write.
        Subscribers attach to the version signal and get notified when
        the string changes.

        Args:
            initial: The initial string value.

        Returns:
            A tuple of (string_key, version_key).
        """
        var string_key = self.strings.create(initial)
        var version_key = self.signals.create[Int32](Int32(0))
        return Tuple(string_key, version_key)

    def peek_signal_string(self, string_key: UInt32) -> String:
        """Read a string signal's value WITHOUT subscribing.

        Args:
            string_key: The key in the StringStore.

        Returns:
            A copy of the current string value.
        """
        return self.strings.read(string_key)

    def read_signal_string(
        mut self, string_key: UInt32, version_key: UInt32
    ) -> String:
        """Read a string signal's value AND subscribe the current context.

        Subscribes via the companion version signal so the reading
        context is marked dirty when the string changes.

        Args:
            string_key: The key in the StringStore.
            version_key: The companion version signal key.

        Returns:
            A copy of the current string value.
        """
        # Subscribe the current context to the version signal
        _ = self.read_signal[Int32](version_key)
        return self.strings.read(string_key)

    def write_signal_string(
        mut self, string_key: UInt32, version_key: UInt32, value: String
    ):
        """Write a new string value and notify subscribers.

        Updates the StringStore entry and bumps the version signal,
        which marks all subscribers dirty via the existing mechanism.

        If currently inside a batch (Phase 38), the string value and
        version are stored immediately but propagation is deferred
        until end_batch().

        Args:
            string_key: The key in the StringStore.
            version_key: The companion version signal key.
            value: The new string value.
        """
        self.strings.write(string_key, value)

        if self._batch_depth > 0:
            # Batch mode: bump version directly (no propagation),
            # track version_key for deferred propagation
            var ver = self.peek_signal[Int32](version_key)
            self.signals.write[Int32](version_key, ver + 1)
            var already = False
            for i in range(len(self._batch_keys)):
                if self._batch_keys[i] == version_key:
                    already = True
                    break
            if not already:
                self._batch_keys.append(version_key)
            return

        # Non-batch: bump the version signal — triggers subscriber notification
        var ver = self.peek_signal[Int32](version_key)
        self.write_signal[Int32](version_key, ver + 1)

    def destroy_signal_string(
        mut self, string_key: UInt32, version_key: UInt32
    ):
        """Destroy a string signal, freeing both the string and version.

        Args:
            string_key: The key in the StringStore.
            version_key: The companion version signal key.
        """
        self.strings.destroy(string_key)
        self.signals.destroy(version_key)

    def string_signal_count(self) -> Int:
        """Return the number of live string signals in the StringStore."""
        return self.strings.count()

    # ── Hook-based string signal creation ────────────────────────────

    def use_signal_string(mut self, initial: String) -> Tuple[UInt32, UInt32]:
        """Hook: create or retrieve a string signal for the current scope.

        On first render: creates a new string signal (string_key +
        version_key), stores both keys in the scope's hook array
        (two HOOK_SIGNAL entries), and returns them.

        On re-render: retrieves the existing keys from the hook array
        and returns them (initial value is ignored).

        Precondition: `has_scope()` is True.

        Args:
            initial: The initial string value (first render only).

        Returns:
            A tuple of (string_key, version_key).
        """
        var scope_id = self.get_scope()
        if self.scopes.is_first_render(scope_id):
            # First render — create string signal and store in hooks
            var keys = self.create_signal_string(initial)
            # Store string_key as a HOOK_SIGNAL entry
            self.scopes.push_hook(scope_id, HOOK_SIGNAL, keys[0])
            # Store version_key as a HOOK_SIGNAL entry
            self.scopes.push_hook(scope_id, HOOK_SIGNAL, keys[1])
            return keys
        else:
            # Re-render — return existing keys
            var string_key = self.scopes.next_hook(scope_id)
            var version_key = self.scopes.next_hook(scope_id)
            return Tuple(string_key, version_key)

    # ── Dirty queue ──────────────────────────────────────────────────

    def drain_dirty(mut self) -> List[UInt32]:
        """Return and clear the dirty-scope queue."""
        var result = self.dirty_scopes^
        self.dirty_scopes = List[UInt32]()
        return result^

    def has_dirty(self) -> Bool:
        """Check whether any scopes need re-rendering."""
        return len(self.dirty_scopes) > 0

    def dirty_count(self) -> Int:
        """Number of scopes in the dirty queue."""
        return len(self.dirty_scopes)

    def mark_scope_dirty(mut self, scope_id: UInt32):
        """Manually add a scope to the dirty queue.

        Used by non-signal state changes (e.g. router navigation triggered
        by an external JS call) that need to trigger a re-render without
        going through the reactive signal system.

        Deduplicates — if the scope is already in the dirty queue, this
        is a no-op.

        Args:
            scope_id: The scope to mark as needing re-render.
        """
        var found = False
        for i in range(len(self.dirty_scopes)):
            if self.dirty_scopes[i] == scope_id:
                found = True
                break
        if not found:
            self.dirty_scopes.append(scope_id)

    # ── Changed-signals tracking (Phase 37) ──────────────────────────

    def signal_changed_this_cycle(self, key: UInt32) -> Bool:
        """Check whether a signal was written with a new value this flush cycle.

        Returns True if `key` appears in `_changed_signals`, meaning
        either `write_signal` wrote it (source signal) or a memo's
        `end_compute` wrote a changed value to it (memo output signal).
        """
        for i in range(len(self._changed_signals)):
            if self._changed_signals[i] == key:
                return True
        return False

    def clear_changed_signals(mut self):
        """Reset the changed-signals set.

        Called at the end of `settle_scopes()` to prepare for the next
        flush cycle.  Can also be called manually in tests.
        """
        self._changed_signals = List[UInt32]()

    # ── Batch signal writes (Phase 38) ───────────────────────────────

    def begin_batch(mut self):
        """Enter batch mode.  Signal writes store values but defer propagation.

        Can be nested — only the outermost `end_batch()` triggers
        propagation.
        """
        self._batch_depth += 1

    def end_batch(mut self):
        """Exit batch mode.  On the outermost call, run a single combined
        propagation pass over all signals written during the batch.

        Decrements the batch depth.  If the depth reaches 0, runs
        propagation for all keys in `_batch_keys` using a single
        shared worklist — a memo that subscribes to multiple batched
        signals is added to the worklist at most once.
        """
        if self._batch_depth <= 0:
            return  # not in a batch — no-op
        self._batch_depth -= 1
        if self._batch_depth > 0:
            return  # still in a nested batch — defer

        # Outermost end_batch: propagate all batched keys
        if len(self._batch_keys) == 0:
            return  # empty batch — no-op

        # Combined propagation: shared worklist across all source signals
        var memo_worklist = List[UInt32]()

        for b in range(len(self._batch_keys)):
            var key = self._batch_keys[b]
            self._changed_signals.append(key)
            var subs = self.signals.get_subscribers(key)

            # Phase 1: direct subscribers (same logic as write_signal)
            for i in range(len(subs)):
                var ctx = subs[i]
                if (ctx & SCOPE_CONTEXT_TAG) != 0:
                    var scope_id = ctx & ~SCOPE_CONTEXT_TAG
                    var found = False
                    for j in range(len(self.dirty_scopes)):
                        if self.dirty_scopes[j] == scope_id:
                            found = True
                            break
                    if not found:
                        self.dirty_scopes.append(scope_id)
                    continue
                # Memo check
                var is_memo = False
                for m in range(len(self._memo_ctx_ids)):
                    if self._memo_ctx_ids[m] == ctx:
                        var memo_id = self._memo_ids[m]
                        if self.memos.contains(memo_id):
                            if not self.memos.is_dirty(memo_id):
                                self.memos.mark_dirty(memo_id)
                                memo_worklist.append(memo_id)
                        is_memo = True
                        break
                if is_memo:
                    continue
                # Effect check
                var is_effect = False
                for e in range(len(self._effect_ctx_ids)):
                    if self._effect_ctx_ids[e] == ctx:
                        var effect_id = self._effect_ids[e]
                        if self.effects.contains(effect_id):
                            self.effects.mark_pending(effect_id)
                        is_effect = True
                        break
                if is_effect:
                    continue
                # Unknown: treat as scope (legacy)
                var found = False
                for j in range(len(self.dirty_scopes)):
                    if self.dirty_scopes[j] == ctx:
                        found = True
                        break
                if not found:
                    self.dirty_scopes.append(ctx)

        # Phase 2: drain shared worklist (same as write_signal Phase 2)
        var wl_idx = 0
        while wl_idx < len(memo_worklist):
            var mid = memo_worklist[wl_idx]
            wl_idx += 1
            if not self.memos.contains(mid):
                continue
            var out_key = self.memos.output_key(mid)
            var out_subs = self.signals.get_subscribers(out_key)
            for k in range(len(out_subs)):
                var sub_ctx = out_subs[k]
                if (sub_ctx & SCOPE_CONTEXT_TAG) != 0:
                    var scope_id = sub_ctx & ~SCOPE_CONTEXT_TAG
                    var found2 = False
                    for j2 in range(len(self.dirty_scopes)):
                        if self.dirty_scopes[j2] == scope_id:
                            found2 = True
                            break
                    if not found2:
                        self.dirty_scopes.append(scope_id)
                    continue
                var is_memo2 = False
                for m2 in range(len(self._memo_ctx_ids)):
                    if self._memo_ctx_ids[m2] == sub_ctx:
                        var downstream_id = self._memo_ids[m2]
                        if self.memos.contains(downstream_id):
                            if not self.memos.is_dirty(downstream_id):
                                self.memos.mark_dirty(downstream_id)
                                memo_worklist.append(downstream_id)
                        is_memo2 = True
                        break
                if is_memo2:
                    continue
                var is_eff = False
                for e2 in range(len(self._effect_ctx_ids)):
                    if self._effect_ctx_ids[e2] == sub_ctx:
                        var eff_id = self._effect_ids[e2]
                        if self.effects.contains(eff_id):
                            self.effects.mark_pending(eff_id)
                        is_eff = True
                        break
                if is_eff:
                    continue
                var found2 = False
                for j2 in range(len(self.dirty_scopes)):
                    if self.dirty_scopes[j2] == sub_ctx:
                        found2 = True
                        break
                if not found2:
                    self.dirty_scopes.append(sub_ctx)

        # Clear batch state
        self._batch_keys = List[UInt32]()

    def is_batching(self) -> Bool:
        """Return True if currently inside a begin_batch/end_batch bracket."""
        return self._batch_depth > 0

    def settle_scopes(mut self):
        """Remove dirty scopes whose subscribed signals all have unchanged values.

        After memo recomputation, some scopes may have been eagerly marked
        dirty (Phase 36) but none of their subscribed signals actually
        changed value.  This method checks each dirty scope and removes
        those where no subscribed signal is in `_changed_signals`.

        Call after `run_memos()` and before `render()` to skip unnecessary
        re-renders.

        Algorithm: scan `_changed_signals` and collect scope IDs that
        subscribe to at least one changed signal.  Replace `dirty_scopes`
        with only those scopes.  This is O(C × avg_subscribers × D) where
        C = changed signals (small), avg_subscribers ≈ 1–3, D = dirty
        scopes (small).  Much more efficient than scanning all signals.
        """
        if len(self._changed_signals) == 0:
            # No signals changed at all → all scopes are settled
            self.dirty_scopes = List[UInt32]()
            self._changed_signals = List[UInt32]()
            return

        if len(self.dirty_scopes) == 0:
            # No dirty scopes to settle
            self._changed_signals = List[UInt32]()
            return

        # Build set of scopes that subscribe to at least one changed signal
        var keep = List[UInt32]()
        for i in range(len(self._changed_signals)):
            var sig_key = self._changed_signals[i]
            if Int(sig_key) >= len(self.signals._entries):
                continue
            if not self.signals._states[Int(sig_key)].occupied:
                continue
            var num_subs = len(self.signals._entries[Int(sig_key)].subscribers)
            for k in range(num_subs):
                var ctx = self.signals._entries[Int(sig_key)].subscribers[k]
                if (ctx & SCOPE_CONTEXT_TAG) != 0:
                    var scope_id = ctx & ~SCOPE_CONTEXT_TAG
                    # Check if this scope is in dirty_scopes
                    for d in range(len(self.dirty_scopes)):
                        if self.dirty_scopes[d] == scope_id:
                            # Keep this scope dirty — deduplicate
                            var already = False
                            for j in range(len(keep)):
                                if keep[j] == scope_id:
                                    already = True
                                    break
                            if not already:
                                keep.append(scope_id)
                            break
        self.dirty_scopes = keep^
        self._changed_signals = List[UInt32]()

    def memo_did_value_change(self, memo_id: UInt32) -> Bool:
        """Check whether the last end_compute changed the memo's value.

        Forwarding method for WASM exports and tests.
        """
        return self.memos.did_value_change(memo_id)

    # ── Scope management ─────────────────────────────────────────────

    def create_scope(mut self, height: UInt32, parent_id: Int) -> UInt32:
        """Create a new scope and return its ID."""
        return self.scopes.create(height, parent_id)

    def create_child_scope(mut self, parent_id: UInt32) -> UInt32:
        """Create a child scope whose height is parent.height + 1."""
        return self.scopes.create_child(parent_id)

    def destroy_scope(mut self, id: UInt32):
        """Destroy a scope, freeing its slot for reuse."""
        self.scopes.destroy(id)

    def scope_count(self) -> Int:
        """Return the number of live scopes."""
        return self.scopes.count()

    def scope_contains(self, id: UInt32) -> Bool:
        """Check whether `id` is a live scope."""
        return self.scopes.contains(id)

    # ── Scope rendering ──────────────────────────────────────────────

    @always_inline
    def has_scope(self) -> Bool:
        """Check whether a scope is currently active (being rendered)."""
        return self.current_scope != -1

    @always_inline
    def get_scope(self) -> UInt32:
        """Return the current scope ID.

        Precondition: `has_scope()` is True.
        """
        return UInt32(self.current_scope)

    def begin_scope_render(mut self, scope_id: UInt32) -> Int:
        """Begin rendering a scope.

        Sets the current scope, begins the render pass on the scope state,
        and sets the reactive context to the scope ID tagged with
        SCOPE_CONTEXT_TAG (so signal reads during rendering are tracked
        and the subscriber can be distinguished from memo/effect contexts).

        Returns the previous scope ID (as Int, -1 if none) for restoration.
        """
        var prev_scope = self.current_scope
        self.current_scope = Int(scope_id)
        self.scopes.begin_render(scope_id)
        # Tag the scope ID so it can be distinguished from memo/effect
        # context IDs (which are bare signal keys) in write_signal.
        self.set_context(scope_id | SCOPE_CONTEXT_TAG)
        return prev_scope

    def end_scope_render(mut self, prev_scope: Int):
        """End rendering the current scope and restore the previous scope.

        Clears the reactive context and restores the previous scope/context.
        """
        self.current_scope = prev_scope
        if prev_scope == -1:
            self.clear_context()
        else:
            self.set_context(UInt32(prev_scope) | SCOPE_CONTEXT_TAG)

    # ── Hook-based signal creation ───────────────────────────────────

    def use_signal_i32(mut self, initial: Int32) -> UInt32:
        """Hook: create or retrieve an Int32 signal for the current scope.

        On first render: creates a new signal with `initial`, stores its
        key in the scope's hook array, and returns the key.

        On re-render: retrieves the existing signal key from the hook array
        (initial value is ignored) and returns it.

        Precondition: `has_scope()` is True.
        """
        var scope_id = self.get_scope()
        if self.scopes.is_first_render(scope_id):
            # First render — create signal and store in hooks
            var key = self.signals.create[Int32](initial)
            self.scopes.push_hook(scope_id, HOOK_SIGNAL, key)
            return key
        else:
            # Re-render — return existing signal key
            return self.scopes.next_hook(scope_id)

    def use_effect(mut self) -> UInt32:
        """Hook: create or retrieve an effect for the current scope.

        Follows the same pattern as use_signal_i32 and use_memo_i32:
          - First render: creates an effect via create_effect, pushes
            HOOK_EFFECT tag + effect ID onto the scope's hook list.
          - Re-render: advances the hook cursor and returns the existing
            effect ID.

        Precondition: current_scope is set (inside a begin/end render).
        """
        var scope_id = self.get_scope()
        if self.scopes.is_first_render(scope_id):
            # First render — create new effect
            var effect_id = self.create_effect(scope_id)
            self.scopes.push_hook(scope_id, HOOK_EFFECT, UInt32(effect_id))
            return UInt32(effect_id)
        else:
            # Re-render — return existing effect ID
            return self.scopes.next_hook(scope_id)

    def use_memo_i32(mut self, initial: Int32) -> UInt32:
        """Hook: create or retrieve an Int32 memo for the current scope.

        On first render: creates a new memo, stores its ID in the scope's
        hook array (with HOOK_MEMO tag), and returns the memo ID.

        On re-render: retrieves the existing memo ID from the hook array
        and returns it (initial value is ignored).

        Precondition: has_scope() is True.
        """
        var scope_id = self.get_scope()
        if self.scopes.is_first_render(scope_id):
            # First render — create memo and store in hooks
            var memo_id = self.create_memo_i32(scope_id, initial)
            self.scopes.push_hook(scope_id, HOOK_MEMO, memo_id)
            return memo_id
        else:
            # Re-render — return existing memo ID
            return self.scopes.next_hook(scope_id)

    def use_memo_bool(mut self, initial: Bool) -> UInt32:
        """Hook: create or retrieve a Bool memo for the current scope.

        On first render: creates a new memo, stores its ID in the scope's
        hook array (with HOOK_MEMO tag), and returns the memo ID.

        On re-render: retrieves the existing memo ID from the hook array
        and returns it (initial value is ignored).

        Precondition: has_scope() is True.
        """
        var scope_id = self.get_scope()
        if self.scopes.is_first_render(scope_id):
            var memo_id = self.create_memo_bool(scope_id, initial)
            self.scopes.push_hook(scope_id, HOOK_MEMO, memo_id)
            return memo_id
        else:
            return self.scopes.next_hook(scope_id)

    def use_memo_string(mut self, initial: String) -> UInt32:
        """Hook: create or retrieve a String memo for the current scope.

        On first render: creates a new memo, stores its ID in the scope's
        hook array (with HOOK_MEMO tag), and returns the memo ID.

        On re-render: retrieves the existing memo ID from the hook array
        and returns it (initial value is ignored).

        Precondition: has_scope() is True.
        """
        var scope_id = self.get_scope()
        if self.scopes.is_first_render(scope_id):
            var memo_id = self.create_memo_string(scope_id, initial)
            self.scopes.push_hook(scope_id, HOOK_MEMO, memo_id)
            return memo_id
        else:
            return self.scopes.next_hook(scope_id)

    # ── Event handler management ─────────────────────────────────────

    def register_handler(mut self, entry: HandlerEntry) -> UInt32:
        """Register an event handler and return its stable ID.

        The handler ID is used in AVAL_EVENT attribute values and by
        the JS EventBridge to dispatch events back to WASM.
        """
        return self.handlers.register(entry)

    def remove_handler(mut self, id: UInt32):
        """Remove an event handler by ID."""
        self.handlers.remove(id)

    def dispatch_event(mut self, handler_id: UInt32, event_type: UInt8) -> Bool:
        """Dispatch an event to the handler at `handler_id`.

        Executes the handler's action (e.g. signal write) and returns
        True if the action was executed, False if the handler was not
        found or is a no-op.

        After dispatching, affected scopes will be in the dirty queue
        (via signal write → subscriber notification).

        Args:
            handler_id: The handler to invoke.
            event_type: The DOM event type tag (EVT_CLICK, etc.).

        Returns:
            True if an action was executed, False otherwise.
        """
        if not self.handlers.contains(handler_id):
            return False

        var entry = self.handlers.get(handler_id)
        var action = entry.action

        if action == ACTION_NONE:
            # No-op handler — just mark the scope dirty directly
            var found = False
            for j in range(len(self.dirty_scopes)):
                if self.dirty_scopes[j] == entry.scope_id:
                    found = True
                    break
            if not found:
                self.dirty_scopes.append(entry.scope_id)
            return False

        elif action == ACTION_SIGNAL_SET_I32:
            self.write_signal[Int32](entry.signal_key, entry.operand)
            return True

        elif action == ACTION_SIGNAL_ADD_I32:
            var current = self.peek_signal[Int32](entry.signal_key)
            self.write_signal[Int32](entry.signal_key, current + entry.operand)
            return True

        elif action == ACTION_SIGNAL_SUB_I32:
            var current = self.peek_signal[Int32](entry.signal_key)
            self.write_signal[Int32](entry.signal_key, current - entry.operand)
            return True

        elif action == ACTION_SIGNAL_TOGGLE:
            var current = self.peek_signal[Int32](entry.signal_key)
            if current == 0:
                self.write_signal[Int32](entry.signal_key, Int32(1))
            else:
                self.write_signal[Int32](entry.signal_key, Int32(0))
            return True

        elif action == ACTION_CUSTOM:
            # Custom handlers are handled by JS — just mark scope dirty
            var found = False
            for j in range(len(self.dirty_scopes)):
                if self.dirty_scopes[j] == entry.scope_id:
                    found = True
                    break
            if not found:
                self.dirty_scopes.append(entry.scope_id)
            return False

        return False

    def dispatch_event_with_i32(
        mut self, handler_id: UInt32, event_type: UInt8, value: Int32
    ) -> Bool:
        """Dispatch an event with an Int32 payload (e.g. from input).

        For ACTION_SIGNAL_SET_INPUT, the payload is used as the new signal
        value instead of the handler's operand.  For other actions, this
        falls back to the normal dispatch.

        Args:
            handler_id: The handler to invoke.
            event_type: The DOM event type tag.
            value: The Int32 payload from the event.

        Returns:
            True if an action was executed, False otherwise.
        """
        if not self.handlers.contains(handler_id):
            return False

        var entry = self.handlers.get(handler_id)

        if entry.action == ACTION_SIGNAL_SET_INPUT:
            self.write_signal[Int32](entry.signal_key, value)
            return True

        # Fall back to normal dispatch for other action types
        return self.dispatch_event(handler_id, event_type)

    def dispatch_event_with_string(
        mut self, handler_id: UInt32, event_type: UInt8, value: String
    ) -> Bool:
        """Dispatch an event with a String payload (e.g. from input).

        Phase 20: For ACTION_SIGNAL_SET_STRING, the payload is written
        to the SignalString identified by the handler's signal_key
        (string_key) and operand (version_key).  For other actions,
        this falls back to the normal dispatch.

        The string value is passed from JS via writeStringStruct() into
        WASM linear memory; the @export wrapper reads it as a Mojo
        String and forwards it here.

        Args:
            handler_id: The handler to invoke.
            event_type: The DOM event type tag.
            value: The String payload from the event (e.g. input.value).

        Returns:
            True if an action was executed, False otherwise.
        """
        if not self.handlers.contains(handler_id):
            return False

        var entry = self.handlers.get(handler_id)

        if entry.action == ACTION_SIGNAL_SET_STRING:
            # signal_key holds the string_key, operand holds the version_key
            self.write_signal_string(
                entry.signal_key, UInt32(entry.operand), value
            )
            return True

        if entry.action == ACTION_KEY_ENTER_CUSTOM:
            # Phase 22: Only fire when the key string is "Enter".
            # Mark scope dirty (same as ACTION_CUSTOM) so the app's
            # handle_event() can perform custom routing.
            if value == String("Enter"):
                var found = False
                for j in range(len(self.dirty_scopes)):
                    if self.dirty_scopes[j] == entry.scope_id:
                        found = True
                        break
                if not found:
                    self.dirty_scopes.append(entry.scope_id)
                return True
            return False

        # Fall back to normal dispatch for other action types
        return self.dispatch_event(handler_id, event_type)

    def handler_count(self) -> Int:
        """Return the number of live event handlers."""
        return self.handlers.count()

    # ── Memo operations ──────────────────────────────────────────────

    def create_memo_i32(mut self, scope_id: UInt32, initial: Int32) -> UInt32:
        """Create a memo with an initial cached value.

        Allocates a reactive context (a scope-level context ID via a
        dedicated signal used only for tracking) and an output signal.
        The memo starts dirty so the first read triggers a computation.

        Returns the memo ID (not the output signal key).
        """
        # Allocate a "context signal" whose sole purpose is to act as a
        # reactive-context identifier.  We use a dummy Int32 signal whose
        # key doubles as the context_id.
        var context_id = self.signals.create[Int32](Int32(0))
        # Allocate the output signal that stores the cached result.
        var output_key = self.signals.create[Int32](initial)
        var memo_id = self.memos.create(context_id, output_key, scope_id)
        # Register the context→memo mapping for dirty propagation.
        self._memo_ctx_ids.append(context_id)
        self._memo_ids.append(memo_id)
        return memo_id

    def memo_begin_compute(mut self, memo_id: UInt32):
        """Begin memo computation.

        Sets the memo's reactive context as the current context so that
        signal reads during computation are tracked as dependencies.
        Clears old subscriptions (re-subscribes fresh on each compute).
        """
        if not self.memos.contains(memo_id):
            return
        var entry = self.memos.get(memo_id)
        self.memos.set_computing(memo_id, True)
        # Clear old subscriptions: unsubscribe this context from all signals
        # it was previously subscribed to.  We do a full scan of signals —
        # acceptable for now since memo count is small.
        for i in range(len(self.signals._entries)):
            if (
                i < len(self.signals._states)
                and self.signals._states[i].occupied
            ):
                self.signals.unsubscribe(UInt32(i), entry.context_id)
        # Push the memo's context as the current reactive context
        # (saves the previous context for restore in end_compute).
        # We store the previous context in a simple way: use the
        # context signal's value as storage for the previous context.
        var prev = self.current_context
        self.signals.write[Int32](entry.context_id, Int32(prev))
        self.current_context = Int(entry.context_id)

    def memo_end_compute_i32(mut self, memo_id: UInt32, value: Int32):
        """End memo computation and store the result.

        Writes the computed value to the memo's output signal and clears
        the dirty flag.  Restores the previous reactive context.

        Phase 37 equality gate: compares old vs new value before writing.
        If the value is unchanged (value-stable), the output signal is
        NOT written, so downstream memos that read it will see the same
        value and can themselves be value-stable.  The `value_changed`
        flag records whether the value actually changed; `_changed_signals`
        is updated only when it did.
        """
        if not self.memos.contains(memo_id):
            return
        var entry = self.memos.get(memo_id)
        # Equality check: compare old cached value vs new computed value
        var old_value = self.signals.read[Int32](entry.output_key)
        var changed = old_value != value
        if changed:
            # Value changed — write to output signal (does NOT trigger
            # dirty propagation through write_signal — we write directly
            # to avoid recursive memo updates during computation).
            self.signals.write[Int32](entry.output_key, value)
            self._changed_signals.append(entry.output_key)
        # Record whether this recomputation produced a new value
        self.memos.set_value_changed(memo_id, changed)
        self.memos.clear_dirty(memo_id)
        self.memos.set_computing(memo_id, False)
        # Restore previous context
        var prev = Int(self.signals.read[Int32](entry.context_id))
        self.current_context = prev

    def memo_read_i32(mut self, memo_id: UInt32) -> Int32:
        """Read the memo's cached value.

        Subscribes the current reactive context (if any) to the memo's
        output signal, so the reader is notified when the memo recomputes.

        Does NOT trigger recomputation — the caller must check
        memo_is_dirty() and call begin/end_compute if needed.
        """
        if not self.memos.contains(memo_id):
            return Int32(0)
        var entry = self.memos.get(memo_id)
        # Subscribe the current context to the memo's output signal
        if self.has_context():
            self.signals._entries[Int(entry.output_key)].subscribe(
                self.get_context()
            )
        return self.signals.read[Int32](entry.output_key)

    def memo_is_dirty(self, memo_id: UInt32) -> Bool:
        """Check whether the memo needs recomputation."""
        if not self.memos.contains(memo_id):
            return False
        return self.memos.is_dirty(memo_id)

    def destroy_memo(mut self, memo_id: UInt32):
        """Destroy a memo, cleaning up its context and output signal.

        Removes the context→memo mapping, destroys the context signal
        and output signal, and frees the memo slot.  For string memos,
        also destroys the StringStore entry.
        """
        if not self.memos.contains(memo_id):
            return
        var entry = self.memos.get(memo_id)
        # Remove context→memo mapping
        for i in range(len(self._memo_ctx_ids)):
            if self._memo_ctx_ids[i] == entry.context_id:
                # Swap-remove for O(1)
                var last = len(self._memo_ctx_ids) - 1
                if i != last:
                    self._memo_ctx_ids[i] = self._memo_ctx_ids[last]
                    self._memo_ids[i] = self._memo_ids[last]
                _ = self._memo_ctx_ids.pop()
                _ = self._memo_ids.pop()
                break
        # Destroy the context signal and output signal
        self.signals.destroy(entry.context_id)
        self.signals.destroy(entry.output_key)
        # For string memos, also destroy the StringStore entry
        if entry.string_key != MEMO_NO_STRING:
            self.strings.destroy(entry.string_key)
        # Destroy the memo entry
        self.memos.destroy(memo_id)

    def memo_count(self) -> Int:
        """Return the number of live memos."""
        return self.memos.count()

    def memo_output_key(self, memo_id: UInt32) -> UInt32:
        """Return the output signal key of the memo (for testing)."""
        return self.memos.output_key(memo_id)

    def memo_context_id(self, memo_id: UInt32) -> UInt32:
        """Return the reactive context ID of the memo (for testing)."""
        return self.memos.context_id(memo_id)

    def memo_string_key(self, memo_id: UInt32) -> UInt32:
        """Return the StringStore key of the memo (for testing)."""
        return self.memos.string_key(memo_id)

    # ── MemoBool operations ──────────────────────────────────────────

    def create_memo_bool(mut self, scope_id: UInt32, initial: Bool) -> UInt32:
        """Create a Bool memo with an initial cached value.

        Allocates a reactive context and an output signal (stored as
        Int32 0/1, same pattern as SignalBool).  The memo starts dirty
        so the first read triggers a computation.

        Returns the memo ID.
        """
        var context_id = self.signals.create[Int32](Int32(0))
        var initial_i32: Int32
        if initial:
            initial_i32 = Int32(1)
        else:
            initial_i32 = Int32(0)
        var output_key = self.signals.create[Int32](initial_i32)
        var memo_id = self.memos.create(context_id, output_key, scope_id)
        self._memo_ctx_ids.append(context_id)
        self._memo_ids.append(memo_id)
        return memo_id

    def memo_end_compute_bool(mut self, memo_id: UInt32, value: Bool):
        """End Bool memo computation and store the result.

        Writes the computed Bool value (as Int32 0/1) to the memo's
        output signal and clears the dirty flag.  Restores the previous
        reactive context.

        Phase 37 equality gate: compares old vs new value before writing.
        If the Bool value is unchanged, the output signal is NOT written
        and `value_changed` is set to False.
        """
        if not self.memos.contains(memo_id):
            return
        var entry = self.memos.get(memo_id)
        var val_i32: Int32
        if value:
            val_i32 = Int32(1)
        else:
            val_i32 = Int32(0)
        # Equality check: compare old cached value vs new computed value
        var old_value = self.signals.read[Int32](entry.output_key)
        var changed = old_value != val_i32
        if changed:
            self.signals.write[Int32](entry.output_key, val_i32)
            self._changed_signals.append(entry.output_key)
        # Record whether this recomputation produced a new value
        self.memos.set_value_changed(memo_id, changed)
        self.memos.clear_dirty(memo_id)
        self.memos.set_computing(memo_id, False)
        var prev = Int(self.signals.read[Int32](entry.context_id))
        self.current_context = prev

    def memo_read_bool(mut self, memo_id: UInt32) -> Bool:
        """Read the Bool memo's cached value.

        Subscribes the current reactive context (if any) to the memo's
        output signal, so the reader is notified when the memo recomputes.

        Returns the cached Bool value.
        """
        if not self.memos.contains(memo_id):
            return False
        var entry = self.memos.get(memo_id)
        if self.has_context():
            self.signals._entries[Int(entry.output_key)].subscribe(
                self.get_context()
            )
        return self.signals.read[Int32](entry.output_key) != 0

    # ── MemoString operations ────────────────────────────────────────

    def create_memo_string(
        mut self, scope_id: UInt32, initial: String
    ) -> UInt32:
        """Create a String memo with an initial cached value.

        Allocates a reactive context, a StringStore entry for the cached
        string, and a version signal for subscriber tracking (same
        pattern as SignalString).  The memo starts dirty.

        Returns the memo ID.
        """
        var context_id = self.signals.create[Int32](Int32(0))
        var string_key = self.strings.create(initial)
        var version_key = self.signals.create[Int32](Int32(0))
        var memo_id = self.memos.create(
            context_id, version_key, string_key, scope_id
        )
        self._memo_ctx_ids.append(context_id)
        self._memo_ids.append(memo_id)
        return memo_id

    def memo_end_compute_string(mut self, memo_id: UInt32, value: String):
        """End String memo computation and store the result.

        Writes the computed string to the StringStore, bumps the version
        signal, clears the dirty flag, and restores the previous
        reactive context.

        Phase 37 equality gate: compares old vs new string before writing.
        If the string is unchanged, neither the StringStore nor the version
        signal is updated, and `value_changed` is set to False.
        """
        if not self.memos.contains(memo_id):
            return
        var entry = self.memos.get(memo_id)
        # Equality check: compare old cached string vs new computed string
        var old_value = self.strings.read(entry.string_key)
        var changed = old_value != value
        if changed:
            # Write the string to the StringStore
            self.strings.write(entry.string_key, value)
            # Bump the version signal (output_key) to notify subscribers
            var ver = self.peek_signal[Int32](entry.output_key)
            self.signals.write[Int32](entry.output_key, ver + 1)
            self._changed_signals.append(entry.output_key)
        # Record whether this recomputation produced a new value
        self.memos.set_value_changed(memo_id, changed)
        self.memos.clear_dirty(memo_id)
        self.memos.set_computing(memo_id, False)
        var prev = Int(self.signals.read[Int32](entry.context_id))
        self.current_context = prev

    def memo_read_string(mut self, memo_id: UInt32) -> String:
        """Read the String memo's cached value (with context tracking).

        Subscribes the current reactive context (if any) to the memo's
        version signal, so the reader is notified when the memo
        recomputes to a new value.

        Returns a copy of the cached String value.
        """
        if not self.memos.contains(memo_id):
            return String("")
        var entry = self.memos.get(memo_id)
        # Subscribe via the version signal (output_key)
        if self.has_context():
            self.signals._entries[Int(entry.output_key)].subscribe(
                self.get_context()
            )
        _ = self.signals.read[Int32](entry.output_key)
        return self.strings.read(entry.string_key)

    def memo_peek_string(self, memo_id: UInt32) -> String:
        """Read the String memo's cached value WITHOUT subscribing.

        Returns a copy of the cached String value.
        """
        if not self.memos.contains(memo_id):
            return String("")
        var entry = self.memos.get(memo_id)
        return self.strings.read(entry.string_key)

    # ── Effect operations ────────────────────────────────────────────

    def create_effect(mut self, scope_id: UInt32) -> UInt32:
        """Create an effect with a reactive context.

        Allocates a "context signal" (a dummy Int32 signal whose key
        doubles as the context_id) for dependency tracking.
        The effect starts pending so the first run is triggered.

        Returns the effect ID.
        """
        # Allocate a context signal for dependency tracking
        var context_id = self.signals.create[Int32](Int32(0))
        var effect_id = self.effects.create(context_id, scope_id)
        # Register the context→effect mapping for pending propagation
        self._effect_ctx_ids.append(context_id)
        self._effect_ids.append(effect_id)
        return effect_id

    def effect_begin_run(mut self, effect_id: UInt32):
        """Begin effect execution.

        Sets the effect's reactive context as the current context so that
        signal reads during execution are tracked as dependencies.
        Clears old subscriptions (re-subscribes fresh on each run).
        """
        if not self.effects.contains(effect_id):
            return
        var entry = self.effects.get(effect_id)
        self.effects.set_running(effect_id, True)
        # Clear old subscriptions: unsubscribe this context from all signals
        # it was previously subscribed to.  Full scan — acceptable for now
        # since effect count is small.
        for i in range(len(self.signals._entries)):
            if (
                i < len(self.signals._states)
                and self.signals._states[i].occupied
            ):
                self.signals.unsubscribe(UInt32(i), entry.context_id)
        # Save previous context in the context signal's value
        var prev = self.current_context
        self.signals.write[Int32](entry.context_id, Int32(prev))
        self.current_context = Int(entry.context_id)

    def effect_end_run(mut self, effect_id: UInt32):
        """End effect execution.

        Clears the pending flag and running flag.
        Restores the previous reactive context.
        """
        if not self.effects.contains(effect_id):
            return
        var entry = self.effects.get(effect_id)
        self.effects.clear_pending(effect_id)
        self.effects.set_running(effect_id, False)
        # Restore previous context
        var prev = Int(self.signals.read[Int32](entry.context_id))
        self.current_context = prev

    def effect_is_pending(self, effect_id: UInt32) -> Bool:
        """Check whether the effect needs re-execution."""
        if not self.effects.contains(effect_id):
            return False
        return self.effects.is_pending(effect_id)

    def destroy_effect(mut self, effect_id: UInt32):
        """Destroy an effect, cleaning up its context signal.

        Removes the context→effect mapping, destroys the context signal,
        and frees the effect slot.
        """
        if not self.effects.contains(effect_id):
            return
        var entry = self.effects.get(effect_id)
        # Remove context→effect mapping
        for i in range(len(self._effect_ctx_ids)):
            if self._effect_ctx_ids[i] == entry.context_id:
                # Swap-remove for O(1)
                var last = len(self._effect_ctx_ids) - 1
                if i != last:
                    self._effect_ctx_ids[i] = self._effect_ctx_ids[last]
                    self._effect_ids[i] = self._effect_ids[last]
                _ = self._effect_ctx_ids.pop()
                _ = self._effect_ids.pop()
                break
        # Destroy the context signal
        self.signals.destroy(entry.context_id)
        # Destroy the effect entry
        self.effects.destroy(effect_id)

    def effect_count(self) -> Int:
        """Return the number of live effects."""
        return self.effects.count()

    def effect_context_id(self, effect_id: UInt32) -> UInt32:
        """Return the reactive context ID of the effect (for testing)."""
        return self.effects.context_id(effect_id)

    def drain_pending_effects(self) -> List[UInt32]:
        """Return a list of effect IDs that are currently pending.

        Does NOT clear pending flags — the caller must begin_run/end_run
        each effect to clear them.
        """
        return self.effects.pending_effects()

    def pending_effect_count(self) -> Int:
        """Return the number of pending effects."""
        return len(self.effects.pending_effects())

    def pending_effect_at(self, index: Int) -> UInt32:
        """Return the effect ID at the given index in the pending list.

        This is a convenience for WASM export (avoids returning a List).
        Precondition: index < pending_effect_count().
        """
        var pending = self.effects.pending_effects()
        return pending[index]


# ── Heap-allocated Runtime handle ────────────────────────────────────────────
#
# Since Mojo doesn't support module-level `var`, we heap-allocate the
# Runtime and pass its pointer (as Int64) through exported WASM functions.
# These helpers create and access the heap-allocated instance.


def create_runtime() -> Pointer[Runtime, MutUntrackedOrigin]:
    """Allocate a Runtime on the heap and return a pointer to it."""
    var ptr = unsafe_alloc[Runtime](1)
    ptr.unsafe_write(Runtime())
    return ptr


def destroy_runtime(ptr: Pointer[Runtime, MutUntrackedOrigin]):
    """Destroy and free a heap-allocated Runtime."""
    ptr.unsafe_deinit_pointee()
    ptr.unsafe_free()
