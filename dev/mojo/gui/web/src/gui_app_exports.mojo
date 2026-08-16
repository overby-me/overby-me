# GuiApp WASM Export Helpers — Generic lifecycle functions for @export wrappers.
#
# This module provides parametric helper functions that implement the standard
# WASM export lifecycle for any GuiApp implementor. Instead of writing
# per-app free functions (counter_app_init, counter_app_rebuild, etc.) and
# per-app @export wrappers that duplicate the same heap alloc / writer alloc /
# dispatch / free pattern, each app's @export surface becomes a one-liner:
#
#     @export fn counter_init() -> Int64:
#         return gui_app_init[CounterApp]()
#
#     @export fn counter_rebuild(app_ptr: Int64, buf: Int64, cap: Int32) -> Int32:
#         return gui_app_mount[CounterApp](app_ptr, buf, cap)
#
#     @export fn counter_handle_event(app_ptr: Int64, hid: Int32, et: Int32) -> Int32:
#         return gui_app_handle_event[CounterApp](app_ptr, hid, et)
#
#     @export fn counter_flush(app_ptr: Int64, buf: Int64, cap: Int32) -> Int32:
#         return gui_app_flush[CounterApp](app_ptr, buf, cap)
#
#     @export fn counter_destroy(app_ptr: Int64):
#         gui_app_destroy[CounterApp](app_ptr)
#
# This eliminates the backwards-compatible free functions in each example
# (counter_app_init, counter_app_rebuild, counter_app_flush, etc.) and
# the duplicated alloc/free boilerplate in main.mojo's @export wrappers.
#
# The generic helpers use the GuiApp trait methods directly:
#   - __init__()      → gui_app_init
#   - mount()         → gui_app_mount
#   - handle_event()  → gui_app_handle_event / gui_app_handle_event_string
#   - flush()         → gui_app_flush
#   - has_dirty()     → gui_app_has_dirty
#   - consume_dirty() → gui_app_consume_dirty
#   - destroy()       → gui_app_destroy
#
# App-specific query exports (counter_count_value, todo_item_count, etc.)
# remain as hand-written @export functions in main.mojo — they reach into
# app-specific fields that the GuiApp trait does not expose.
#
# Design notes:
#
#   - These helpers are NOT @export themselves — they are ordinary parametric
#     functions. @export functions cannot be parametric in Mojo. The @export
#     wrappers in main.mojo call these helpers with a concrete type parameter.
#
#   - The pointer ↔ Int64 conversion and MutationWriter heap allocation
#     patterns are inlined from main.mojo's Section 1 utilities. This module
#     re-uses _as_ptr, _to_i64, _heap_new, _heap_del, _get, _b2i,
#     _alloc_writer, _free_writer from main.mojo (imported by the caller).
#     To keep this module self-contained and avoid circular imports, we
#     duplicate the minimal set of helpers needed.
#
#   - The WASM ABI uses Int64 for pointers (wasm64) and Int32 for return
#     values. Bool is returned as Int32 (1 or 0) via _b2i().

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from platform import GuiApp


# ══════════════════════════════════════════════════════════════════════════════
# Internal helpers — pointer conversion and writer management
# ══════════════════════════════════════════════════════════════════════════════
#
# These are self-contained versions of the utilities in main.mojo Section 1.
# Having them here avoids circular imports and keeps this module independent.


@always_inline
fn _as_ptr[T: AnyType](addr: Int) -> UnsafePointer[T, MutExternalOrigin]:
    """Reinterpret an integer address as an UnsafePointer[T, MutExternalOrigin].
    """
    var slot = alloc[Int](1)
    slot[0] = addr
    var result = slot.bitcast[UnsafePointer[T, MutExternalOrigin]]()[0]
    slot.free()
    return result


@always_inline
fn _to_i64[T: AnyType](ptr: UnsafePointer[T, MutExternalOrigin]) -> Int64:
    """Return the raw address of a typed pointer as Int64."""
    return Int64(Int(ptr))


@always_inline
fn _get[T: AnyType](ptr: Int64) -> UnsafePointer[T, MutExternalOrigin]:
    """Reinterpret an Int64 WASM handle as an UnsafePointer[T, MutExternalOrigin].
    """
    return _as_ptr[T](Int(ptr))


@always_inline
fn _b2i(val: Bool) -> Int32:
    """Convert a Bool to Int32 (1 or 0) for WASM export returns."""
    if val:
        return 1
    return 0


@always_inline
fn _alloc_writer(
    buf_ptr: Int64, capacity: Int32
) -> UnsafePointer[MutationWriter, MutExternalOrigin]:
    """Allocate a MutationWriter on the heap with the given buffer and capacity.
    """
    var ptr = alloc[MutationWriter](1)
    ptr.init_pointee_move(MutationWriter(_get[UInt8](buf_ptr), Int(capacity)))
    return ptr


@always_inline
fn _free_writer(ptr: UnsafePointer[MutationWriter, MutExternalOrigin]):
    """Destroy and free a heap-allocated MutationWriter."""
    ptr.destroy_pointee()
    ptr.free()


# ══════════════════════════════════════════════════════════════════════════════
# Generic GuiApp lifecycle helpers
# ══════════════════════════════════════════════════════════════════════════════


@always_inline
fn gui_app_init[T: GuiApp]() -> Int64:
    """Allocate a GuiApp on the heap and initialize it.

    Calls T.__init__() which performs all app setup (context creation,
    signal creation, view registration, event handler binding).

    Returns the heap pointer as Int64 for the WASM ABI.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Returns:
        Int64 address of the heap-allocated app instance.
    """
    var ptr = alloc[T](1)
    ptr.init_pointee_move(T())
    return _to_i64(ptr)


@always_inline
fn gui_app_destroy[T: GuiApp](app_ptr: Int64):
    """Destroy a heap-allocated GuiApp and free its memory.

    Calls T.destroy() to release all app resources (ComponentContext,
    signals, VNode store, etc.), then destroys and frees the heap slot.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.
    """
    var ptr = _get[T](app_ptr)
    ptr[0].destroy()
    ptr.destroy_pointee()
    ptr.free()


@always_inline
fn gui_app_mount[
    T: GuiApp
](app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Perform the initial render (mount) of a GuiApp.

    Allocates a MutationWriter on the heap, calls T.mount() to emit
    RegisterTemplate + CreateEngine + AppendChildren mutations, then
    frees the writer.

    This is the generic equivalent of counter_app_rebuild, todo_app_rebuild,
    bench_app_rebuild, etc.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.
        buf_ptr: Int64 address of the mutation buffer in WASM linear memory.
        capacity: Size of the mutation buffer in bytes.

    Returns:
        Byte length of mutation data written. The JS Interpreter reads
        this many bytes from the buffer to apply the initial DOM.
    """
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _get[T](app_ptr)[0].mount(writer_ptr)
    _free_writer(writer_ptr)
    return offset


@always_inline
fn gui_app_handle_event[
    T: GuiApp
](app_ptr: Int64, handler_id: Int32, event_type: Int32) -> Int32:
    """Dispatch a user interaction event (no string payload) to a GuiApp.

    Calls T.handle_event() with an empty string value. This is the
    standard path for click events, keyboard shortcuts, and other
    events that don't carry a string payload.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.
        handler_id: The handler to invoke (from HandlerRegistry).
        event_type: The event type tag (EVT_CLICK, EVT_INPUT, etc.).

    Returns:
        1 if an action was executed, 0 otherwise.
    """
    return _b2i(
        _get[T](app_ptr)[0].handle_event(
            UInt32(handler_id), UInt8(event_type), String("")
        )
    )


@always_inline
fn gui_app_handle_event_string[
    T: GuiApp
](app_ptr: Int64, handler_id: Int32, event_type: Int32, value: String) -> Int32:
    """Dispatch a user interaction event with a string payload to a GuiApp.

    Calls T.handle_event() with the provided string value. This is the
    path for input/change events where event.target.value is extracted
    by the JS EventBridge and passed through to WASM.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.
        handler_id: The handler to invoke (from HandlerRegistry).
        event_type: The event type tag (EVT_INPUT, EVT_CHANGE, etc.).
        value: String payload from the event (e.g., event.target.value).

    Returns:
        1 if an action was executed, 0 otherwise.
    """
    return _b2i(
        _get[T](app_ptr)[0].handle_event(
            UInt32(handler_id), UInt8(event_type), value
        )
    )


@always_inline
fn gui_app_flush[
    T: GuiApp
](app_ptr: Int64, buf_ptr: Int64, capacity: Int32) -> Int32:
    """Flush pending updates (re-render dirty scopes) for a GuiApp.

    Allocates a MutationWriter on the heap, calls T.flush() to diff
    old and new VNode trees and write update mutations, then frees
    the writer.

    This is the generic equivalent of counter_app_flush, todo_app_flush,
    bench_app_flush, etc.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.
        buf_ptr: Int64 address of the mutation buffer in WASM linear memory.
        capacity: Size of the mutation buffer in bytes.

    Returns:
        Byte length of mutation data written, or 0 if nothing was dirty.
    """
    var writer_ptr = _alloc_writer(buf_ptr, capacity)
    var offset = _get[T](app_ptr)[0].flush(writer_ptr)
    _free_writer(writer_ptr)
    return offset


@always_inline
fn gui_app_has_dirty[T: GuiApp](app_ptr: Int64) -> Int32:
    """Check if a GuiApp has dirty scopes needing re-render.

    Calls T.has_dirty() — a non-consuming check that does not drain
    the dirty queue.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.

    Returns:
        1 if at least one scope is marked dirty, 0 otherwise.
    """
    return _b2i(_get[T](app_ptr)[0].has_dirty())


@always_inline
fn gui_app_consume_dirty[T: GuiApp](app_ptr: Int64) -> Int32:
    """Collect and consume all dirty scopes in a GuiApp.

    Calls T.consume_dirty() — drains the scheduler's dirty queue.
    After this call, has_dirty() returns False until new events
    mark scopes dirty again.

    Type Parameters:
        T: A concrete type implementing the GuiApp trait.

    Args:
        app_ptr: Int64 address of the heap-allocated app instance.

    Returns:
        1 if any scopes were dirty (and consumed), 0 otherwise.
    """
    return _b2i(_get[T](app_ptr)[0].consume_dirty())
