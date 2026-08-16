# Platform Features — Runtime feature detection for platform capabilities.
#
# This module provides a capability query system that lets app code and
# framework internals check what the current platform supports at runtime.
# Unlike compile-time target detection (is_wasm_target / is_native_target),
# feature detection answers questions about *runtime* capabilities that may
# vary even within a single target family.
#
# Examples of features that vary by renderer:
#
#   - DOM manipulation (web: native DOM, desktop: Blitz DOM, native: none)
#   - CSS styling (web: full browser CSS, desktop: Stylo subset, native: themes)
#   - Clipboard access (web: async Clipboard API, desktop: OS clipboard, native: OS)
#   - File system access (web: limited/sandboxed, desktop: full, native: full)
#   - Network access (web: fetch/XHR with CORS, desktop: unrestricted, native: unrestricted)
#   - GPU rendering (web: WebGL/WebGPU, desktop: Vello, native: platform compositor)
#   - Accessibility (web: ARIA, desktop: AccessKit, native: platform a11y)
#   - Multi-window (web: no, desktop: yes via Winit, native: yes)
#
# Usage:
#
#     from platform.features import PlatformFeatures, current_features
#
#     var features = current_features()
#     if features.has_clipboard:
#         # Enable paste button
#         pass
#     if features.has_multi_window:
#         # Enable "Open in new window" menu item
#         pass
#
# Design notes:
#
#   - Features are detected once at startup and cached in a struct.
#   - The struct is plain data (no pointers, no heap) — safe to copy.
#   - Renderers call register_features() during init() to advertise
#     their capabilities. App code reads via current_features().
#   - Unknown features default to False (conservative).
#   - This is intentionally a flat struct rather than a dynamic map
#     for zero-overhead field access in Mojo.

from .app import is_wasm_target


# ══════════════════════════════════════════════════════════════════════════════
# PlatformFeatures — Capability descriptor
# ══════════════════════════════════════════════════════════════════════════════


struct PlatformFeatures(Copyable, Movable):
    """Describes the capabilities of the current platform and renderer.

    Each field is a Bool indicating whether the feature is available.
    Renderers populate this struct during initialization; app code and
    framework internals query it to adapt behavior.

    All features default to False. Renderers set True for features they
    support. App code should treat False as "not available or unknown"
    and degrade gracefully.
    """

    # ── Rendering capabilities ────────────────────────────────────────

    var has_dom: Bool
    """True if the renderer provides a DOM-like tree (web, desktop/Blitz).
    False for native widget renderers that map to platform controls."""

    var has_css: Bool
    """True if CSS styling is supported (web: full browser CSS,
    desktop: Stylo/Blitz subset). False for native widget renderers."""

    var has_gpu: Bool
    """True if GPU-accelerated rendering is available (web: WebGL/WebGPU,
    desktop: Vello). May be False on headless or software-only backends."""

    # ── Window management ─────────────────────────────────────────────

    var has_multi_window: Bool
    """True if multiple windows can be created (desktop, native).
    False for web (single tab context)."""

    var has_native_chrome: Bool
    """True if the renderer provides native window chrome (title bar,
    resize handles, system menus). True for desktop and native renderers."""

    # ── I/O capabilities ──────────────────────────────────────────────

    var has_clipboard: Bool
    """True if clipboard read/write is available."""

    var has_filesystem: Bool
    """True if unrestricted filesystem access is available.
    Web has limited/sandboxed access; desktop and native have full access."""

    var has_unrestricted_network: Bool
    """True if network requests are unrestricted (no CORS, no sandbox).
    Desktop and native have full access; web is subject to CORS."""

    # ── Accessibility ─────────────────────────────────────────────────

    var has_accessibility: Bool
    """True if the platform's accessibility tree is connected.
    Web: ARIA via browser. Desktop: AccessKit via Blitz. Native: OS a11y."""

    # ── XR capabilities ───────────────────────────────────────────────

    var has_xr: Bool
    """True if the renderer is running in an XR (extended reality) session.
    XR renderers place DOM panels in 3D space via OpenXR (native) or
    WebXR (browser). False for flat desktop and web renderers."""

    var has_xr_hand_tracking: Bool
    """True if XR hand tracking is available (XR_EXT_hand_tracking).
    Enables hand-based input instead of or in addition to controllers.
    Always False when has_xr is False."""

    var has_xr_passthrough: Bool
    """True if XR passthrough (AR mode) is available (XR_FB_passthrough
    or equivalent). Enables mixed reality where virtual panels are
    overlaid on the real world. Always False when has_xr is False."""

    # ── Platform identity ─────────────────────────────────────────────

    var renderer_name: String
    """Human-readable name of the active renderer.
    Examples: "web", "desktop-blitz", "native"."""

    # ── Constructor ───────────────────────────────────────────────────

    fn __init__(out self):
        """Create a PlatformFeatures with all capabilities set to False.

        Renderers should create an instance, set the appropriate fields
        to True, and call register_features().
        """
        self.has_dom = False
        self.has_css = False
        self.has_gpu = False
        self.has_multi_window = False
        self.has_native_chrome = False
        self.has_clipboard = False
        self.has_filesystem = False
        self.has_unrestricted_network = False
        self.has_accessibility = False
        self.has_xr = False
        self.has_xr_hand_tracking = False
        self.has_xr_passthrough = False
        self.renderer_name = String("unknown")

    fn __copyinit__(out self, other: Self):
        self.has_dom = other.has_dom
        self.has_css = other.has_css
        self.has_gpu = other.has_gpu
        self.has_multi_window = other.has_multi_window
        self.has_native_chrome = other.has_native_chrome
        self.has_clipboard = other.has_clipboard
        self.has_filesystem = other.has_filesystem
        self.has_unrestricted_network = other.has_unrestricted_network
        self.has_accessibility = other.has_accessibility
        self.has_xr = other.has_xr
        self.has_xr_hand_tracking = other.has_xr_hand_tracking
        self.has_xr_passthrough = other.has_xr_passthrough
        self.renderer_name = other.renderer_name

    fn __moveinit__(out self, deinit other: Self):
        self.has_dom = other.has_dom
        self.has_css = other.has_css
        self.has_gpu = other.has_gpu
        self.has_multi_window = other.has_multi_window
        self.has_native_chrome = other.has_native_chrome
        self.has_clipboard = other.has_clipboard
        self.has_filesystem = other.has_filesystem
        self.has_unrestricted_network = other.has_unrestricted_network
        self.has_accessibility = other.has_accessibility
        self.has_xr = other.has_xr
        self.has_xr_hand_tracking = other.has_xr_hand_tracking
        self.has_xr_passthrough = other.has_xr_passthrough
        self.renderer_name = other.renderer_name^


# ══════════════════════════════════════════════════════════════════════════════
# Preset feature sets — convenience constructors for known renderers
# ══════════════════════════════════════════════════════════════════════════════


fn web_features() -> PlatformFeatures:
    """Return the feature set for the web (WASM + browser) renderer.

    Web has full DOM and CSS support but is sandboxed: no multi-window,
    restricted filesystem and network (CORS), clipboard via async API.
    """
    var f = PlatformFeatures()
    f.has_dom = True
    f.has_css = True
    f.has_gpu = True  # WebGL/WebGPU available in modern browsers
    f.has_multi_window = False
    f.has_native_chrome = False
    f.has_clipboard = True  # Async Clipboard API
    f.has_filesystem = False  # Sandboxed
    f.has_unrestricted_network = False  # CORS
    f.has_accessibility = True  # Browser ARIA
    f.has_xr = False
    f.has_xr_hand_tracking = False
    f.has_xr_passthrough = False
    f.renderer_name = String("web")
    return f


fn desktop_blitz_features() -> PlatformFeatures:
    """Return the feature set for the desktop Blitz renderer.

    Blitz provides DOM/CSS via Stylo + Taffy + Vello, native chrome via
    Winit, and accessibility via AccessKit. Full native I/O capabilities.
    """
    var f = PlatformFeatures()
    f.has_dom = True
    f.has_css = True  # Stylo (Firefox CSS engine)
    f.has_gpu = True  # Vello GPU rendering
    f.has_multi_window = True
    f.has_native_chrome = True
    f.has_clipboard = True
    f.has_filesystem = True
    f.has_unrestricted_network = True
    f.has_accessibility = True  # AccessKit
    f.has_xr = False
    f.has_xr_hand_tracking = False
    f.has_xr_passthrough = False
    f.renderer_name = String("desktop-blitz")
    return f


fn native_features() -> PlatformFeatures:
    """Return the feature set for the native widget renderer (future).

    Native renderers map DOM mutations to platform widgets (Cocoa, Win32,
    etc.). They don't have a DOM or CSS engine — styling is via the
    platform's native theme system.
    """
    var f = PlatformFeatures()
    f.has_dom = False  # Platform widgets, not DOM
    f.has_css = False  # Platform themes, not CSS
    f.has_gpu = True  # Platform compositor
    f.has_multi_window = True
    f.has_native_chrome = True
    f.has_clipboard = True
    f.has_filesystem = True
    f.has_unrestricted_network = True
    f.has_accessibility = True  # Platform a11y
    f.has_xr = False
    f.has_xr_hand_tracking = False
    f.has_xr_passthrough = False
    f.renderer_name = String("native")
    return f


fn xr_native_features() -> PlatformFeatures:
    """Return the feature set for the OpenXR native renderer.

    The XR native renderer uses the Blitz stack (same as desktop) but
    renders to offscreen textures composited via OpenXR quad layers.
    It has full native I/O capabilities plus XR-specific features.

    XR hand tracking and passthrough depend on the runtime and are set
    to False by default. The XR launcher updates these after querying
    the OpenXR runtime's extension support at session creation time.
    """
    var f = PlatformFeatures()
    f.has_dom = True
    f.has_css = True  # Stylo (same as desktop)
    f.has_gpu = True  # Vello GPU rendering → offscreen textures
    f.has_multi_window = False  # XR uses panels, not OS windows
    f.has_native_chrome = False  # No OS window chrome in XR
    f.has_clipboard = True
    f.has_filesystem = True
    f.has_unrestricted_network = True
    f.has_accessibility = True  # AccessKit per-panel
    f.has_xr = True
    f.has_xr_hand_tracking = False  # Updated after runtime query
    f.has_xr_passthrough = False  # Updated after runtime query
    f.renderer_name = String("xr-native")
    return f


fn xr_web_features() -> PlatformFeatures:
    """Return the feature set for the WebXR browser renderer.

    The WebXR renderer extends the web renderer with XR session management
    and DOM-to-texture panel rendering. It inherits the web renderer's
    sandboxing constraints but adds XR capabilities.
    """
    var f = PlatformFeatures()
    f.has_dom = True
    f.has_css = True
    f.has_gpu = True  # WebGL/WebGPU
    f.has_multi_window = False
    f.has_native_chrome = False
    f.has_clipboard = True  # Async Clipboard API
    f.has_filesystem = False  # Sandboxed
    f.has_unrestricted_network = False  # CORS
    f.has_accessibility = True  # Browser ARIA
    f.has_xr = True
    f.has_xr_hand_tracking = False  # Updated after runtime query
    f.has_xr_passthrough = False  # Updated after runtime query
    f.renderer_name = String("xr-web")
    return f


# ══════════════════════════════════════════════════════════════════════════════
# Global feature registry
# ══════════════════════════════════════════════════════════════════════════════
#
# The active renderer registers its features during init(). App code
# queries via current_features(). This is a simple global — there is
# only one active renderer per process.

var _current_features: PlatformFeatures = PlatformFeatures()
var _features_registered: Bool = False


fn register_features(features: PlatformFeatures):
    """Register the active renderer's feature set.

    Called by the renderer during its init() phase. Subsequent calls
    overwrite the previous registration (last renderer wins).

    Args:
        features: The feature set advertised by the renderer.
    """
    _current_features = features
    _features_registered = True


fn current_features() -> PlatformFeatures:
    """Return the currently registered platform features.

    If no renderer has registered features yet, returns a default
    PlatformFeatures with all capabilities set to False.

    Apps should call this after launch() / renderer init() to get
    accurate feature information.
    """
    return _current_features


fn features_registered() -> Bool:
    """Return True if a renderer has registered its features.

    Useful for framework internals to detect whether platform
    initialization has completed.
    """
    return _features_registered


fn default_features() -> PlatformFeatures:
    """Return a default feature set based on compile-time target detection.

    This provides a best-guess feature set without requiring renderer
    registration. Useful for early startup code that runs before the
    renderer is initialized.

    For WASM targets, returns web_features().
    For native targets, returns desktop_blitz_features() (the default
    desktop renderer).
    """
    if is_wasm_target():
        return web_features()
    else:
        return desktop_blitz_features()
