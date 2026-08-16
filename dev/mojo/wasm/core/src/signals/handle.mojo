# Reactive Handles — Ergonomic wrappers around raw signal/memo keys.
#
# These handle types bundle a raw UInt32 key with a Runtime pointer,
# providing a safe, ergonomic API for reading and writing reactive
# values.  They are the foundation for the Dioxus-like concise API
# described in the plan's "Ergonomics-First API Design" section.
#
# Instead of:
#
#     var key = shell.use_signal_i32(0)
#     var val = shell.peek_signal_i32(key)
#     shell.write_signal_i32(key, val + 1)
#
# Developers write:
#
#     var count = SignalI32(key, runtime)
#     count += 1
#
# SignalI32 supports:
#   - peek()       — read without subscribing the current reactive context
#   - read()       — read and subscribe the current context
#   - set(value)   — write a new value (marks subscribers dirty)
#   - += -= *= //= — read-modify-write operators via peek + set
#   - __str__()    — for easy interpolation in text ("Count: " + str(count))
#
# MemoI32 supports:
#   - read()       — read the cached value (with context tracking)
#   - peek()       — read without subscribing
#   - is_dirty()   — check if recomputation is needed
#   - recompute()  — manual recompute when dirty (reads deps, writes cache)
#   - __str__()    — for easy interpolation
#
# SignalString supports:
#   - get() / peek() — read the string without subscribing
#   - read()         — read and subscribe the current context
#   - set(value)     — write a new string (marks subscribers dirty)
#   - is_empty()     — convenience check
#   - __str__()      — for easy interpolation
#
# All handle types are lightweight value types (Copyable) that hold a
# non-owning pointer to the Runtime.  They do NOT manage the Runtime's
# lifetime — the ComponentContext or AppShell owns that.
#
# Thread safety: WASM is single-threaded, so no synchronisation needed.

from memory import UnsafePointer
from .runtime import Runtime


# ══════════════════════════════════════════════════════════════════════════════
# SignalI32 — Ergonomic handle for an Int32 signal
# ══════════════════════════════════════════════════════════════════════════════


struct SignalI32(Copyable, Stringable):
    """Ergonomic handle wrapping a raw signal key + runtime pointer.

    Provides operator overloading for concise reactive state management:

        var count = SignalI32(key, runtime_ptr)
        count += 1          # read-modify-write
        count -= 1
        count.set(42)       # direct write
        var v = count.peek() # read without subscribing
        var v = count.read() # read and subscribe context

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var key: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        key: UInt32,
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
    ):
        """Create a signal handle from a raw key and runtime pointer.

        Args:
            key: The signal's key in the Runtime's SignalStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.key = key
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.key = other.key
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.key = other.key
        self.runtime = other.runtime

    # ── Read ─────────────────────────────────────────────────────────

    fn peek(self) -> Int32:
        """Read the signal value WITHOUT subscribing the current context.

        Use this for one-off reads (e.g. in event handlers) where you
        don't want the calling scope/memo/effect to re-run when the
        signal changes.

        Returns:
            The current Int32 value.
        """
        return self.runtime[0].peek_signal[Int32](self.key)

    fn read(self) -> Int32:
        """Read the signal value AND subscribe the current reactive context.

        If a scope, memo, or effect is currently rendering/computing/running,
        it will be added to this signal's subscriber set and marked dirty
        when the signal changes.

        Returns:
            The current Int32 value.
        """
        return self.runtime[0].read_signal[Int32](self.key)

    # ── Write ────────────────────────────────────────────────────────

    fn set(self, value: Int32):
        """Write a new value to the signal.

        All subscribers (scopes, memos, effects) will be marked dirty.

        Args:
            value: The new Int32 value.
        """
        self.runtime[0].write_signal[Int32](self.key, value)

    # ── Operator overloading — read-modify-write ─────────────────────

    fn __iadd__(mut self, rhs: Int32):
        """Add `rhs` to the signal value.  `count += 1`"""
        self.set(self.peek() + rhs)

    fn __isub__(mut self, rhs: Int32):
        """Subtract `rhs` from the signal value.  `count -= 1`"""
        self.set(self.peek() - rhs)

    fn __imul__(mut self, rhs: Int32):
        """Multiply the signal value by `rhs`.  `count *= 2`"""
        self.set(self.peek() * rhs)

    fn __ifloordiv__(mut self, rhs: Int32):
        """Floor-divide the signal value by `rhs`.  `count //= 2`"""
        self.set(self.peek() // rhs)

    fn __imod__(mut self, rhs: Int32):
        """Modulo the signal value by `rhs`.  `count %= 3`"""
        self.set(self.peek() % rhs)

    # ── Toggle (Bool-as-Int32) ───────────────────────────────────────

    fn toggle(self):
        """Toggle a boolean signal stored as Int32 (0 ↔ 1).

        Reads the current value and writes 1 if it was 0, or 0 otherwise.
        Useful for checkbox/switch state.
        """
        var current = self.peek()
        if current == 0:
            self.set(1)
        else:
            self.set(0)

    # ── Queries ──────────────────────────────────────────────────────

    fn version(self) -> UInt32:
        """Return the signal's write version (monotonically increasing).

        Useful for staleness checks — if the version hasn't changed,
        the value hasn't changed.
        """
        return self.runtime[0].signals.version(self.key)

    # ── Stringable ───────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return the signal value as a String for display/interpolation.

        Uses peek() so it does NOT subscribe the calling context.
        For reactive display, use read() explicitly and convert.
        """
        return String(self.peek())


# ══════════════════════════════════════════════════════════════════════════════
# MemoI32 — Ergonomic handle for an Int32 memo (computed/derived signal)
# ══════════════════════════════════════════════════════════════════════════════


struct MemoI32(Copyable, Stringable):
    """Ergonomic handle wrapping a raw memo ID + runtime pointer.

    Memos are derived values that cache their result and recompute only
    when their dependencies change.  Since Mojo WASM cannot store
    closures, the recomputation logic lives in the component — the
    memo handle provides lifecycle management and cached value access.

    Typical usage:

        var doubled = MemoI32(memo_id, runtime_ptr)

        # In render / effect:
        if doubled.is_dirty():
            doubled.begin_compute()
            var count = count_signal.read()  # subscribes memo to signal
            doubled.end_compute(count * 2)

        var text = "Doubled: " + str(doubled)

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var id: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self, id: UInt32, runtime: UnsafePointer[Runtime, MutExternalOrigin]
    ):
        """Create a memo handle from a raw ID and runtime pointer.

        Args:
            id: The memo's ID in the Runtime's MemoStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.id = id
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.runtime = other.runtime

    # ── Read ─────────────────────────────────────────────────────────

    fn read(self) -> Int32:
        """Read the memo's cached value (with context tracking).

        If a scope or effect is currently active, it will be subscribed
        to this memo's output signal and marked dirty when the memo
        recomputes to a new value.

        Returns:
            The cached Int32 value.
        """
        return self.runtime[0].memo_read_i32(self.id)

    fn peek(self) -> Int32:
        """Read the memo's cached value WITHOUT subscribing.

        Returns:
            The cached Int32 value.
        """
        # memo_read_i32 does context tracking; we need to read without it.
        # The MemoStore stores the value in its output signal (output_key),
        # which we can peek directly.
        return self.runtime[0].peek_signal[Int32](
            self.runtime[0].memos.output_key(self.id)
        )

    # ── Dirty / Recompute lifecycle ──────────────────────────────────

    fn is_dirty(self) -> Bool:
        """Check whether the memo needs recomputation.

        A memo becomes dirty when any of its input signals are written.

        Returns:
            True if the memo should be recomputed before reading.
        """
        return self.runtime[0].memo_is_dirty(self.id)

    fn begin_compute(self):
        """Begin memo recomputation.

        Sets the memo's reactive context as current, so any signals
        read during computation will be tracked as dependencies.
        Must be paired with end_compute().
        """
        self.runtime[0].memo_begin_compute(self.id)

    fn end_compute(self, value: Int32):
        """End memo recomputation and cache the result.

        Writes the computed value to the memo's output signal and
        restores the previous reactive context.

        Args:
            value: The newly computed Int32 value to cache.
        """
        self.runtime[0].memo_end_compute_i32(self.id, value)

    fn recompute_from(self, value: Int32):
        """Convenience: begin_compute + end_compute in one call.

        Use this when the computation doesn't need to read any signals
        inside the compute bracket (e.g. when the component already has
        the value).  Note: this does NOT set up dependency tracking for
        signals read outside the bracket.

        For proper dependency tracking, use begin_compute/end_compute
        and read signals between them.

        Args:
            value: The newly computed value.
        """
        self.begin_compute()
        self.end_compute(value)

    # ── Stringable ───────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return the memo's cached value as a String.

        Uses peek() so it does NOT subscribe the calling context.
        """
        return String(self.peek())


# ══════════════════════════════════════════════════════════════════════════════
# EffectI32 — Ergonomic handle for an effect (reactive side effect)
# ══════════════════════════════════════════════════════════════════════════════


struct EffectHandle(Copyable):
    """Ergonomic handle wrapping a raw effect ID + runtime pointer.

    Effects are reactive side effects that run when their dependencies
    change.  Since Mojo WASM cannot store closures, the effect logic
    lives in the component — the handle provides lifecycle management.

    Typical usage:

        var fx = EffectHandle(effect_id, runtime_ptr)

        # After event dispatch:
        if fx.is_pending():
            fx.begin_run()
            var count = count_signal.read()  # re-subscribes
            # ... perform side effect ...
            fx.end_run()

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var id: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self, id: UInt32, runtime: UnsafePointer[Runtime, MutExternalOrigin]
    ):
        """Create an effect handle from a raw ID and runtime pointer.

        Args:
            id: The effect's ID in the Runtime's EffectStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.id = id
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.runtime = other.runtime

    # ── Lifecycle ────────────────────────────────────────────────────

    fn is_pending(self) -> Bool:
        """Check whether this effect needs to run.

        An effect becomes pending when any of its subscribed signals
        are written.

        Returns:
            True if the effect should be executed.
        """
        return self.runtime[0].effect_is_pending(self.id)

    fn begin_run(self):
        """Begin effect execution.

        Sets the effect's reactive context as current, so any signals
        read during execution will be tracked as dependencies.
        Must be paired with end_run().
        """
        self.runtime[0].effect_begin_run(self.id)

    fn end_run(self):
        """End effect execution.

        Clears the pending flag and restores the previous reactive
        context.
        """
        self.runtime[0].effect_end_run(self.id)


# ══════════════════════════════════════════════════════════════════════════════
# SignalBool — Ergonomic handle for a Bool signal (stored as Int32 0/1)
# ══════════════════════════════════════════════════════════════════════════════


struct SignalBool(Copyable, Stringable):
    """Ergonomic handle wrapping a Bool signal stored as Int32 (0/1).

    Provides a proper boolean API on top of the Int32 signal store,
    since Mojo WASM only supports Int32 signals currently (generic
    `Signal[T]` is blocked on conditional conformance).

    Usage:

        var visible = SignalBool(key, runtime_ptr)
        visible.set(True)       # write
        var v = visible.get()   # read without subscribing
        var v = visible.read()  # read and subscribe context
        visible.toggle()        # flip True ↔ False

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var key: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        key: UInt32,
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
    ):
        """Create a bool signal handle from a raw key and runtime pointer.

        Args:
            key: The signal's key in the Runtime's SignalStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.key = key
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.key = other.key
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.key = other.key
        self.runtime = other.runtime

    # ── Read ─────────────────────────────────────────────────────────

    fn get(self) -> Bool:
        """Read the signal value as Bool WITHOUT subscribing.

        Equivalent to `peek()` on SignalI32 but returns Bool.
        Use this for one-off reads (e.g. in event handlers).

        Returns:
            True if the stored Int32 value is non-zero, False otherwise.
        """
        return self.runtime[0].peek_signal[Int32](self.key) != 0

    fn read(self) -> Bool:
        """Read the signal value as Bool AND subscribe the current context.

        If a scope, memo, or effect is currently rendering/computing,
        it will be subscribed to this signal and marked dirty on change.

        Returns:
            True if the stored Int32 value is non-zero, False otherwise.
        """
        return self.runtime[0].read_signal[Int32](self.key) != 0

    fn peek_i32(self) -> Int32:
        """Read the raw Int32 value (0 or 1) without subscribing.

        Useful when you need the Int32 representation directly.

        Returns:
            The raw Int32 value (0 or 1).
        """
        return self.runtime[0].peek_signal[Int32](self.key)

    # ── Write ────────────────────────────────────────────────────────

    fn set(self, value: Bool):
        """Write a boolean value to the signal.

        All subscribers (scopes, memos, effects) will be marked dirty.

        Args:
            value: The new Bool value (stored as Int32 1 or 0).
        """
        if value:
            self.runtime[0].write_signal[Int32](self.key, 1)
        else:
            self.runtime[0].write_signal[Int32](self.key, 0)

    fn toggle(self):
        """Flip the boolean value (True ↔ False).

        Reads the current value and writes its logical inverse.
        """
        var current = self.runtime[0].peek_signal[Int32](self.key)
        if current == 0:
            self.runtime[0].write_signal[Int32](self.key, 1)
        else:
            self.runtime[0].write_signal[Int32](self.key, 0)

    # ── Queries ──────────────────────────────────────────────────────

    fn version(self) -> UInt32:
        """Return the signal's write version (monotonically increasing).

        Returns:
            The version counter.
        """
        return self.runtime[0].signals.version(self.key)

    # ── Stringable ───────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return "true" or "false" for display/interpolation.

        Uses get() (peek) so it does NOT subscribe the calling context.
        """
        if self.get():
            return String("true")
        else:
            return String("false")


# ══════════════════════════════════════════════════════════════════════════════
# SignalString — Ergonomic handle for a reactive String signal
# ══════════════════════════════════════════════════════════════════════════════


struct SignalString(Copyable, Stringable):
    """Ergonomic handle wrapping a reactive String signal.

    Unlike SignalI32/SignalBool which store values in the type-erased
    SignalStore, SignalString stores the String in a separate StringStore
    (safe for heap types) and uses a companion Int32 "version signal"
    in the SignalStore for subscriber tracking and dirty-marking.

    Usage:

        var name = SignalString(string_key, version_key, runtime_ptr)
        name.set(String("hello"))    # write
        var v = name.get()           # read without subscribing
        var v = name.read()          # read and subscribe context

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var string_key: UInt32
    var version_key: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self,
        string_key: UInt32,
        version_key: UInt32,
        runtime: UnsafePointer[Runtime, MutExternalOrigin],
    ):
        """Create a string signal handle from raw keys and runtime pointer.

        Args:
            string_key: The key in the Runtime's StringStore.
            version_key: The companion version signal key in SignalStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.string_key = string_key
        self.version_key = version_key
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.string_key = other.string_key
        self.version_key = other.version_key
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.string_key = other.string_key
        self.version_key = other.version_key
        self.runtime = other.runtime

    # ── Read ─────────────────────────────────────────────────────────

    fn get(self) -> String:
        """Read the string value WITHOUT subscribing the current context.

        Use this for one-off reads (e.g. in event handlers) where you
        don't want the calling scope/memo/effect to re-run when the
        string changes.

        Returns:
            A copy of the current String value.
        """
        return self.runtime[0].peek_signal_string(self.string_key)

    fn peek(self) -> String:
        """Alias for get() — read without subscribing.

        Returns:
            A copy of the current String value.
        """
        return self.get()

    fn read(self) -> String:
        """Read the string value AND subscribe the current reactive context.

        If a scope, memo, or effect is currently rendering/computing/running,
        it will be added to the version signal's subscriber set and marked
        dirty when the string changes.

        Returns:
            A copy of the current String value.
        """
        return self.runtime[0].read_signal_string(
            self.string_key, self.version_key
        )

    # ── Write ────────────────────────────────────────────────────────

    fn set(self, value: String):
        """Write a new string value to the signal.

        Updates the StringStore entry and bumps the version signal,
        which marks all subscribers (scopes, memos, effects) dirty.

        Args:
            value: The new String value.
        """
        self.runtime[0].write_signal_string(
            self.string_key, self.version_key, value
        )

    # ── Queries ──────────────────────────────────────────────────────

    fn version(self) -> UInt32:
        """Return the signal's write version (monotonically increasing).

        Useful for staleness checks — if the version hasn't changed,
        the value hasn't changed.
        """
        return self.runtime[0].signals.version(self.version_key)

    fn is_empty(self) -> Bool:
        """Check whether the string value is empty.

        Uses get() (peek) so it does NOT subscribe the calling context.

        Returns:
            True if the string is empty, False otherwise.
        """
        return len(self.get()) == 0

    # ── Stringable ───────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return the string value for display/interpolation.

        Uses get() (peek) so it does NOT subscribe the calling context.
        For reactive display, use read() explicitly.
        """
        return self.get()


# ══════════════════════════════════════════════════════════════════════════════
# MemoBool — Ergonomic handle for a Bool memo (computed/derived signal)
# ══════════════════════════════════════════════════════════════════════════════


struct MemoBool(Copyable, Stringable):
    """Ergonomic handle wrapping a Bool memo ID + runtime pointer.

    Memos are derived values that cache their result and recompute only
    when their dependencies change.  MemoBool stores a Bool value
    (as Int32 0/1) in the output signal.

    Typical usage:

        var is_valid = MemoBool(memo_id, runtime_ptr)

        # In render / flush:
        if is_valid.is_dirty():
            is_valid.begin_compute()
            var name = name_signal.read()  # subscribes memo to signal
            is_valid.end_compute(len(name) > 0)

        var text = "Valid: " + str(is_valid)

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var id: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self, id: UInt32, runtime: UnsafePointer[Runtime, MutExternalOrigin]
    ):
        """Create a memo handle from a raw ID and runtime pointer.

        Args:
            id: The memo's ID in the Runtime's MemoStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.id = id
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.runtime = other.runtime

    # ── Read ─────────────────────────────────────────────────────────

    fn read(self) -> Bool:
        """Read the memo's cached value (with context tracking).

        If a scope or effect is currently active, it will be subscribed
        to this memo's output signal and marked dirty when the memo
        recomputes to a new value.

        Returns:
            The cached Bool value.
        """
        return self.runtime[0].memo_read_bool(self.id)

    fn peek(self) -> Bool:
        """Read the memo's cached value WITHOUT subscribing.

        Returns:
            The cached Bool value.
        """
        return (
            self.runtime[0].peek_signal[Int32](
                self.runtime[0].memos.output_key(self.id)
            )
            != 0
        )

    # ── Dirty / Recompute lifecycle ──────────────────────────────────

    fn is_dirty(self) -> Bool:
        """Check whether the memo needs recomputation.

        A memo becomes dirty when any of its input signals are written.

        Returns:
            True if the memo should be recomputed before reading.
        """
        return self.runtime[0].memo_is_dirty(self.id)

    fn begin_compute(self):
        """Begin memo recomputation.

        Sets the memo's reactive context as current, so any signals
        read during computation will be tracked as dependencies.
        Must be paired with end_compute().
        """
        self.runtime[0].memo_begin_compute(self.id)

    fn end_compute(self, value: Bool):
        """End memo recomputation and cache the result.

        Writes the computed value to the memo's output signal and
        restores the previous reactive context.

        Args:
            value: The newly computed Bool value to cache.
        """
        self.runtime[0].memo_end_compute_bool(self.id, value)

    fn recompute_from(self, value: Bool):
        """Convenience: begin_compute + end_compute in one call.

        Use this when the computation doesn't need to read any signals
        inside the compute bracket (e.g. when the component already has
        the value).  Note: this does NOT set up dependency tracking for
        signals read outside the bracket.

        For proper dependency tracking, use begin_compute/end_compute
        and read signals between them.

        Args:
            value: The newly computed value.
        """
        self.begin_compute()
        self.end_compute(value)

    # ── Stringable ───────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return the memo's cached value as a String.

        Uses peek() so it does NOT subscribe the calling context.
        """
        if self.peek():
            return String("True")
        else:
            return String("False")


# ══════════════════════════════════════════════════════════════════════════════
# MemoString — Ergonomic handle for a String memo (computed/derived signal)
# ══════════════════════════════════════════════════════════════════════════════


struct MemoString(Copyable, Stringable):
    """Ergonomic handle wrapping a String memo ID + runtime pointer.

    MemoString stores its cached value in the StringStore (same pattern
    as SignalString), with a companion version signal for subscriber
    tracking.

    Typical usage:

        var status = MemoString(memo_id, runtime_ptr)

        # In render / flush:
        if status.is_dirty():
            status.begin_compute()
            var count = count_signal.read()  # subscribes memo to signal
            if count > 0:
                status.end_compute(String("positive"))
            else:
                status.end_compute(String("zero"))

        var text = "Status: " + str(status)

    The handle does NOT own the Runtime — it holds a non-owning pointer.
    """

    var id: UInt32
    var runtime: UnsafePointer[Runtime, MutExternalOrigin]

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(
        out self, id: UInt32, runtime: UnsafePointer[Runtime, MutExternalOrigin]
    ):
        """Create a memo handle from a raw ID and runtime pointer.

        Args:
            id: The memo's ID in the Runtime's MemoStore.
            runtime: Non-owning pointer to the Runtime.
        """
        self.id = id
        self.runtime = runtime

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.runtime = other.runtime

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.runtime = other.runtime

    # ── Read ─────────────────────────────────────────────────────────

    fn read(self) -> String:
        """Read the memo's cached value (with context tracking).

        Subscribes the current reactive context via the version signal,
        so the reader is notified when the memo recomputes to a new value.

        Returns:
            A copy of the cached String value.
        """
        return self.runtime[0].memo_read_string(self.id)

    fn peek(self) -> String:
        """Read the memo's cached value WITHOUT subscribing.

        Returns:
            A copy of the cached String value.
        """
        return self.runtime[0].memo_peek_string(self.id)

    fn get(self) -> String:
        """Alias for read() — read with context tracking.

        Matches the SignalString API for consistency.

        Returns:
            A copy of the cached String value.
        """
        return self.read()

    # ── Dirty / Recompute lifecycle ──────────────────────────────────

    fn is_dirty(self) -> Bool:
        """Check whether the memo needs recomputation.

        A memo becomes dirty when any of its input signals are written.

        Returns:
            True if the memo should be recomputed before reading.
        """
        return self.runtime[0].memo_is_dirty(self.id)

    fn begin_compute(self):
        """Begin memo recomputation.

        Sets the memo's reactive context as current, so any signals
        read during computation will be tracked as dependencies.
        Must be paired with end_compute().
        """
        self.runtime[0].memo_begin_compute(self.id)

    fn end_compute(self, value: String):
        """End memo recomputation and cache the result.

        Writes the computed string to the StringStore, bumps the
        version signal, and restores the previous reactive context.

        Args:
            value: The newly computed String value to cache.
        """
        self.runtime[0].memo_end_compute_string(self.id, value)

    fn recompute_from(self, value: String):
        """Convenience: begin_compute + end_compute in one call.

        Use this when the computation doesn't need to read any signals
        inside the compute bracket (e.g. when the component already has
        the value).  Note: this does NOT set up dependency tracking for
        signals read outside the bracket.

        For proper dependency tracking, use begin_compute/end_compute
        and read signals between them.

        Args:
            value: The newly computed value.
        """
        self.begin_compute()
        self.end_compute(value)

    # ── Queries ──────────────────────────────────────────────────────

    fn is_empty(self) -> Bool:
        """Check whether the cached string value is empty.

        Uses peek() so it does NOT subscribe the calling context.

        Returns:
            True if the string is empty, False otherwise.
        """
        return len(self.peek()) == 0

    # ── Stringable ───────────────────────────────────────────────────

    fn __str__(self) -> String:
        """Return the memo's cached value as a String.

        Uses peek() so it does NOT subscribe the calling context.
        """
        return self.peek()
