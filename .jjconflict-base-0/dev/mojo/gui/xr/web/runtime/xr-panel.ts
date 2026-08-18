// XR Panel Manager — DOM subtree rendering, texture capture, and 3D placement.
//
// Each XR panel owns:
//   1. An offscreen DOM container (hidden div) where mutations are applied
//      via the standard web Interpreter
//   2. A 2D canvas used to rasterize the DOM subtree into a texture
//   3. A WebGL texture uploaded from the canvas each time the panel is dirty
//   4. 3D transform data (position, rotation) for placement in the XR scene
//
// The panel manager coordinates multiple panels, tracking which are dirty
// and need texture re-capture, and providing the WebGL textures + transforms
// to the XR renderer for quad drawing.
//
// DOM-to-texture strategy:
//
//   The initial approach uses SVG foreignObject to rasterize DOM content
//   to a canvas. This works for static/simple content and provides real
//   CSS rendering fidelity. The flow is:
//
//     1. Mutations arrive via the binary protocol → Interpreter applies
//        them to a real DOM subtree (inside a hidden container).
//     2. When the panel is marked dirty, we serialize the subtree's
//        outerHTML into an SVG foreignObject, create a Blob, load it
//        as an Image, and drawImage() onto the panel's canvas.
//     3. The canvas pixels are uploaded to a WebGL texture.
//
//   Limitations of SVG foreignObject:
//     - No external resources (images, fonts) unless inlined
//     - No JavaScript execution or interactive states (:hover, :focus)
//     - Some CSS features may not render (e.g. backdrop-filter)
//
//   Future improvements:
//     - Use OffscreenCanvas + html2canvas for higher fidelity
//     - Direct 2D canvas rendering (bypassing DOM entirely)
//     - WebGPU compute-based text rendering
//
// Usage:
//
//   const manager = new XRPanelManager(document);
//   const panel = manager.createPanel(defaultPanelConfig());
//   panel.interpreter.applyMutations(buffer, offset, length);
//   panel.markDirty();
//
//   // In the XR frame loop:
//   await manager.updateDirtyTextures(gl);
//   for (const panel of manager.panels) {
//     // Draw panel.glTexture as a quad at panel.position/rotation
//   }

import type {
	PanelConfig,
	PanelState,
	Quaternion,
	RaycastHit,
	Vec3,
	XRInputRay,
	XRPanelDescriptor,
} from "./xr-types.ts";
import { defaultPanelConfig } from "./xr-types.ts";

// ── XR Panel ────────────────────────────────────────────────────────────────

/**
 * A live XR panel instance.
 *
 * Each panel has its own offscreen DOM container, 2D rasterization canvas,
 * and WebGL texture. The panel accepts DOM mutations via its container
 * element (which the Interpreter targets), tracks dirtiness, and provides
 * its texture + 3D transform to the renderer.
 */
export class XRPanel implements XRPanelDescriptor {
	/** Unique panel ID (assigned by XRPanelManager). */
	readonly id: number;

	/** Panel configuration (immutable after creation). */
	readonly config: Readonly<PanelConfig>;

	/** World-space position (meters). */
	position: Vec3;

	/** World-space rotation (unit quaternion). */
	rotation: Quaternion;

	/** Runtime state. */
	state: PanelState;

	/** Texture width in pixels. */
	readonly textureWidth: number;

	/** Texture height in pixels. */
	readonly textureHeight: number;

	/**
	 * The offscreen DOM container for this panel's content.
	 * The Interpreter targets this element as its root.
	 * Hidden from view via CSS (position: absolute, off-screen).
	 */
	readonly container: HTMLDivElement;

	/**
	 * The 2D canvas used to rasterize DOM content for texture upload.
	 * Sized to textureWidth × textureHeight.
	 */
	readonly rasterCanvas: HTMLCanvasElement;

	/** The 2D rendering context for the raster canvas. */
	readonly rasterCtx: CanvasRenderingContext2D;

	/**
	 * The WebGL texture for this panel, or null if not yet created.
	 * Created lazily on the first call to `uploadTexture()`.
	 */
	glTexture: WebGLTexture | null = null;

	/**
	 * Whether the GL texture has been initialized (texImage2D called
	 * at least once with the correct dimensions).
	 */
	private _textureInitialized = false;

	/**
	 * Timestamp (ms) of the last texture update. Used for throttling
	 * texture re-captures to the configured update rate.
	 */
	lastTextureUpdate = 0;

	constructor(
		id: number,
		config: PanelConfig,
		doc: Document,
		panelBackground: string,
	) {
		this.id = id;
		this.config = Object.freeze({ ...config });

		// Default transform: 1m in front of the user at eye height
		this.position = { x: 0, y: 1.4, z: -1.0 };
		this.rotation = { x: 0, y: 0, z: 0, w: 1 };

		this.state = {
			visible: true,
			focused: false,
			dirty: true, // Dirty on creation — needs initial render
			mounted: false,
		};

		// Derive texture dimensions from physical size and pixel density
		this.textureWidth = Math.round(config.widthM * config.pixelsPerMeter);
		this.textureHeight = Math.round(config.heightM * config.pixelsPerMeter);

		// Create offscreen DOM container
		this.container = doc.createElement("div");
		this.container.setAttribute("data-xr-panel", String(id));
		this.container.style.cssText = [
			"position: absolute",
			"left: -99999px",
			"top: -99999px",
			`width: ${this.textureWidth}px`,
			`height: ${this.textureHeight}px`,
			"overflow: hidden",
			"visibility: hidden",
			"pointer-events: none",
		].join("; ");

		// Inject XR panel stylesheet for background and font defaults
		const styleElement = doc.createElement("style");
		styleElement.textContent = `
			[data-xr-panel="${id}"] {
				background: ${panelBackground};
				font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
				font-size: 16px;
				line-height: 1.5;
				color: #1a1a1a;
			}
		`;
		this.container.appendChild(styleElement);

		// Append to document body so styles are computed
		doc.body.appendChild(this.container);

		// Create rasterization canvas
		this.rasterCanvas = doc.createElement("canvas");
		this.rasterCanvas.width = this.textureWidth;
		this.rasterCanvas.height = this.textureHeight;

		const ctx = this.rasterCanvas.getContext("2d", {
			willReadFrequently: false,
			alpha: true,
		});
		if (!ctx) {
			throw new Error(`XRPanel(${id}): failed to create 2D canvas context`);
		}
		this.rasterCtx = ctx;
	}

	// ── Transform Helpers ─────────────────────────────────────────────

	/** Set the panel's world-space position. */
	setPosition(x: number, y: number, z: number): void {
		this.position = { x, y, z };
	}

	/**
	 * Set the panel's rotation from Euler angles (radians, YXZ order).
	 * Converts to a unit quaternion internally.
	 */
	setRotationEuler(pitch: number, yaw: number, roll: number): void {
		// YXZ Euler → quaternion conversion
		const cy = Math.cos(yaw * 0.5);
		const sy = Math.sin(yaw * 0.5);
		const cx = Math.cos(pitch * 0.5);
		const sx = Math.sin(pitch * 0.5);
		const cz = Math.cos(roll * 0.5);
		const sz = Math.sin(roll * 0.5);

		this.rotation = {
			x: sx * cy * cz + cx * sy * sz,
			y: cx * sy * cz - sx * cy * sz,
			z: cx * cy * sz - sx * sy * cz,
			w: cx * cy * cz + sx * sy * sz,
		};
	}

	/** Set the panel's rotation directly from a quaternion. */
	setRotation(x: number, y: number, z: number, w: number): void {
		this.rotation = { x, y, z, w };
	}

	/** Mark the panel's DOM content as changed (needs texture re-capture). */
	markDirty(): void {
		this.state.dirty = true;
	}

	/** Mark the panel as having completed its initial mount. */
	markMounted(): void {
		this.state.mounted = true;
		this.state.dirty = true;
	}

	// ── DOM → Canvas Rasterization ────────────────────────────────────

	/**
	 * Rasterize the panel's DOM content to the 2D canvas.
	 *
	 * Uses the SVG foreignObject technique:
	 *   1. Serialize the container's innerHTML
	 *   2. Wrap in an SVG foreignObject
	 *   3. Create a Blob → object URL → Image
	 *   4. Draw the image onto the raster canvas
	 *
	 * Returns a Promise that resolves when the canvas is updated.
	 * This is async because Image loading is inherently async.
	 */
	async rasterize(): Promise<void> {
		const width = this.textureWidth;
		const height = this.textureHeight;
		const ctx = this.rasterCtx;

		// Serialize the panel's DOM content
		const htmlContent = this.container.innerHTML;

		// Build SVG with foreignObject wrapping the HTML
		// We include xmlns declarations so the SVG + XHTML are well-formed.
		const svgMarkup = [
			`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">`,
			`<foreignObject width="100%" height="100%">`,
			`<div xmlns="http://www.w3.org/1999/xhtml" style="`,
			`width: ${width}px; height: ${height}px; overflow: hidden;`,
			`background: ${this.config.interact ? "#ffffff" : "transparent"};`,
			`font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;`,
			`font-size: 16px; line-height: 1.5; color: #1a1a1a;`,
			`">`,
			htmlContent,
			`</div>`,
			`</foreignObject>`,
			`</svg>`,
		].join("");

		// Create a Blob and object URL
		const blob = new Blob([svgMarkup], { type: "image/svg+xml;charset=utf-8" });
		const url = URL.createObjectURL(blob);

		try {
			// Load as Image
			const img = await loadImage(url);

			// Draw to canvas
			ctx.clearRect(0, 0, width, height);
			ctx.drawImage(img, 0, 0, width, height);
		} finally {
			URL.revokeObjectURL(url);
		}

		this.state.dirty = false;
	}

	/**
	 * Rasterize using a simpler fallback: render a solid background with
	 * the panel's text content overlaid. This is used when SVG foreignObject
	 * is unavailable or fails (e.g. in some test environments).
	 */
	rasterizeFallback(): void {
		const width = this.textureWidth;
		const height = this.textureHeight;
		const ctx = this.rasterCtx;

		// Clear with background
		ctx.fillStyle = "#ffffff";
		ctx.fillRect(0, 0, width, height);

		// Extract text content from the DOM container
		const text = this.container.textContent || "(empty panel)";

		// Render text
		ctx.fillStyle = "#1a1a1a";
		ctx.font = "16px -apple-system, BlinkMacSystemFont, sans-serif";
		ctx.textBaseline = "top";

		// Simple word-wrap
		const padding = 16;
		const lineHeight = 24;
		const maxWidth = width - padding * 2;
		const words = text.split(/\s+/);
		let line = "";
		let y = padding;

		for (const word of words) {
			const testLine = line ? `${line} ${word}` : word;
			const metrics = ctx.measureText(testLine);
			if (metrics.width > maxWidth && line) {
				ctx.fillText(line, padding, y);
				line = word;
				y += lineHeight;
				if (y > height - padding) break;
			} else {
				line = testLine;
			}
		}
		if (line && y <= height - padding) {
			ctx.fillText(line, padding, y);
		}

		this.state.dirty = false;
	}

	// ── WebGL Texture Upload ──────────────────────────────────────────

	/**
	 * Upload the raster canvas content to a WebGL texture.
	 *
	 * Creates the texture on first call. Subsequent calls update the
	 * existing texture via `texSubImage2D` for efficiency.
	 *
	 * @param gl - The WebGL2 rendering context.
	 */
	uploadTexture(gl: WebGL2RenderingContext): void {
		if (!this.glTexture) {
			const tex = gl.createTexture();
			if (!tex) {
				throw new Error(`XRPanel(${this.id}): failed to create WebGL texture`);
			}
			this.glTexture = tex;
		}

		gl.bindTexture(gl.TEXTURE_2D, this.glTexture);

		if (!this._textureInitialized) {
			// First upload — allocate the texture with texImage2D
			gl.texImage2D(
				gl.TEXTURE_2D,
				0, // mip level
				gl.RGBA, // internal format
				gl.RGBA, // format
				gl.UNSIGNED_BYTE, // type
				this.rasterCanvas,
			);

			// Set texture parameters for XR rendering
			// Linear filtering for readable text at varying distances
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);

			// Clamp to edge — panels don't tile
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
			gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);

			this._textureInitialized = true;
		} else {
			// Subsequent uploads — update existing texture
			gl.texSubImage2D(
				gl.TEXTURE_2D,
				0, // mip level
				0, // x offset
				0, // y offset
				gl.RGBA, // format
				gl.UNSIGNED_BYTE, // type
				this.rasterCanvas,
			);
		}

		gl.bindTexture(gl.TEXTURE_2D, null);
		this.lastTextureUpdate = performance.now();
	}

	// ── Raycasting ────────────────────────────────────────────────────

	/**
	 * Test whether a ray intersects this panel's quad in world space.
	 *
	 * The panel is treated as a flat rectangle in 3D, centered at
	 * `this.position` with dimensions `config.widthM × config.heightM`,
	 * oriented by `this.rotation`.
	 *
	 * @param ray - The input ray to test.
	 * @returns A RaycastHit if the ray intersects, or null.
	 */
	raycast(ray: XRInputRay): RaycastHit | null {
		if (!this.state.visible || !this.config.interact) return null;

		// Panel normal vector: the panel faces -Z in local space,
		// rotated by the panel's quaternion.
		const normal = rotateVecByQuat({ x: 0, y: 0, z: -1 }, this.rotation);

		// Ray-plane intersection:
		//   t = dot(panelPos - rayOrigin, normal) / dot(rayDir, normal)
		const denom = dot(ray.direction, normal);
		if (Math.abs(denom) < 1e-6) return null; // Ray is parallel to the panel

		const diff: Vec3 = {
			x: this.position.x - ray.origin.x,
			y: this.position.y - ray.origin.y,
			z: this.position.z - ray.origin.z,
		};
		const t = dot(diff, normal) / denom;

		if (t < 0) return null; // Intersection is behind the ray origin

		// Hit point in world space
		const hitWorld: Vec3 = {
			x: ray.origin.x + ray.direction.x * t,
			y: ray.origin.y + ray.direction.y * t,
			z: ray.origin.z + ray.direction.z * t,
		};

		// Transform hit point to panel-local space
		// panelLocal = inverseRotate(hitWorld - panelPos)
		const hitRelative: Vec3 = {
			x: hitWorld.x - this.position.x,
			y: hitWorld.y - this.position.y,
			z: hitWorld.z - this.position.z,
		};
		const invRotation = quatInverse(this.rotation);
		const hitLocal = rotateVecByQuat(hitRelative, invRotation);

		// Check if hit is within panel bounds (centered at origin in local space)
		const halfW = this.config.widthM * 0.5;
		const halfH = this.config.heightM * 0.5;

		if (
			hitLocal.x < -halfW ||
			hitLocal.x > halfW ||
			hitLocal.y < -halfH ||
			hitLocal.y > halfH
		) {
			return null; // Outside panel bounds
		}

		// Convert to UV coordinates [0,1] from top-left
		const u = (hitLocal.x + halfW) / this.config.widthM;
		const v = 1.0 - (hitLocal.y + halfH) / this.config.heightM; // Flip Y: DOM Y grows downward

		// Convert to pixel coordinates
		const pixelX = Math.round(u * this.textureWidth);
		const pixelY = Math.round(v * this.textureHeight);

		return {
			panelId: this.id,
			distance: t,
			uv: { u, v },
			pixel: { x: pixelX, y: pixelY },
		};
	}

	// ── Compute Model Matrix ──────────────────────────────────────────

	/**
	 * Compute the 4×4 model matrix for this panel.
	 *
	 * The matrix transforms from panel-local space (where the panel is
	 * a unit quad centered at the origin) to world space, incorporating
	 * position, rotation, and scale (from physical dimensions).
	 *
	 * @returns A Float32Array of 16 elements (column-major).
	 */
	getModelMatrix(): Float32Array {
		const m = new Float32Array(16);

		// Build rotation matrix from quaternion
		const { x: qx, y: qy, z: qz, w: qw } = this.rotation;
		const x2 = qx + qx;
		const y2 = qy + qy;
		const z2 = qz + qz;
		const xx = qx * x2;
		const xy = qx * y2;
		const xz = qx * z2;
		const yy = qy * y2;
		const yz = qy * z2;
		const zz = qz * z2;
		const wx = qw * x2;
		const wy = qw * y2;
		const wz = qw * z2;

		// Scale by panel physical dimensions (half-extents for a centered quad)
		const sx = this.config.widthM;
		const sy = this.config.heightM;

		// Column 0 (X axis, scaled by width)
		m[0] = (1 - (yy + zz)) * sx;
		m[1] = (xy + wz) * sx;
		m[2] = (xz - wy) * sx;
		m[3] = 0;

		// Column 1 (Y axis, scaled by height)
		m[4] = (xy - wz) * sy;
		m[5] = (1 - (xx + zz)) * sy;
		m[6] = (yz + wx) * sy;
		m[7] = 0;

		// Column 2 (Z axis, no scale — panel has no depth)
		m[8] = xz + wy;
		m[9] = yz - wx;
		m[10] = 1 - (xx + yy);
		m[11] = 0;

		// Column 3 (translation)
		m[12] = this.position.x;
		m[13] = this.position.y;
		m[14] = this.position.z;
		m[15] = 1;

		return m;
	}

	// ── Cleanup ───────────────────────────────────────────────────────

	/**
	 * Destroy the panel: remove DOM container, release GL texture.
	 *
	 * @param gl - The WebGL2 context (optional — omit if GL context is already lost).
	 */
	destroy(gl?: WebGL2RenderingContext): void {
		// Remove DOM container from the document
		if (this.container.parentNode) {
			this.container.parentNode.removeChild(this.container);
		}

		// Delete WebGL texture
		if (this.glTexture && gl) {
			gl.deleteTexture(this.glTexture);
		}
		this.glTexture = null;
		this._textureInitialized = false;

		this.state.visible = false;
		this.state.mounted = false;
	}
}

// ── XR Panel Manager ────────────────────────────────────────────────────────

/**
 * Manages the collection of XR panels.
 *
 * Responsibilities:
 *   - Panel lifecycle (create, destroy)
 *   - Focus management (exclusive keyboard/text focus)
 *   - Dirty tracking and throttled texture updates
 *   - Raycasting across all visible panels
 *   - Spatial layout helpers (arc, grid, stack)
 */
export class XRPanelManager {
	/** The Document used for creating DOM elements. */
	private readonly _doc: Document;

	/** All active panels, keyed by panel ID. */
	private readonly _panels: Map<number, XRPanel> = new Map();

	/** Next panel ID to assign. */
	private _nextId = 0;

	/** ID of the currently focused panel, or -1 for none. */
	private _focusedPanelId = -1;

	/** CSS background color for new panels. */
	private readonly _panelBackground: string;

	/** Minimum interval between texture updates (ms). */
	private readonly _textureUpdateInterval: number;

	/**
	 * @param doc               - The Document for DOM operations.
	 * @param panelBackground   - CSS background color for panels (default: "#ffffff").
	 * @param textureUpdateRate - Target texture update rate in Hz (default: 30).
	 */
	constructor(
		doc: Document,
		panelBackground = "#ffffff",
		textureUpdateRate = 30,
	) {
		this._doc = doc;
		this._panelBackground = panelBackground;
		this._textureUpdateInterval = 1000 / textureUpdateRate;
	}

	// ── Panel Lifecycle ───────────────────────────────────────────────

	/**
	 * Create a new XR panel with the given configuration.
	 *
	 * @param config - Panel configuration (size, pixel density, etc.).
	 *                 Defaults to `defaultPanelConfig()` if omitted.
	 * @returns The newly created panel.
	 */
	createPanel(config?: Partial<PanelConfig>): XRPanel {
		const fullConfig: PanelConfig = {
			...defaultPanelConfig(),
			...config,
		};

		const id = this._nextId++;
		const panel = new XRPanel(id, fullConfig, this._doc, this._panelBackground);

		this._panels.set(id, panel);

		// First panel gets focus automatically
		if (this._panels.size === 1) {
			this.focusPanel(id);
		}

		return panel;
	}

	/**
	 * Get a panel by ID.
	 *
	 * @param id - Panel ID.
	 * @returns The panel, or undefined if not found.
	 */
	getPanel(id: number): XRPanel | undefined {
		return this._panels.get(id);
	}

	/**
	 * Destroy a panel by ID.
	 *
	 * @param id - Panel ID to destroy.
	 * @param gl - WebGL2 context for texture cleanup (optional).
	 */
	destroyPanel(id: number, gl?: WebGL2RenderingContext): void {
		const panel = this._panels.get(id);
		if (!panel) return;

		panel.destroy(gl);
		this._panels.delete(id);

		// Transfer focus if the destroyed panel was focused
		if (this._focusedPanelId === id) {
			this._focusedPanelId = -1;
			// Focus the first remaining panel, if any
			const firstId = this._panels.keys().next().value;
			if (firstId !== undefined) {
				this.focusPanel(firstId);
			}
		}
	}

	/** Iterate over all active panels. */
	get panels(): IterableIterator<XRPanel> {
		return this._panels.values();
	}

	/** Number of active panels. */
	get panelCount(): number {
		return this._panels.size;
	}

	// ── Focus Management ──────────────────────────────────────────────

	/**
	 * Set input focus to a specific panel.
	 *
	 * @param id - Panel ID to focus. Pass -1 to clear focus.
	 */
	focusPanel(id: number): void {
		// Unfocus current
		if (this._focusedPanelId >= 0) {
			const prev = this._panels.get(this._focusedPanelId);
			if (prev) prev.state.focused = false;
		}

		this._focusedPanelId = id;

		// Focus new
		if (id >= 0) {
			const panel = this._panels.get(id);
			if (panel) panel.state.focused = true;
		}
	}

	/** The ID of the currently focused panel, or -1. */
	get focusedPanelId(): number {
		return this._focusedPanelId;
	}

	/** The currently focused panel, or null. */
	get focusedPanel(): XRPanel | null {
		if (this._focusedPanelId < 0) return null;
		return this._panels.get(this._focusedPanelId) ?? null;
	}

	// ── Dirty Tracking & Texture Update ───────────────────────────────

	/**
	 * Get all panels that need texture re-capture.
	 *
	 * Applies throttling: panels are only considered dirty if enough
	 * time has passed since their last texture update.
	 */
	getDirtyPanels(): XRPanel[] {
		const now = performance.now();
		const dirty: XRPanel[] = [];

		for (const panel of this._panels.values()) {
			if (
				panel.state.dirty &&
				panel.state.visible &&
				now - panel.lastTextureUpdate >= this._textureUpdateInterval
			) {
				dirty.push(panel);
			}
		}

		return dirty;
	}

	/**
	 * Update textures for all dirty panels.
	 *
	 * For each dirty panel (subject to throttling):
	 *   1. Rasterize DOM → canvas (async via SVG foreignObject)
	 *   2. Upload canvas → WebGL texture
	 *
	 * @param gl - The WebGL2 rendering context.
	 * @param useFallback - If true, use the simple text-only rasterizer
	 *                      instead of SVG foreignObject (for environments
	 *                      where SVG rasterization doesn't work).
	 */
	async updateDirtyTextures(
		gl: WebGL2RenderingContext,
		useFallback = false,
	): Promise<void> {
		const dirty = this.getDirtyPanels();
		if (dirty.length === 0) return;

		// Rasterize all dirty panels
		if (useFallback) {
			for (const panel of dirty) {
				panel.rasterizeFallback();
				panel.uploadTexture(gl);
			}
		} else {
			// SVG foreignObject rasterization is async — run in parallel
			const rasterPromises = dirty.map(async (panel) => {
				try {
					await panel.rasterize();
				} catch {
					// SVG foreignObject may fail in some environments.
					// Fall back to the simple text rasterizer.
					panel.rasterizeFallback();
				}
				panel.uploadTexture(gl);
			});

			await Promise.all(rasterPromises);
		}
	}

	// ── Raycasting ────────────────────────────────────────────────────

	/**
	 * Raycast against all visible, interactive panels.
	 *
	 * Returns the closest hit, or null if no panel was hit.
	 *
	 * @param ray - The input ray to test.
	 */
	raycast(ray: XRInputRay): RaycastHit | null {
		let closestHit: RaycastHit | null = null;

		for (const panel of this._panels.values()) {
			const hit = panel.raycast(ray);
			if (hit && (closestHit === null || hit.distance < closestHit.distance)) {
				closestHit = hit;
			}
		}

		return closestHit;
	}

	/**
	 * Raycast and return all hits, sorted by distance (nearest first).
	 *
	 * @param ray - The input ray to test.
	 */
	raycastAll(ray: XRInputRay): RaycastHit[] {
		const hits: RaycastHit[] = [];

		for (const panel of this._panels.values()) {
			const hit = panel.raycast(ray);
			if (hit) hits.push(hit);
		}

		hits.sort((a, b) => a.distance - b.distance);
		return hits;
	}

	// ── Spatial Layout Helpers ─────────────────────────────────────────
	// Mirror the layout helpers from xr/native/src/xr/scene.mojo

	/**
	 * Arrange panels in an arc centered at a point.
	 *
	 * Panels are evenly distributed along an arc of `totalAngle` radians
	 * at `radius` meters from the center position, at `height` meters.
	 *
	 * @param panels     - Array of panel IDs to arrange.
	 * @param center     - Center position of the arc (XZ plane).
	 * @param radius     - Distance from center to panels (meters).
	 * @param height     - Y position of the panels (meters).
	 * @param totalAngle - Total arc angle in radians (default: Math.PI/2 = 90°).
	 */
	arrangeArc(
		panels: number[],
		center: Vec3,
		radius: number,
		height: number,
		totalAngle = Math.PI / 2,
	): void {
		const count = panels.length;
		if (count === 0) return;

		const startAngle = -totalAngle / 2;
		const step = count > 1 ? totalAngle / (count - 1) : 0;

		for (let i = 0; i < count; i++) {
			const panel = this._panels.get(panels[i]);
			if (!panel) continue;

			const angle = startAngle + step * i;
			panel.setPosition(
				center.x + Math.sin(angle) * radius,
				height,
				center.z - Math.cos(angle) * radius,
			);
			// Face the panel toward the center
			panel.setRotationEuler(0, angle, 0);
		}
	}

	/**
	 * Arrange panels in a grid.
	 *
	 * @param panels  - Array of panel IDs to arrange.
	 * @param origin  - Top-left position of the grid.
	 * @param columns - Number of columns in the grid.
	 * @param spacingX - Horizontal spacing between panel centers (meters).
	 * @param spacingY - Vertical spacing between panel centers (meters).
	 */
	arrangeGrid(
		panels: number[],
		origin: Vec3,
		columns: number,
		spacingX = 0.9,
		spacingY = 0.7,
	): void {
		for (let i = 0; i < panels.length; i++) {
			const panel = this._panels.get(panels[i]);
			if (!panel) continue;

			const col = i % columns;
			const row = Math.floor(i / columns);

			panel.setPosition(
				origin.x + col * spacingX,
				origin.y - row * spacingY,
				origin.z,
			);
			// All panels face the same direction (forward)
			panel.setRotation(0, 0, 0, 1);
		}
	}

	/**
	 * Arrange panels in a vertical stack.
	 *
	 * @param panels  - Array of panel IDs to arrange (top to bottom).
	 * @param top     - Position of the topmost panel center.
	 * @param spacing - Vertical spacing between panel centers (meters).
	 */
	arrangeStack(panels: number[], top: Vec3, spacing = 0.7): void {
		for (let i = 0; i < panels.length; i++) {
			const panel = this._panels.get(panels[i]);
			if (!panel) continue;

			panel.setPosition(top.x, top.y - i * spacing, top.z);
			panel.setRotation(0, 0, 0, 1);
		}
	}

	// ── Cleanup ───────────────────────────────────────────────────────

	/**
	 * Destroy all panels and release all resources.
	 *
	 * @param gl - WebGL2 context for texture cleanup (optional).
	 */
	destroyAll(gl?: WebGL2RenderingContext): void {
		for (const panel of this._panels.values()) {
			panel.destroy(gl);
		}
		this._panels.clear();
		this._focusedPanelId = -1;
		this._nextId = 0;
	}
}

// ── Math Utilities (module-private) ─────────────────────────────────────────

/** Dot product of two Vec3. */
function dot(a: Vec3, b: Vec3): number {
	return a.x * b.x + a.y * b.y + a.z * b.z;
}

/** Rotate a vector by a quaternion: q * v * q⁻¹. */
function rotateVecByQuat(v: Vec3, q: Quaternion): Vec3 {
	// Optimized quaternion-vector rotation (avoids full quat multiply)
	const tx = 2 * (q.y * v.z - q.z * v.y);
	const ty = 2 * (q.z * v.x - q.x * v.z);
	const tz = 2 * (q.x * v.y - q.y * v.x);

	return {
		x: v.x + q.w * tx + (q.y * tz - q.z * ty),
		y: v.y + q.w * ty + (q.z * tx - q.x * tz),
		z: v.z + q.w * tz + (q.x * ty - q.y * tx),
	};
}

/** Quaternion conjugate (inverse for unit quaternions). */
function quatInverse(q: Quaternion): Quaternion {
	return { x: -q.x, y: -q.y, z: -q.z, w: q.w };
}

/**
 * Load an image from a URL. Returns a Promise that resolves when the
 * image is fully loaded.
 */
function loadImage(url: string): Promise<HTMLImageElement> {
	return new Promise((resolve, reject) => {
		const img = new Image();
		img.onload = () => resolve(img);
		img.onerror = (err) => reject(new Error(`Failed to load image: ${err}`));
		img.src = url;
	});
}
