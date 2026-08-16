# Platform Launch — Compile-time target-dispatching entry point.
#
# The launch() function is the single entry point that all mojo-gui apps use.
# The renderer is selected at **compile time** based on the build target:
#
#   - WASM target → web renderer (JS runtime drives the event loop)
#   - XR target   → XR renderer (OpenXR frame loop drives rendering)
#   - Native target → desktop renderer (Blitz drives the event loop)
#
# This is the key enabler for shared examples. App code calls launch() and
# never imports a specific renderer. The build target determines which
# renderer is used.
#
# Usage in a shared example app:
#
#     from platform import launch, AppConfig
#     from counter import CounterApp
#
#     fn main() raises:
#         launch[CounterApp](AppConfig(
#             title="Counter",
#             width=400,
#             height=350,
#         ))
#
# Build for different targets:
#
#     # Web (WASM):
#     mojo build app.mojo --target wasm64-wasi -I core/src -I web/src
#
#     # Desktop (native):
#     mojo build app.mojo -I core/src -I desktop/src
#
#     # XR (native + OpenXR):
#     mojo build app.mojo -D MOJO_TARGET_XR -I core/src -I xr/native/src
#
# The same source file, different build targets — the framework handles
# the rest.
#
# Design notes:
#
#   - launch() is parametric on the GuiApp type. The type parameter tells
#     the framework which app to instantiate and run.
#
#   - For WASM targets, launch() returns immediately. The JS runtime
#     drives the event loop; @export wrappers in main.mojo use the
#     generic gui_app_exports helpers to call GuiApp trait methods.
#
#   - For native targets, launch() passes the config directly to
#     desktop_launch[AppType](config), which creates the Blitz renderer
#     window, mounts the initial DOM, and enters the platform event loop
#     (blocking until the window is closed).
#
#   - For XR targets (compile with -D MOJO_TARGET_XR), launch() passes
#     the config to xr_launch[AppType](config), which creates an OpenXR
#     session, wraps the app in a single floating panel, mounts the DOM,
#     and enters the XR frame loop (blocking until the session ends).
#
#   - The compile-time dispatch uses @parameter if with is_wasm_target()
#     so that only the relevant renderer code is compiled for each target.
#     This avoids pulling in desktop dependencies for WASM builds and
#     vice versa.
#
# Module-level var workaround:
#
#   Mojo does not support module-level `var` on native targets. Previous
#   versions used `var _global_config: AppConfig` at module scope, which
#   compiled fine for WASM but failed on native.
#
#   The current design avoids global mutable state entirely:
#     - On native: config is passed directly to desktop_launch() as an
#       argument. No global storage needed.
#     - On WASM: @export wrappers receive config through compile-time
#       type parameters and constructor arguments.
#     - get_launch_config() and has_launched() return defaults on both
#       targets for API compatibility. Callers should use the config
#       passed directly to them rather than relying on global state.
#
# Step 3.9.3: launch() uses @parameter if is_wasm_target() for
# compile-time target dispatch. On WASM, it returns immediately
# (JS drives the loop). On XR native, it calls xr_launch[AppType]().
# On desktop native, it calls desktop_launch[AppType]() with the
# Blitz desktop renderer.

from .app import is_wasm_target, is_native_target, is_xr_target
from .gui_app import GuiApp


# ══════════════════════════════════════════════════════════════════════════════
# AppConfig — Optional configuration for launch()
# ══════════════════════════════════════════════════════════════════════════════


struct AppConfig(Copyable, Movable):
    """Configuration for the application launcher.

    Provides platform-independent configuration that renderers interpret
    according to their capabilities.

    Fields:
        title: Window title (desktop/native) or document title (web).
        width: Initial viewport width in logical pixels.
        height: Initial viewport height in logical pixels.
        debug: Enable developer tools / debug overlays.
    """

    var title: String
    var width: Int
    var height: Int
    var debug: Bool

    fn __init__(
        out self,
        title: String = "mojo-gui",
        width: Int = 800,
        height: Int = 600,
        debug: Bool = False,
    ):
        self.title = title
        self.width = width
        self.height = height
        self.debug = debug

    fn __copyinit__(out self, other: Self):
        self.title = other.title
        self.width = other.width
        self.height = other.height
        self.debug = other.debug

    fn __moveinit__(out self, deinit other: Self):
        self.title = other.title^
        self.width = other.width
        self.height = other.height
        self.debug = other.debug


# ══════════════════════════════════════════════════════════════════════════════
# Config accessors (API compatibility)
# ══════════════════════════════════════════════════════════════════════════════
#
# These functions exist for backwards compatibility with WebApp and other
# renderer infrastructure. They return defaults because module-level `var`
# is not supported on native targets, making global config storage
# impractical.
#
# Callers should prefer using the config passed directly to them:
#   - WebApp receives config via its __init__ constructor
#   - gui_app_exports receive config via compile-time type parameters
#   - desktop_launch() receives config as a direct argument from launch()


fn get_launch_config() -> AppConfig:
    """Retrieve the AppConfig set by the most recent launch() call.

    Note: This returns the default AppConfig on all targets. The config
    is passed directly to renderers via launch() arguments rather than
    stored globally, because module-level `var` is not supported on
    native targets.

    For the WASM target, use the config passed to WebApp's constructor
    or to the @export wrappers directly. For the native target,
    desktop_launch() receives the config as an argument.

    Returns:
        The default AppConfig.
    """
    return AppConfig()


fn has_launched() -> Bool:
    """Return True if launch() has been called.

    Note: This always returns False because module-level `var` is not
    supported on native targets. Renderer infrastructure should not
    depend on this function for correctness — it's a diagnostic hint.

    Returns:
        False.
    """
    return False


# ══════════════════════════════════════════════════════════════════════════════
# launch() — The universal entry point
# ══════════════════════════════════════════════════════════════════════════════


fn launch[AppType: GuiApp](config: AppConfig = AppConfig()) raises:
    """Launch the mojo-gui application on the current platform.

    This is the universal entry point for all mojo-gui apps. The renderer
    is selected at compile time based on the build target via
    `@parameter if is_wasm_target()`.

    Type Parameters:
        AppType: A concrete type implementing the GuiApp trait. This is
                 the app to instantiate and run.

    Args:
        config: Optional application configuration (title, size, debug).

    On WASM targets:
        Returns immediately. The JS runtime drives the event loop;
        @export wrappers in main.mojo use gui_app_exports helpers to
        call GuiApp trait methods. The config is available to @export
        init functions via the AppType parameter.

    On native targets:
        Calls desktop_launch[AppType](config) which creates the Blitz
        renderer window, mounts the initial DOM, and enters the platform
        event loop (blocking until the window is closed). The config is
        passed directly — no global state needed.

    Example (shared app entry point):

        from platform import launch, AppConfig
        from counter import CounterApp

        fn main() raises:
            launch[CounterApp](AppConfig(
                title="My Counter",
                width=400,
                height=350,
                debug=True,
            ))

    The same source compiles for web (--target wasm64-wasi) and desktop
    (native). The GuiApp trait ensures the renderer can drive the app
    uniformly regardless of platform.
    """

    @parameter
    if is_wasm_target():
        # Web path: JS runtime drives the event loop.
        # The @export wrappers in main.mojo use gui_app_exports helpers
        # (gui_app_init[AppType], gui_app_mount[AppType], etc.) to call
        # GuiApp trait methods. Nothing more to do here.
        pass
    elif is_xr_target():
        # XR path: create OpenXR session + panel and enter XR frame loop.
        # xr_launch creates the session, wraps the app in a default panel,
        # mounts the initial DOM, and enters the XR frame loop (blocking
        # until the session ends). The app never sees the XR infrastructure.
        from xr.launcher import xr_launch

        xr_launch[AppType](config)
    else:
        # Desktop path: create Blitz window and enter event loop.
        # desktop_launch creates the native window, mounts the initial
        # DOM, and enters a blocking event loop until the window is closed.
        # The config is passed directly — no need for global state.
        from desktop.launcher import desktop_launch

        desktop_launch[AppType](config)
