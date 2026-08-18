// WebXR Runtime — Module re-exports.
//
// Public API surface for the WebXR browser renderer (Step 5.6).
//
// This module aggregates and re-exports all WebXR runtime types,
// classes, and utilities so consumers can import from a single path:
//
//   import { XRRuntime, defaultXRRuntimeConfig } from "./xr/web/runtime/mod.ts";
//
// Architecture:
//
//   xr-types.ts     — Type definitions, math primitives, panel config presets
//   xr-session.ts   — WebXR session lifecycle, reference spaces, GL setup
//   xr-panel.ts     — Panel DOM containers, texture capture, raycasting, layout
//   xr-renderer.ts  — WebGL2 textured quad drawing for XR panels
//   xr-input.ts     — XR input sources → raycasting → DOM pointer events
//   xr-runtime.ts   — Main entry point tying all subsystems together

// ── Types & Config ──────────────────────────────────────────────────────────

export type {
	Mat4,
	PanelConfig,
	PanelState,
	Quaternion,
	RaycastHit,
	Vec3,
	XRHandedness,
	XRInputRay,
	XRPanelDescriptor,
	XRRuntimeConfig,
	XRRuntimeEvent,
	XRRuntimeEventListener,
	XRSessionMode,
} from "./xr-types.ts";

export {
	dashboardPanelConfig,
	defaultPanelConfig,
	defaultXRRuntimeConfig,
	handAnchoredPanelConfig,
	tooltipPanelConfig,
	XRInputEventType,
} from "./xr-types.ts";

// ── Compat types (for environments without WebXR typings) ───────────────────

export type {
	XRFrameCompat,
	XRInputSourceCompat,
	XRPoseCompat,
	XRReferenceSpaceCompat,
	XRRigidTransformCompat,
	XRSessionCompat,
	XRSpaceCompat,
	XRSystemCompat,
	XRViewCompat,
	XRViewerPoseCompat,
	XRWebGLLayerCompat,
	XRWebGLLayerConstructor,
} from "./xr-types.ts";

// ── Session Manager ─────────────────────────────────────────────────────────

export type {
	SessionStartOptions,
	SessionStateName,
	XRFrameCallback,
} from "./xr-session.ts";
export { SessionState, XRSessionManager } from "./xr-session.ts";

// ── Panel Manager ───────────────────────────────────────────────────────────

export { XRPanel, XRPanelManager } from "./xr-panel.ts";

// ── Quad Renderer ───────────────────────────────────────────────────────────

export { XRQuadRenderer } from "./xr-renderer.ts";

// ── Input Handler ───────────────────────────────────────────────────────────

export type {
	XRPointerEventCallback,
	XRPointerEventNameType,
} from "./xr-input.ts";
export { XRInputHandler, XRPointerEventName } from "./xr-input.ts";

// ── Runtime (main entry point) ──────────────────────────────────────────────

export type {
	AppPanelConfig,
	AppPanelHandle,
	RuntimeStateName,
} from "./xr-runtime.ts";
export { RuntimeState, XRRuntime } from "./xr-runtime.ts";
