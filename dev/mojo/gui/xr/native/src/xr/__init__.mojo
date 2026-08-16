"""mojo-gui/xr/native — XR renderer for mojo-gui applications.

This package provides an XR (extended reality) backend that renders
mojo-gui panels into 3D space via OpenXR. Each XR panel owns an
independent Blitz DOM document (reusing the same rendering stack as the
desktop renderer) rendered to an offscreen GPU texture by Vello. The
OpenXR compositor places these textures as quad layers in the XR scene.

Architecture:

    mojo-gui/core (MutationWriter)
        │ binary opcode buffer (per-panel)
        ▼
    xr/scene.mojo (XRScene — panel registry + event routing)
        │ routes mutations to the correct panel
        ▼
    xr/panel.mojo (XRPanel — 2D DOM document + 3D transform)
        │ owns a MutationInterpreter + Blitz document
        ▼
    xr/xr_blitz.mojo (XRBlitz FFI wrapper)
        │ DLHandle calls
        ▼
    libmojo_xr.so (Rust cdylib — xr/native/shim/src/lib.rs)
        │ Blitz DOM ops → Vello → offscreen texture per panel
        │ OpenXR session + frame loop + compositor
        ▼
    OpenXR Runtime → HMD

Modules:
    panel:        XRPanel, PanelConfig, PanelState, Vec3, Quaternion
    scene:        XRScene, XREvent, RaycastHit, layout helpers
    xr_blitz:     Mojo FFI bindings to libmojo_xr (XR Blitz shim)
    renderer:     XRMutationInterpreter (per-panel binary opcode interpreter)
    xr_launcher:  xr_launch[AppType: GuiApp]() — OpenXR entry point (future)

Usage (via the unified launch() entry point — single-panel apps):

    from platform import launch, AppConfig
    from counter import CounterApp

    fn main() raises:
        launch[CounterApp](AppConfig(title="Counter", width=400, height=350))

    # When compiled with --feature xr, launch() calls xr_launch[CounterApp]()
    # which creates an XR session, wraps the app in a default panel, and
    # enters the OpenXR frame loop.

Usage (direct, for multi-panel XR apps — future, Step 5.9):

    from xr.panel import XRPanel, PanelConfig, Vec3
    from xr.scene import XRScene, create_single_panel_scene
    from xr.xr_blitz import XRBlitz
    from xr.renderer import XRMutationInterpreter

Current Status:
    Step 5.1 — ✅ Complete. Panel and scene abstractions defined.
    Step 5.2 — 🔧 In progress. Real Blitz documents in shim (37 tests pass).
               Output-pointer FFI variants (_into) implemented.
    Step 5.3 — ✅ Complete. Mojo FFI bindings (XRBlitz) and per-panel
               mutation interpreter (XRMutationInterpreter) implemented.
    Step 5.4 — ✅ Complete. XRScene wired to XRBlitz FFI.
    Step 5.5 — ✅ Complete. xr_launch[AppType: GuiApp]() implemented.
"""

# ── Panel abstraction (Step 5.1) ─────────────────────────────────────────

from .panel import (
    XRPanel,
    PanelConfig,
    PanelState,
    Vec3,
    Quaternion,
    default_panel_config,
    dashboard_panel_config,
    tooltip_panel_config,
    hand_anchored_panel_config,
)

# ── Scene manager (Step 5.1) ─────────────────────────────────────────────

from .scene import (
    XRScene,
    XREvent,
    RaycastHit,
    create_single_panel_scene,
    create_dual_panel_scene,
    MAX_PANELS,
)

# ── FFI bindings (Step 5.3) ──────────────────────────────────────────────

from .xr_blitz import (
    XRBlitz,
    XREvent as XRBlitzEvent,
    XRPose,
    XRRaycastHit,
    # Event type constants
    EVT_CLICK,
    EVT_INPUT,
    EVT_CHANGE,
    EVT_KEYDOWN,
    EVT_KEYUP,
    EVT_FOCUS,
    EVT_BLUR,
    EVT_SUBMIT,
    EVT_MOUSEDOWN,
    EVT_MOUSEUP,
    EVT_MOUSEMOVE,
    EVT_XR_SELECT,
    EVT_XR_SQUEEZE,
    EVT_XR_HOVER_ENTER,
    EVT_XR_HOVER_EXIT,
    # Hand/controller identifiers
    HAND_LEFT,
    HAND_RIGHT,
    HAND_HEAD,
    # Reference space types
    SPACE_LOCAL,
    SPACE_STAGE,
    SPACE_VIEW,
    SPACE_UNBOUNDED,
    # Session state constants
    STATE_IDLE,
    STATE_READY,
    STATE_FOCUSED,
    STATE_VISIBLE,
    STATE_STOPPING,
    STATE_EXITING,
)

# ── Mutation interpreter (Step 5.3) ──────────────────────────────────────

from .renderer import XRMutationInterpreter

# ── XR launcher (Step 5.5) ───────────────────────────────────────────────

from .launcher import xr_launch
