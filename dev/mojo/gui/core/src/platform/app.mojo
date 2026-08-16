# Platform App Trait — Interface every renderer must implement.
#
# This trait defines the contract between the mojo-gui/core reactive framework
# and the platform-specific renderer (web, desktop, native). Each renderer
# provides a concrete implementation that handles:
#
#   - Mutation buffer delivery (how binary DOM mutations reach the renderer)
#   - Event polling (how user interactions flow back to the framework)
#   - Render scheduling (how the renderer requests redraws)
#   - Event loop execution (who drives the main loop)
#
# The trait is the **only** thing that differs between platforms. App code
# never sees it directly — it interacts only with ComponentContext, signals,
# and the HTML DSL.
#
# Renderers implement this trait:
#
#   - WebApp (mojo-gui/web)     — WASM target; JS runtime drives the loop
#   - DesktopApp (mojo-gui/desktop) — Native target; Blitz drives the loop
#   - NativeApp (mojo-gui/native)   — Native target; platform widgets (future)
#
# Architecture:
#
#   ┌──────────────────────┐     binary mutation buffer      ┌─────────────────────┐
#   │                      │  ───────────────────────────►   │                     │
#   │  mojo-gui/core       │     (shared linear memory       │  Renderer           │
#   │  (reactive framework │      or heap buffer)            │  (web / desktop /   │
#   │   + virtual DOM      │                                 │   native)           │
#   │   + diff engine)     │  ◄───────────────────────────   │                     │
#   │                      │     event dispatch callbacks     │                     │
#   └──────────────────────┘                                 └─────────────────────┘
#
# Usage by renderer implementors:
#
#     struct MyRenderer(PlatformApp):
#         fn flush_mutations(mut self, buf: UnsafePointer[UInt8], length: Int):
#             # Send the mutation buffer to the rendering backend
#             ...
#
#         fn poll_events(mut self, handler_fn: fn(UInt32, UInt8) -> Bool):
#             # Drain events from the platform and dispatch via handler_fn
#             ...
#
#         fn request_animation_frame(mut self):
#             # Schedule the next render cycle
#             ...
#
#         fn run(mut self):
#             # Enter the platform event loop (blocking)
#             ...
#
#         fn should_quit(self) -> Bool:
#             # Return True if the application should exit
#             ...

from memory import UnsafePointer


# ══════════════════════════════════════════════════════════════════════════════
# PlatformApp — The renderer contract
# ══════════════════════════════════════════════════════════════════════════════


trait PlatformApp(Movable):
    """Platform host that drives the mojo-gui reactive framework.

    Every renderer (web, desktop, native) implements this trait to provide
    the platform-specific glue between the core framework and the rendering
    backend.

    The core framework produces a binary mutation buffer (opcodes like
    LOAD_TEMPLATE, SET_ATTRIBUTE, SET_TEXT, APPEND_CHILDREN, etc.) and
    the renderer consumes it to update the actual UI.

    In the reverse direction, the renderer captures user interaction events
    (clicks, input, keyboard) and dispatches them back to the framework's
    HandlerRegistry.

    Lifecycle:

        1. The renderer is created with platform-specific configuration
           (window size, title, debug flags, etc.)
        2. init() is called to set up the rendering surface
        3. The core framework writes mutations; flush_mutations() delivers them
        4. The renderer polls for events; poll_events() dispatches them
        5. run() enters the platform event loop (or returns immediately for
           WASM where the JS runtime owns the loop)
        6. destroy() cleans up platform resources
    """

    fn init(mut self) raises:
        """Initialize the rendering surface and platform resources.

        For web: No-op — the JS runtime sets up the DOM and WASM memory.
        For desktop: Create the Blitz engine, Winit window, and rendering surface.
        For native: Create the platform window and widget root.

        This is called once before any mutations are sent. Implementations
        should be idempotent (safe to call multiple times).

        Raises if platform initialization fails (e.g., missing display
        server, library not found).
        """
        ...

    fn flush_mutations(mut self, buf: UnsafePointer[UInt8], length: Int) raises:
        """Deliver a completed mutation buffer to the renderer.

        Args:
            buf: Pointer to the binary mutation buffer. The buffer contains
                 a sequence of opcodes (LOAD_TEMPLATE, SET_ATTRIBUTE, SET_TEXT,
                 APPEND_CHILDREN, REMOVE, etc.) terminated by OP_END.
            length: Number of valid bytes in the buffer (writer.offset after
                    finalize).

        The renderer reads the opcodes and applies them to the actual UI:
          - Web: The JS Interpreter reads from WASM shared memory (this may
                 be a no-op if the JS side reads the buffer directly).
          - Desktop: A native interpreter maps opcodes to Blitz DOM operations
                     via FFI.
          - Native: A Mojo interpreter maps opcodes to widget operations.

        The buffer is owned by the caller and remains valid for the duration
        of this call. The renderer must not hold a reference to it after
        returning.

        Raises if the rendering backend encounters an error (e.g., Blitz
        rendering failure, FFI error).
        """
        ...

    fn request_animation_frame(mut self):
        """Request that the renderer schedule a new render cycle.

        For web: Triggers requestAnimationFrame on the JS side.
        For desktop: Marks the window as needing redraw.
        For native: Queues a redraw with the platform compositor.

        This is a hint — the renderer may coalesce multiple requests into
        a single frame. The core framework calls this after event dispatch
        when dirty scopes exist.
        """
        ...

    fn should_quit(self) -> Bool:
        """Return True if the application should exit.

        For web: Always False (the browser tab lifecycle is separate).
        For desktop: True when the window is closed.
        For native: True when the last window is closed.

        The event loop in run() checks this each iteration.
        """
        ...

    fn destroy(mut self):
        """Release all platform resources.

        Called once when the application is shutting down. After this call,
        the renderer is in an invalid state and must not be used.

        For web: No-op (WASM memory is reclaimed by the browser).
        For desktop: Destroys the Blitz engine, Winit window, and frees the
                     mutation buffer.
        For native: Destroys platform windows and widgets.
        """
        ...


# ══════════════════════════════════════════════════════════════════════════════
# Target detection helpers
# ══════════════════════════════════════════════════════════════════════════════


fn is_wasm_target() -> Bool:
    """Return True if the current compilation target is WASM.

    Detection strategy: uses `sys.param_env.is_defined` to check for the
    `MOJO_TARGET_WASM` compile-time define. The build system must pass
    `-D MOJO_TARGET_WASM` when compiling for a WASM target.

    This replaces the previous `info.os_is_wasi()` call which was removed
    in Mojo 0.26.1.

    Note: The current WASM build pipeline (`mojo build --emit llvm` then
    `llc --mtriple=wasm64-wasi`) means the Mojo compiler itself targets
    the host architecture — it doesn't know about WASM at compile time.
    The web entry point (`web/src/main.mojo`) uses `@export` functions
    directly and never calls `launch()`, so this function returning False
    on native builds is the correct behavior. The define-based approach
    is forward-looking for when `mojo build --target wasm64-wasi` is
    supported natively.

    Used by launch() to select the appropriate renderer at compile time.
    """
    from sys.param_env import is_defined

    return is_defined["MOJO_TARGET_WASM"]()


fn is_native_target() -> Bool:
    """Return True if the current compilation target is native (non-WASM).

    Convenience inverse of is_wasm_target().
    """
    return not is_wasm_target()


fn is_xr_target() -> Bool:
    """Return True if the current compilation target is XR (OpenXR native).

    Detection strategy: uses `sys.param_env.is_defined` to check for the
    `MOJO_TARGET_XR` compile-time define. The build system must pass
    `-D MOJO_TARGET_XR` when compiling for an XR target.

    When this is True, `launch()` dispatches to `xr_launch[AppType]()`
    instead of `desktop_launch[AppType]()`. The app's GuiApp instance
    is wrapped in a single XR panel automatically.

    Build command:
        mojo build examples/counter/main.mojo -D MOJO_TARGET_XR \\
            -I core/src -I xr/native/src -I examples

    Used by launch() to select the XR renderer at compile time.
    """
    from sys.param_env import is_defined

    return is_defined["MOJO_TARGET_XR"]()
