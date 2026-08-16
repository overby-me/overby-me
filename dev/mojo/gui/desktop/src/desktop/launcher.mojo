# Desktop Launcher — Generic event loop that drives any GuiApp on the desktop.
#
# This module provides `desktop_launch[AppType: GuiApp]()`, the desktop-side
# counterpart to the WASM @export wrappers. It creates a Blitz renderer
# window, mounts the initial DOM, and enters the platform event loop —
# polling for user events, dispatching them to the app, re-rendering dirty
# scopes, and flushing mutations to the Blitz DOM.
#
# This is the function that `launch[AppType: GuiApp]()` calls on native
# targets (via `@parameter if not is_wasm_target()`).
#
# Architecture:
#
#   desktop_launch[AppType: GuiApp](config)
#     │
#     ├── 1. Create Blitz renderer (window + DOM + rendering pipeline)
#     ├── 2. Instantiate the GuiApp via AppType()
#     ├── 3. Mount: app.mount(writer_ptr) → apply mutations to Blitz DOM
#     ├── 4. Event loop:
#     │      ├── blitz.step(blocking=False)     — process OS events
#     │      ├── blitz.poll_event()             — drain buffered events
#     │      ├── app.handle_event(...)          — dispatch to HandlerRegistry
#     │      ├── if app.has_dirty():
#     │      │     app.flush(writer_ptr)        — re-render + diff
#     │      │     apply mutations to Blitz DOM
#     │      │     blitz.request_redraw()
#     │      └── else: blitz.step(blocking=True) — sleep until next event
#     └── 5. Cleanup: app.destroy() + blitz.destroy()
#
# The mutation interpreter reads the binary opcode buffer produced by
# mojo-gui/core's MutationWriter and translates each opcode into the
# corresponding Blitz C FFI call. This is the Mojo equivalent of the
# JS Interpreter class in the web renderer.
#
# Step 3.9.2: This implements the generic desktop event loop that was
# blocked on Phase 4 (Blitz). With this in place, Steps 3.9.6 (delete
# per-renderer example duplicates) and 3.9.7 (cross-target verification)
# become unblocked.
#
# Usage (called by launch() in core/src/platform/launch.mojo):
#
#     from desktop.launcher import desktop_launch
#     desktop_launch[CounterApp](config)
#
# Or directly in a shared example's main():
#
#     fn main() raises:
#         launch[CounterApp](AppConfig(title="Counter", width=400, height=350))
#
# The same source compiles for web (--target wasm64-wasi) and desktop
# (native). On WASM, launch() stores config and returns (JS drives the
# loop). On native, launch() calls desktop_launch() which blocks until
# the window is closed.

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from platform.gui_app import GuiApp
from platform.launch import AppConfig
from .blitz import Blitz, BlitzEvent
from .renderer import MutationInterpreter


# ══════════════════════════════════════════════════════════════════════════════
# Constants
# ══════════════════════════════════════════════════════════════════════════════

# Default mutation buffer size (64 KiB — same as the webview desktop renderer).
comptime _DEFAULT_BUF_CAPACITY: Int = 65536


# ══════════════════════════════════════════════════════════════════════════════
# MutationWriter heap management helpers
# ══════════════════════════════════════════════════════════════════════════════


fn _alloc_mutation_buffer(
    capacity: Int,
) -> UnsafePointer[UInt8, MutExternalOrigin]:
    """Allocate a heap buffer for mutation data.

    Returns a pointer to a zeroed buffer of `capacity` bytes.
    """
    var buf = alloc[UInt8](capacity)
    for i in range(capacity):
        buf[i] = 0
    return buf


fn _alloc_writer(
    buf_ptr: UnsafePointer[UInt8, MutExternalOrigin], capacity: Int
) -> UnsafePointer[MutationWriter, MutExternalOrigin]:
    """Allocate a MutationWriter on the heap backed by the given buffer.

    The writer is initialized at offset 0 with the given buffer and capacity.

    Args:
        buf_ptr: Pointer to the mutation buffer.
        capacity: Size of the buffer in bytes.

    Returns:
        Heap-allocated MutationWriter pointer.
    """
    # We need to cast the buffer pointer to MutExternalOrigin for the writer.
    # This is safe because the buffer is heap-allocated and we control its lifetime.
    var slot = alloc[Int](1)
    slot[0] = Int(buf_ptr)
    var ext_ptr = slot.bitcast[UnsafePointer[UInt8, MutExternalOrigin]]()[0]
    slot.free()

    var ptr = alloc[MutationWriter](1)
    ptr.init_pointee_move(MutationWriter(ext_ptr, capacity))
    return ptr


fn _reset_writer(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    buf_ptr: UnsafePointer[UInt8, MutExternalOrigin],
    capacity: Int,
):
    """Reset a MutationWriter to offset 0, reusing the same buffer.

    This is called between mount and flush, and between successive flush
    calls, to reuse the mutation buffer without reallocating.

    Args:
        writer_ptr: Pointer to the heap-allocated MutationWriter.
        buf_ptr: Pointer to the mutation buffer (same buffer as before).
        capacity: Buffer capacity in bytes.
    """
    var slot = alloc[Int](1)
    slot[0] = Int(buf_ptr)
    var ext_ptr = slot.bitcast[UnsafePointer[UInt8, MutExternalOrigin]]()[0]
    slot.free()

    writer_ptr.destroy_pointee()
    writer_ptr.init_pointee_move(MutationWriter(ext_ptr, capacity))


fn _free_writer(
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
):
    """Destroy and free a heap-allocated MutationWriter.

    Args:
        writer_ptr: Pointer to the MutationWriter to free.
    """
    writer_ptr.destroy_pointee()
    writer_ptr.free()


# ══════════════════════════════════════════════════════════════════════════════
# desktop_launch — The generic desktop entry point
# ══════════════════════════════════════════════════════════════════════════════


fn desktop_launch[AppType: GuiApp](config: AppConfig) raises:
    """Launch a mojo-gui application on the desktop via Blitz.

    This is the desktop-side counterpart to the WASM @export lifecycle.
    It creates a native window with the Blitz HTML/CSS rendering engine,
    mounts the app's initial DOM, and enters a blocking event loop that
    runs until the window is closed.

    The event loop follows this pattern each iteration:

      1. Process OS events (Winit) via blitz.step(blocking=False)
      2. Drain buffered events from the Blitz shim
      3. Dispatch each event to app.handle_event()
      4. If any scopes are dirty:
         a. Reset the mutation writer
         b. Call app.flush() to re-render and diff
         c. Apply the resulting mutations to the Blitz DOM
         d. Request a redraw
      5. If no events and no dirty scopes:
         a. Block on blitz.step(blocking=True) until the next OS event

    This single function replaces all per-app desktop entry points. The
    GuiApp trait methods encapsulate all app-specific logic (ConditionalSlot
    management, KeyedList flush, custom event routing, etc.).

    Type Parameters:
        AppType: A concrete type implementing the GuiApp trait.

    Args:
        config: Application configuration (title, size, debug).

    Raises:
        If the Blitz shared library cannot be loaded or the renderer
        fails to initialize.
    """
    # ── 1. Create the Blitz renderer ─────────────────────────────────────

    var blitz = Blitz.create(
        config.title,
        config.width,
        config.height,
        debug=config.debug,
    )

    # Add a basic user-agent stylesheet for consistent rendering.
    # This provides reasonable defaults for the HTML elements used by
    # mojo-gui's DSL (div, h1, button, p, etc.).
    blitz.add_ua_stylesheet(_DEFAULT_UA_CSS)

    # ── 2. Instantiate the app ───────────────────────────────────────────

    var app = AppType()

    # ── 3. Allocate the mutation buffer and writer ───────────────────────

    var buf_capacity = _DEFAULT_BUF_CAPACITY
    var buf_ptr = _alloc_mutation_buffer(buf_capacity)
    var writer_ptr = _alloc_writer(buf_ptr, buf_capacity)

    # Create the mutation interpreter that translates opcodes to Blitz calls
    var interpreter = MutationInterpreter(blitz)

    # ── 4. Mount — initial render ────────────────────────────────────────

    var mount_len = app.mount(writer_ptr)
    if mount_len > 0:
        blitz.begin_mutations()
        interpreter.apply(buf_ptr, Int(mount_len))
        blitz.end_mutations()
        blitz.request_redraw()

    # ── 5. Event loop ────────────────────────────────────────────────────

    while blitz.is_alive():
        # 5a. Process pending OS events (non-blocking)
        _ = blitz.step(blocking=False)

        # 5b. Drain buffered events from the Blitz shim
        var had_event = False
        while True:
            var event = blitz.poll_event()
            if not event.valid:
                break
            had_event = True

            # 5c. Dispatch to the app's handler registry
            _ = app.handle_event(
                event.handler_id,
                event.event_type,
                event.value,
            )

        # 5d. If dirty scopes exist, flush mutations
        if app.has_dirty():
            # Reset the writer to reuse the buffer
            _reset_writer(writer_ptr, buf_ptr, buf_capacity)

            var flush_len = app.flush(writer_ptr)
            if flush_len > 0:
                blitz.begin_mutations()
                interpreter.apply(buf_ptr, Int(flush_len))
                blitz.end_mutations()
                blitz.request_redraw()
        elif not had_event:
            # 5e. Nothing to do — block until the next OS event
            _ = blitz.step(blocking=True)

    # ── 6. Cleanup ───────────────────────────────────────────────────────

    _free_writer(writer_ptr)
    buf_ptr.free()
    app.destroy()
    blitz.destroy()


# ══════════════════════════════════════════════════════════════════════════════
# Default user-agent CSS
# ══════════════════════════════════════════════════════════════════════════════
#
# This provides sensible defaults for the HTML elements commonly used by
# mojo-gui applications. It's injected into the Blitz document as a
# user-agent stylesheet.
#
# Blitz already includes its own UA stylesheet (based on Firefox defaults),
# so this is supplementary — it adds interactive affordances (button hover,
# focus outlines, etc.) and tweaks for the mojo-gui counter/todo/bench
# examples.

comptime _DEFAULT_UA_CSS = """
/* mojo-gui desktop UA stylesheet */

*, *::before, *::after {
    box-sizing: border-box;
}

html {
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI',
                 Roboto, 'Helvetica Neue', Arial, sans-serif;
    font-size: 16px;
    line-height: 1.5;
    color: #1a1a1a;
    background-color: #ffffff;
}

body {
    margin: 0;
    padding: 16px;
}

h1 { font-size: 2em; margin: 0.67em 0; }
h2 { font-size: 1.5em; margin: 0.83em 0; }
h3 { font-size: 1.17em; margin: 1em 0; }
h4, h5, h6 { margin: 1.33em 0; }

p { margin: 1em 0; }

button {
    font-family: inherit;
    font-size: inherit;
    padding: 8px 16px;
    border: 1px solid #ccc;
    border-radius: 4px;
    background-color: #f0f0f0;
    cursor: pointer;
    margin: 4px;
}

input, textarea, select {
    font-family: inherit;
    font-size: inherit;
    padding: 8px;
    border: 1px solid #ccc;
    border-radius: 4px;
}

/* Dark mode support */
@media (prefers-color-scheme: dark) {
    html {
        color: #e0e0e0;
        background-color: #1a1a1a;
    }
    button {
        background-color: #333;
        border-color: #555;
        color: #e0e0e0;
    }
    input, textarea, select {
        background-color: #333;
        border-color: #555;
        color: #e0e0e0;
    }
}
"""
