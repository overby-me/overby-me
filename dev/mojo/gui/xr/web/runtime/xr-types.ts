// WebXR Runtime — Type Definitions
//
// TypeScript types for the WebXR panel renderer. These types mirror the
// native XR panel types (xr/native/src/xr/panel.mojo) but are adapted
// for the browser WebXR environment.
//
// The WebXR runtime reuses the existing web mutation interpreter and
// event bridge from web/runtime/, adding XR session management,
// DOM-to-texture rendering, and spatial input handling on top.

// ── Math Primitives ─────────────────────────────────────────────────────────
// Mirror Vec3 and Quaternion from xr/native/src/xr/panel.mojo

/** 3D vector for positions, scales, and directions in world space (meters). */
export interface Vec3 {
	x: number;
	y: number;
	z: number;
}

/** Quaternion rotation (unit quaternion, Hamilton convention). */
export interface Quaternion {
	x: number;
	y: number;
	z: number;
	w: number;
}

/** 4×4 column-major transform matrix (Float32Array of length 16). */
export type Mat4 = Float32Array;

// ── Panel Configuration ─────────────────────────────────────────────────────
// Mirrors PanelConfig from xr/native/src/xr/panel.mojo

/** Configuration for creating an XR panel. */
export interface PanelConfig {
	/** Physical width in meters (default: 0.8). */
	widthM: number;

	/** Physical height in meters (default: 0.6). */
	heightM: number;

	/** Pixels per meter — determines texture resolution and text legibility (default: 1200). */
	pixelsPerMeter: number;

	/** Whether the panel surface is curved (cylindrical). Default: false. */
	curved: boolean;

	/** Whether the panel accepts pointer input (raycasting). Default: true. */
	interact: boolean;
}

/** Preset: general-purpose reading panel (~27" monitor equivalent). */
export function defaultPanelConfig(): PanelConfig {
	return {
		widthM: 0.8,
		heightM: 0.6,
		pixelsPerMeter: 1200,
		curved: false,
		interact: true,
	};
}

/** Preset: wide curved dashboard. */
export function dashboardPanelConfig(): PanelConfig {
	return {
		widthM: 1.6,
		heightM: 0.9,
		pixelsPerMeter: 1000,
		curved: true,
		interact: true,
	};
}

/** Preset: small non-interactive HUD overlay. */
export function tooltipPanelConfig(): PanelConfig {
	return {
		widthM: 0.3,
		heightM: 0.15,
		pixelsPerMeter: 800,
		curved: false,
		interact: false,
	};
}

/** Preset: panel attached to a controller or hand. */
export function handAnchoredPanelConfig(): PanelConfig {
	return {
		widthM: 0.2,
		heightM: 0.15,
		pixelsPerMeter: 1400,
		curved: false,
		interact: true,
	};
}

// ── Panel State ─────────────────────────────────────────────────────────────

/** Runtime state of an XR panel. */
export interface PanelState {
	/** Whether the panel is visible in the scene. */
	visible: boolean;

	/** Whether this panel currently has input focus. */
	focused: boolean;

	/** Whether the panel's DOM has changed since last texture update. */
	dirty: boolean;

	/** Whether the panel's app has been mounted (initial render done). */
	mounted: boolean;
}

// ── XR Panel ────────────────────────────────────────────────────────────────

/** An XR panel instance — a 2D DOM document placed in 3D XR space. */
export interface XRPanelDescriptor {
	/** Unique panel ID (assigned by the runtime). */
	id: number;

	/** Panel configuration (immutable after creation). */
	config: Readonly<PanelConfig>;

	/** World-space position (meters). */
	position: Vec3;

	/** World-space rotation (unit quaternion). */
	rotation: Quaternion;

	/** Runtime state. */
	state: PanelState;

	/** Texture width in pixels (derived: config.widthM × config.pixelsPerMeter). */
	textureWidth: number;

	/** Texture height in pixels (derived: config.heightM × config.pixelsPerMeter). */
	textureHeight: number;
}

// ── Raycast Hit ─────────────────────────────────────────────────────────────

/** Result of raycasting an XR input ray against panel quads. */
export interface RaycastHit {
	/** ID of the panel that was hit. */
	panelId: number;

	/** Distance from ray origin to the hit point (meters). */
	distance: number;

	/** 2D hit point on the panel surface, normalized [0,1] from top-left. */
	uv: { u: number; v: number };

	/** Hit point in pixel coordinates (for DOM event dispatch). */
	pixel: { x: number; y: number };
}

// ── XR Input ────────────────────────────────────────────────────────────────

/** XR input source handedness. */
export type XRHandedness = "none" | "left" | "right";

/** Represents an XR input ray for raycasting against panels. */
export interface XRInputRay {
	/** Ray origin in world space (meters). */
	origin: Vec3;

	/** Ray direction (unit vector). */
	direction: Vec3;

	/** Which hand/controller this ray belongs to. */
	handedness: XRHandedness;
}

/** XR input event types dispatched to panels. */
export const XRInputEventType = {
	/** Controller trigger / hand pinch — maps to DOM "click". */
	Select: "select",
	/** Controller trigger start / hand pinch start — maps to DOM "mousedown". */
	SelectStart: "selectstart",
	/** Controller trigger end / hand pinch end — maps to DOM "mouseup". */
	SelectEnd: "selectend",
	/** Controller squeeze / hand grab. */
	Squeeze: "squeeze",
	/** Continuous hover — maps to DOM "mousemove" / "mouseenter" / "mouseleave". */
	Hover: "hover",
} as const;

export type XRInputEventTypeName =
	(typeof XRInputEventType)[keyof typeof XRInputEventType];

// ── Session Configuration ───────────────────────────────────────────────────

/** WebXR session mode. */
export type XRSessionMode = "immersive-vr" | "immersive-ar" | "inline";

/** Configuration for the WebXR runtime. */
export interface XRRuntimeConfig {
	/** WebXR session mode (default: "immersive-vr"). */
	sessionMode: XRSessionMode;

	/**
	 * Required WebXR features.
	 * Default: ["local-floor"].
	 */
	requiredFeatures: string[];

	/**
	 * Optional WebXR features.
	 * Default: ["hand-tracking", "bounded-floor"].
	 */
	optionalFeatures: string[];

	/**
	 * Whether to fall back to flat (non-XR) rendering when WebXR is
	 * unavailable. Default: true.
	 */
	fallbackToFlat: boolean;

	/**
	 * CSS background color for panels (applied to each panel's root).
	 * Default: "#ffffff".
	 */
	panelBackground: string;

	/**
	 * Whether to show a "Enter VR" button when XR is available.
	 * Default: true.
	 */
	showEnterVRButton: boolean;

	/**
	 * Target texture update rate in Hz for dirty panels.
	 * Texture capture is expensive; this throttles re-rasterization.
	 * Default: 30 (update textures at 30fps; XR frame loop still runs at native rate).
	 */
	textureUpdateRate: number;
}

/** Default XR runtime configuration. */
export function defaultXRRuntimeConfig(): XRRuntimeConfig {
	return {
		sessionMode: "immersive-vr",
		requiredFeatures: ["local-floor"],
		optionalFeatures: ["hand-tracking", "bounded-floor"],
		fallbackToFlat: true,
		panelBackground: "#ffffff",
		showEnterVRButton: true,
		textureUpdateRate: 30,
	};
}

// ── WebXR API Type Augmentation ─────────────────────────────────────────────
// Minimal type declarations for WebXR APIs that may not be in all TS libs.
// These supplement lib.dom.d.ts for environments without WebXR typings.

/** Detect whether WebXR types are available at the type level. */
export interface XRSystemCompat {
	isSessionSupported(mode: string): Promise<boolean>;
	requestSession(
		mode: string,
		options?: {
			requiredFeatures?: string[];
			optionalFeatures?: string[];
		},
	): Promise<XRSessionCompat>;
}

export interface XRSessionCompat {
	readonly renderState: { baseLayer?: XRWebGLLayerCompat | null };
	readonly inputSources: Iterable<XRInputSourceCompat>;
	readonly visibilityState: string;
	requestAnimationFrame(
		callback: (time: number, frame: XRFrameCompat) => void,
	): number;
	cancelAnimationFrame(handle: number): void;
	requestReferenceSpace(type: string): Promise<XRReferenceSpaceCompat>;
	updateRenderState(state: { baseLayer?: XRWebGLLayerCompat }): void;
	end(): Promise<void>;
	addEventListener(
		type: string,
		listener: EventListenerOrEventListenerObject,
	): void;
	removeEventListener(
		type: string,
		listener: EventListenerOrEventListenerObject,
	): void;
}

export interface XRFrameCompat {
	readonly session: XRSessionCompat;
	getViewerPose(
		referenceSpace: XRReferenceSpaceCompat,
	): XRViewerPoseCompat | null;
	getPose(
		space: XRSpaceCompat,
		baseSpace: XRReferenceSpaceCompat,
	): XRPoseCompat | null;
}

export interface XRViewerPoseCompat {
	readonly views: readonly XRViewCompat[];
	readonly transform: XRRigidTransformCompat;
}

export interface XRViewCompat {
	readonly eye: string;
	readonly projectionMatrix: Float32Array;
	readonly transform: XRRigidTransformCompat;
}

export interface XRRigidTransformCompat {
	readonly position: DOMPointReadOnly;
	readonly orientation: DOMPointReadOnly;
	readonly matrix: Float32Array;
	readonly inverse: XRRigidTransformCompat;
}

export interface XRReferenceSpaceCompat {
	getOffsetReferenceSpace(
		originOffset: XRRigidTransformCompat,
	): XRReferenceSpaceCompat;
}

export type XRSpaceCompat = Record<string, never>;

export interface XRPoseCompat {
	readonly transform: XRRigidTransformCompat;
}

export interface XRInputSourceCompat {
	readonly handedness: XRHandedness;
	readonly targetRayMode: string;
	readonly targetRaySpace: XRSpaceCompat;
	readonly gripSpace?: XRSpaceCompat;
	readonly profiles: readonly string[];
}

export interface XRWebGLLayerCompat {
	readonly framebuffer: WebGLFramebuffer | null;
	readonly framebufferWidth: number;
	readonly framebufferHeight: number;
	getViewport(view: XRViewCompat): {
		x: number;
		y: number;
		width: number;
		height: number;
	} | null;
}

export interface XRWebGLLayerConstructor {
	new (
		session: XRSessionCompat,
		context: WebGLRenderingContext | WebGL2RenderingContext,
	): XRWebGLLayerCompat;
}

// ── Utility Types ───────────────────────────────────────────────────────────

/** Event emitted by the XR runtime for lifecycle changes. */
export type XRRuntimeEvent =
	| { type: "session-started" }
	| { type: "session-ended" }
	| { type: "visibility-changed"; state: string }
	| { type: "input-sources-changed" }
	| { type: "fallback-to-flat" }
	| { type: "error"; error: Error };

/** Callback for XR runtime events. */
export type XRRuntimeEventListener = (event: XRRuntimeEvent) => void;
