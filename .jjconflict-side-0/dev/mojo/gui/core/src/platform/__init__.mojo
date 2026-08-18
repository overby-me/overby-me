# Platform package — Platform abstraction layer for mojo-gui.
#
# This package provides the abstraction boundary between the renderer-agnostic
# core framework and platform-specific renderers (web, desktop, native).
#
# Sub-modules:
#   app       — PlatformApp trait, target detection helpers
#   gui_app   — GuiApp trait, app-side lifecycle contract
#   launch    — launch() entry point, AppConfig
#   features  — PlatformFeatures, runtime capability detection
#
# Note: Re-exporting the parametric `launch()` function through this
# __init__.mojo triggers a Mojo 0.26.1 bug where the function loses its
# parametric nature ("not subscriptable" error). This appears to be a
# compiler issue with re-exporting parametric functions that have complex
# trait-constrained type parameters through package init files.
#
# Workaround: Import `launch` directly from the submodule:
#
#     from platform.launch import launch, AppConfig
#
# All other symbols (GuiApp, PlatformApp, AppConfig, PlatformFeatures,
# etc.) re-export correctly from this package init.

from .app import (
    PlatformApp,
    is_wasm_target,
    is_native_target,
    is_xr_target,
)
from .gui_app import (
    GuiApp,
)
from .launch import (
    AppConfig,
    launch,
    get_launch_config,
    has_launched,
)
from .features import (
    PlatformFeatures,
    register_features,
    current_features,
    features_registered,
    default_features,
    web_features,
    desktop_blitz_features,
    native_features,
)
