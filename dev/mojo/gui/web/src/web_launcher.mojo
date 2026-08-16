# WebApp — Browser renderer implementing the PlatformApp trait.
#
# This module provides the WASM-target implementation of the platform
# abstraction layer. On the web, the JS runtime drives the event loop
# (requestAnimationFrame + DOM event listeners), so most PlatformApp
# methods are thin stubs or no-ops.
#
# Architecture:
#
#     ┌─ Browser Tab ──────────────────────────────────────────────────┐
#     │                                                                 │
#     │  JS Runtime (runtime/mod.ts)                                    │
#     │    ├── Interpreter (DOM stack machine)                          │
#     │    ├── EventBridge (DOM event delegation → WASM dispatch)       │
#     │    ├── Memory (WASM linear memory management)                   │
#     │    └── requestAnimationFrame loop                               │
#     │         │                                                       │
#     │         │ @export calls                                         │
#     │         ▼                                                       │
#     │  WASM Module                                                    │
#     │    ├── main.mojo (@export wrappers)                             │
#     │    ├── WebApp (this module)                                     │
#     │    └── mojo-gui/core (signals, vdom, diff, mutations)           │
#     │                                                                 │
#     └─────────────────────────────────────────────────────────────────┘
#
# How it works:
#
#   On the WASM target, the browser owns the event loop. The JS runtime:
#     1. Instantiates the WASM module and calls @export init functions
#     2. Reads the mutation buffer from WASM shared linear memory
#     3. Applies mutations to the real DOM via the Interpreter
#     4. Captures DOM events and dispatches them back to WASM via @export
#     5. After each event dispatch, calls @export flush to get new mutations
#
#   WebApp's role is minimal — it stores platform metadata and satisfies
#   the PlatformApp trait so that shared app code can use the platform
#   abstraction layer uniformly across targets.
#
# Mutation flow (WASM target):
#
#   Core writes mutations → WASM linear memory buffer
#                           ↓
#   JS Interpreter reads buffer directly (zero-copy, shared memory)
#                           ↓
#   DOM operations (createElement, setAttribute, appendChild, etc.)
#
# Event flow (WASM target):
#
#   DOM event → EventBridge captures
#                ↓
#   @export handle_event(app_ptr, handler_id, event_type)
#                ↓
#   HandlerRegistry.dispatch() → marks scopes dirty
#                ↓
#   @export flush(app_ptr, buf_ptr, capacity) → re-render + diff
#                ↓
#   JS reads mutation buffer → applies to DOM
#
# Usage:
#
#   The WebApp is not typically instantiated directly by app code.
#   Instead, the @export wrappers in main.mojo create and manage it.
#   App code uses ComponentContext, signals, and the HTML DSL — all
#   platform-agnostic APIs from mojo-gui/core.
#
#   For apps using the launch() pattern:
#
#       from platform import launch, AppConfig
#
#       fn main():
#           launch(AppConfig(title="My App"))
#           # On WASM: config is stored; JS runtime takes over.
#           # On native: config is used by DesktopApp to create a window.
#
# Relationship to main.mojo:
#
#   main.mojo contains the @export WASM wrappers that the JS runtime calls.
#   WebApp provides the PlatformApp implementation that those wrappers
#   can optionally use for trait-based dispatch. Currently, main.mojo
#   wires things manually (direct function calls to app structs). As the
#   platform abstraction matures, main.mojo can be simplified to delegate
#   to WebApp methods.

from memory import UnsafePointer
from platform import (
    PlatformApp,
    AppConfig,
    PlatformFeatures,
    register_features,
    web_features,
    get_launch_config,
)


# ══════════════════════════════════════════════════════════════════════════════
# WebApp — Browser renderer (WASM target)
# ══════════════════════════════════════════════════════════════════════════════


struct WebApp(PlatformApp):
    """Browser renderer — mutations flow to JS Interpreter via shared WASM memory.

    On the WASM target, the JS runtime owns the event loop and drives
    rendering. WebApp is a thin wrapper that:

      - Stores the app configuration (title, debug flags)
      - Registers web platform features on init()
      - Provides no-op implementations for methods the JS side handles

    The actual rendering work happens in the JS Interpreter (runtime/interpreter.ts)
    which reads the binary mutation buffer directly from WASM linear memory.

    Event dispatch is handled by the JS EventBridge (runtime/events.ts) which
    calls @export WASM functions for each event. The @export wrappers in
    main.mojo route events to the appropriate app's HandlerRegistry.

    Lifecycle on WASM:

        1. JS runtime loads WASM, calls @export init → WebApp created
        2. @export rebuild → initial mount, mutations written to buffer
        3. JS reads buffer, applies to DOM
        4. DOM events → @export handle_event → dirty scopes marked
        5. @export flush → re-render + diff → mutations written
        6. JS reads buffer, applies to DOM
        7. Repeat 4-6 until page unload
    """

    var _config: AppConfig
    var _initialized: Bool

    fn __init__(out self, config: AppConfig = AppConfig()):
        """Create a new WebApp with the given configuration.

        Args:
            config: Application configuration. On the web target, `title`
                    can be used to set document.title (via JS), and `debug`
                    enables console logging of mutation traffic.
        """
        self._config = config
        self._initialized = False

    fn __init__(
        out self,
        title: String,
        width: Int = 800,
        height: Int = 600,
        debug: Bool = False,
    ):
        """Convenience constructor matching DesktopApp's signature.

        Args:
            title: Page/document title.
            width: Ignored on web (viewport is controlled by the browser).
            height: Ignored on web (viewport is controlled by the browser).
            debug: Enable debug logging in the JS runtime.
        """
        self._config = AppConfig(title, width, height, debug)
        self._initialized = False

    fn __moveinit__(out self, deinit other: Self):
        self._config = other._config^
        self._initialized = other._initialized

    # ── PlatformApp trait implementation ──────────────────────────────

    fn init(mut self) raises:
        """Register web platform features.

        On the WASM target, platform initialization is handled by the JS
        runtime (DOM setup, WASM instantiation, memory management). This
        method registers the web feature set so that framework code can
        query platform capabilities.

        This is idempotent — safe to call multiple times.
        """
        if self._initialized:
            return

        # Register web platform capabilities.
        register_features(web_features())

        self._initialized = True

    fn flush_mutations(mut self, buf: UnsafePointer[UInt8], length: Int) raises:
        """No-op on WASM — the JS runtime reads the mutation buffer directly.

        On the web target, the mutation buffer lives in WASM linear memory.
        The JS Interpreter reads it directly after each @export flush call
        returns. There is no need to explicitly "send" the buffer — the JS
        side knows the buffer pointer and reads `length` bytes from it.

        The @export flush wrapper in main.mojo returns the byte length to
        the JS caller, which then calls `interp.applyMutations(mem, ptr, len)`.

        Args:
            buf: Pointer to the mutation buffer in WASM linear memory.
            length: Number of valid bytes written by MutationWriter.
        """
        # On WASM, the JS side reads the buffer directly from shared memory.
        # Nothing to do here — the @export return value signals the JS side.
        pass

    fn request_animation_frame(mut self):
        """No-op on WASM — the JS runtime manages requestAnimationFrame.

        The browser's event loop and requestAnimationFrame scheduling are
        handled entirely by the JS runtime. The WASM side doesn't need to
        explicitly request frames — the JS EventBridge automatically calls
        flush after each event dispatch.
        """
        pass

    fn should_quit(self) -> Bool:
        """Always returns False on WASM — the browser tab lifecycle is separate.

        The WASM module runs until the page is unloaded or the app is
        explicitly destroyed via the JS handle.destroy() method. There is
        no concept of "quitting" in the browser — the tab remains open.
        """
        return False

    fn destroy(mut self):
        """No-op on WASM — memory is reclaimed by the browser on page unload.

        The JS runtime handles cleanup via the AppHandle.destroy() method,
        which frees the mutation buffer and calls @export destroy on the
        WASM side. The WebApp struct itself has no platform resources to
        release.
        """
        self._initialized = False

    # ── Web-specific helpers ──────────────────────────────────────────

    fn config(self) -> AppConfig:
        """Return the application configuration.

        Useful for @export init wrappers that need to access the config
        (e.g., to set document.title via JS eval).
        """
        return self._config

    fn is_initialized(self) -> Bool:
        """Return True if init() has been called.

        The @export wrappers can check this to ensure the platform
        abstraction layer has been properly set up.
        """
        return self._initialized

    fn is_debug(self) -> Bool:
        """Return True if debug mode is enabled.

        When True, the JS runtime logs mutation buffer traffic and event
        dispatch to the browser console. Useful for development.
        """
        return self._config.debug


# ══════════════════════════════════════════════════════════════════════════════
# Module-level helpers
# ══════════════════════════════════════════════════════════════════════════════


fn create_web_app() -> WebApp:
    """Create a WebApp using the global launch configuration.

    If launch() has been called, uses the stored AppConfig.
    Otherwise, creates a WebApp with default configuration.

    This is the recommended way to create a WebApp from @export init
    wrappers, as it respects any configuration set by the app's main()
    function via launch().
    """
    return WebApp(get_launch_config())


fn create_web_app(title: String, debug: Bool = False) -> WebApp:
    """Create a WebApp with explicit title and debug settings.

    Args:
        title: Document/page title.
        debug: Enable debug logging.

    Returns:
        A configured WebApp instance.
    """
    return WebApp(AppConfig(title=title, debug=debug))
