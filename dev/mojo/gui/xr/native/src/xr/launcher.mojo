# XR Launcher — Generic XR entry point that drives any GuiApp in XR.
#
# This module provides `xr_launch[AppType: GuiApp]()`, the XR-side
# counterpart to `desktop_launch[AppType: GuiApp]()`. It creates an XR
# session (headless or real OpenXR), wraps the app in a default panel,
# mounts the initial DOM, and enters the XR frame loop — polling for
# input events, dispatching them to the app, re-rendering dirty scopes,
# and flushing mutations to the panel's Blitz DOM.
#
# This is the function that `launch[AppType: GuiApp]()` calls on native
# targets when the XR feature is enabled.
#
# Architecture:
#
#   xr_launch[AppType: GuiApp](config)
#     │
#     ├── 1. Create XR session (headless or OpenXR)
#     ├── 2. Create a default panel (size from config or defaults)
#     ├── 3. Apply UA stylesheet to the panel
#     ├── 4. Instantiate the GuiApp via AppType()
#     ├── 5. Mount: app.mount(writer_ptr) → apply mutations to panel DOM
#     ├── 6. XR frame loop:
#     │      ├── xr.wait_frame()
#     │      ├── xr.begin_frame()
#     │      ├── poll events via xr.poll_event()
#     │      ├── app.handle_event(...)
#     │      ├── if app.has_dirty():
#     │      │     app.flush(writer_ptr)
#     │      │     apply mutations to panel DOM
#     │      ├── xr.render_dirty_panels()
#     │      └── xr.end_frame()
#     └── 7. Cleanup: app.destroy() + xr.destroy()
#
# The mutation interpreter reads the binary opcode buffer produced by
# mojo-gui/core's MutationWriter and translates each opcode into the
# corresponding XR Blitz FFI call scoped to the panel.
#
# For single-panel apps (the common case), the app never sees the XR
# infrastructure — it implements GuiApp exactly as it would for desktop
# or web. The launcher wraps the app in a panel automatically.
#
# Usage (called by launch() in core/src/platform/launch.mojo):
#
#     from xr.launcher import xr_launch
#     xr_launch[CounterApp](config)
#
# Or directly in a shared example's main():
#
#     fn main() raises:
#         launch[CounterApp](AppConfig(title="Counter", width=400, height=350))
#
# The same source compiles for web (--target wasm64-wasi), desktop
# (native), and XR (native + xr feature). On XR, launch() calls
# xr_launch() which creates an OpenXR session and enters the XR
# frame loop.

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from platform.gui_app import GuiApp
from platform.launch import AppConfig
from .xr_blitz import XRBlitz
from .renderer import XRMutationInterpreter
from .panel import PanelConfig, Vec3, Quaternion, default_panel_config


# ══════════════════════════════════════════════════════════════════════════════
# Constants
# ══════════════════════════════════════════════════════════════════════════════

# Default mutation buffer size (64 KiB — same as the desktop renderer).
comptime _DEFAULT_BUF_CAPACITY: Int = 65536

# Default panel pixel density (pixels per meter). 1200 ppm gives sharp text
# at XR viewing distances (~1m), equivalent to a 27" 4K monitor.
comptime _DEFAULT_PX_PER_METER: Float32 = 1200.0

# Default panel position — roughly eye height, 1m in front of the user.
comptime _DEFAULT_PANEL_X: Float32 = 0.0
comptime _DEFAULT_PANEL_Y: Float32 = 1.4
comptime _DEFAULT_PANEL_Z: Float32 = -1.0

# Conversion factor: the AppConfig provides width/height in logical pixels
# for desktop. For XR, we convert to meters using a reference density.
# 96 CSS pixels per inch × 39.37 inches per meter ≈ 3780 px/m.
# But for XR panels we want larger sizes, so we use a more practical
# scaling: 1000 CSS pixels ≈ 0.8m (comfortable reading panel width).
comptime _PX_TO_METERS: Float32 = 0.0008


# ══════════════════════════════════════════════════════════════════════════════
# MutationWriter heap management helpers (same pattern as desktop launcher)
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

    Called between mount and flush, and between successive flush calls,
    to reuse the mutation buffer without reallocating.

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
# xr_launch — The generic XR entry point
# ══════════════════════════════════════════════════════════════════════════════


fn xr_launch[AppType: GuiApp](config: AppConfig) raises:
    """Launch a mojo-gui application in XR via the OpenXR + Blitz shim.

    This is the XR-side counterpart to `desktop_launch`. It creates an
    XR session, wraps the app in a single default panel, mounts the
    initial DOM, and enters the XR frame loop that runs until the
    session ends.

    The XR frame loop follows this pattern each iteration:

      1. xr.wait_frame()         — block until OpenXR signals next frame
      2. xr.begin_frame()        — acquire swapchain image
      3. Poll events via xr.poll_event()
      4. Dispatch each event to app.handle_event()
      5. If dirty scopes exist:
         a. Reset the mutation writer
         b. Call app.flush() to re-render and diff
         c. Apply the resulting mutations to the panel DOM
      6. xr.render_dirty_panels() — Vello re-renders dirty panel textures
      7. xr.end_frame()          — submit quad layers to OpenXR compositor

    For single-panel apps, this single function replaces all app-specific
    XR entry points. The GuiApp trait methods encapsulate all app-specific
    logic.

    Type Parameters:
        AppType: A concrete type implementing the GuiApp trait.

    Args:
        config: Application configuration (title, size, debug).

    Raises:
        If the shared library cannot be loaded or the XR session
        fails to initialize.
    """
    # ── 1. Create the XR session ─────────────────────────────────────

    # Try to create a real OpenXR session first. mxr_create_session()
    # internally attempts to load the OpenXR runtime via dlopen and
    # initialize a Vulkan-backed session. If the runtime is unavailable
    # (no HMD, no OpenXR loader, Vulkan failure), it transparently falls
    # back to headless mode — same Blitz DOM fidelity, no XR compositing.
    var xr = XRBlitz.create_session(config.title)

    # The OpenXR backend (when active) already has GPU via the Vulkan
    # graphics binding. For headless fallback, init_gpu() creates a
    # standalone offscreen renderer (wgpu + Vello). Either way, this
    # call is idempotent and safe.
    var has_gpu = xr.init_gpu()
    if config.debug:
        if has_gpu:
            print("XR: GPU renderer initialised")
        else:
            print("XR: GPU not available — layout-only mode")

    # ── 2. Create the default panel ──────────────────────────────────

    # Convert desktop pixel dimensions to XR panel meters.
    var panel_width_m = Float32(config.width) * _PX_TO_METERS
    var panel_height_m = Float32(config.height) * _PX_TO_METERS

    # Clamp to reasonable XR panel sizes (0.2m – 2.0m).
    if panel_width_m < 0.2:
        panel_width_m = 0.2
    if panel_width_m > 2.0:
        panel_width_m = 2.0
    if panel_height_m < 0.15:
        panel_height_m = 0.15
    if panel_height_m > 1.5:
        panel_height_m = 1.5

    # Compute texture dimensions from physical size and pixel density.
    var tex_w = UInt32(Int(panel_width_m * _DEFAULT_PX_PER_METER + 0.5))
    var tex_h = UInt32(Int(panel_height_m * _DEFAULT_PX_PER_METER + 0.5))

    var panel_id = xr.create_panel(tex_w, tex_h)
    if panel_id == 0:
        raise Error("XR launcher: failed to create panel")

    # Set the panel's 3D transform — centered in front of the user.
    xr.panel_set_transform(
        panel_id,
        _DEFAULT_PANEL_X,
        _DEFAULT_PANEL_Y,
        _DEFAULT_PANEL_Z,
        0.0,
        0.0,
        0.0,
        1.0,  # identity quaternion
    )
    xr.panel_set_size(panel_id, panel_width_m, panel_height_m)
    xr.panel_set_visible(panel_id, True)

    # Add a basic user-agent stylesheet for consistent rendering.
    xr.panel_add_ua_stylesheet(panel_id, _DEFAULT_UA_CSS)

    # ── 3. Instantiate the app ───────────────────────────────────────

    var app = AppType()

    # ── 4. Allocate the mutation buffer and writer ───────────────────

    var buf_capacity = _DEFAULT_BUF_CAPACITY
    var buf_ptr = _alloc_mutation_buffer(buf_capacity)
    var writer_ptr = _alloc_writer(buf_ptr, buf_capacity)

    # Create the mutation interpreter that translates opcodes to XR
    # Blitz FFI calls scoped to our panel.
    var interpreter = XRMutationInterpreter(xr, panel_id)

    # ── 5. Mount — initial render ────────────────────────────────────

    var mount_len = app.mount(writer_ptr)
    if mount_len > 0:
        xr.panel_begin_mutations(panel_id)
        interpreter.apply(buf_ptr, Int(mount_len))
        xr.panel_end_mutations(panel_id)

    # ── 6. XR frame loop ─────────────────────────────────────────────

    # Track consecutive idle frames (no events, no dirty scopes) to detect
    # when a headless session has stabilized. Real OpenXR sessions block in
    # wait_frame() and continue until the runtime signals exit — they never
    # hit this counter. Headless sessions return immediately from wait_frame
    # with a real timestamp (not 0), so we need an explicit idle counter.
    var idle_frames: Int = 0

    while xr.is_alive():
        # 6a. Wait for the next frame from the OpenXR runtime.
        # In headless mode, this returns immediately with a real timestamp.
        var predicted_time = xr.wait_frame()

        # 6b. Begin the frame — acquire swapchain image.
        var should_render = xr.begin_frame()

        # 6c. Poll events from the XR input system.
        var had_event = False
        while True:
            var event = xr.poll_event()
            if not event.valid:
                break
            had_event = True

            # Only dispatch events targeting our panel.
            if event.panel_id == panel_id:
                _ = app.handle_event(
                    event.handler_id,
                    event.event_type,
                    event.value,
                )

        # 6d. If dirty scopes exist, flush mutations.
        var had_dirty = app.has_dirty()
        if had_dirty:
            _reset_writer(writer_ptr, buf_ptr, buf_capacity)

            var flush_len = app.flush(writer_ptr)
            if flush_len > 0:
                xr.panel_begin_mutations(panel_id)
                interpreter.apply(buf_ptr, Int(flush_len))
                xr.panel_end_mutations(panel_id)

        # 6e. Render dirty panel textures via Vello.
        if should_render:
            _ = xr.render_dirty_panels()

        # 6f. End the frame — submit quad layers to compositor.
        xr.end_frame()

        # 6g. Idle detection for headless sessions.
        #
        # Real OpenXR sessions block in wait_frame() and continue until
        # the runtime transitions to STOPPING/EXITING — they never
        # accumulate idle frames because wait_frame paces the loop.
        #
        # Headless sessions return immediately from wait_frame(), so
        # without an exit condition the loop spins forever. We track
        # consecutive idle frames (no events AND no dirty scopes) and
        # break once the app has stabilized.
        #
        # One idle frame is sufficient: after mount + flush, all reactive
        # effects have settled. Any event-driven work (click handlers,
        # input binding) requires injected events which reset the counter.
        if not had_event and not had_dirty:
            idle_frames += 1
            if idle_frames >= 1:
                break
        else:
            idle_frames = 0

    # ── 7. Cleanup ───────────────────────────────────────────────────

    _free_writer(writer_ptr)
    buf_ptr.free()
    app.destroy()
    xr.destroy_panel(panel_id)
    xr.destroy()


# ══════════════════════════════════════════════════════════════════════════════
# Default user-agent CSS for XR panels
# ══════════════════════════════════════════════════════════════════════════════
#
# This provides sensible defaults for HTML elements in XR panel rendering.
# Matches the desktop UA stylesheet but with XR-appropriate tweaks:
# - Slightly larger base font for legibility at XR viewing distances
# - Dark background by default (reduces eye strain in headsets)
# - No hover states (XR input is raycast-based, hover is explicit)

comptime _DEFAULT_UA_CSS = """
/* mojo-gui XR UA stylesheet */

*, *::before, *::after {
    box-sizing: border-box;
}

html {
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI',
                 Roboto, 'Helvetica Neue', Arial, sans-serif;
    font-size: 18px;
    line-height: 1.5;
    color: #e0e0e0;
    background-color: #1a1a2e;
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
    padding: 10px 20px;
    border: 1px solid #555;
    border-radius: 6px;
    background-color: #2a2a4a;
    color: #e0e0e0;
    cursor: pointer;
    margin: 4px;
}

input, textarea, select {
    font-family: inherit;
    font-size: inherit;
    padding: 10px;
    border: 1px solid #555;
    border-radius: 6px;
    background-color: #2a2a4a;
    color: #e0e0e0;
}
"""
