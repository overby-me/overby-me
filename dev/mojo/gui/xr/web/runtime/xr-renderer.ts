// XR Quad Renderer — Draws panel textures as textured quads in the XR scene.
//
// This module provides a minimal WebGL2 renderer that draws XR panels as
// textured quads in 3D space. Each panel has a model matrix (from its
// position, rotation, and physical dimensions), and the renderer composites
// all panels into the XRWebGLLayer framebuffer using the view/projection
// matrices from the XR viewer pose.
//
// The renderer is intentionally simple:
//   - One shader program (textured quad with alpha blending)
//   - One shared quad geometry (unit quad, scaled by model matrix)
//   - Per-panel: bind texture, set model matrix uniform, draw
//
// This is NOT a general-purpose 3D engine. It only draws flat textured
// quads for XR panel rendering. Future improvements could add:
//   - Curved panel geometry (cylindrical mesh for curved panels)
//   - Panel border/shadow effects
//   - Cursor/pointer visualization
//   - Anti-aliasing (MSAA via renderbuffer)
//
// Usage:
//
//   const renderer = new XRQuadRenderer(gl);
//   renderer.initialize();
//
//   // In the XR frame loop:
//   renderer.beginFrame(gl, glLayer, viewerPose);
//   for (const view of viewerPose.views) {
//     renderer.setView(view, glLayer);
//     for (const panel of panels) {
//       renderer.drawPanel(panel);
//     }
//   }
//   renderer.endFrame();

import type { XRPanel } from "./xr-panel.ts";
import type {
	XRViewCompat,
	XRViewerPoseCompat,
	XRWebGLLayerCompat,
} from "./xr-types.ts";

// ── Shader Sources ──────────────────────────────────────────────────────────

const VERTEX_SHADER_SOURCE = `#version 300 es
precision highp float;

// Quad vertex positions: a unit quad from (-0.5, -0.5) to (0.5, 0.5)
// centered at the origin. The model matrix scales this to the panel's
// physical dimensions and places it in world space.
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_texCoord;

uniform mat4 u_model;
uniform mat4 u_view;
uniform mat4 u_projection;

out vec2 v_texCoord;

void main() {
    v_texCoord = a_texCoord;
    gl_Position = u_projection * u_view * u_model * vec4(a_position, 1.0);
}
`;

const FRAGMENT_SHADER_SOURCE = `#version 300 es
precision highp float;

in vec2 v_texCoord;

uniform sampler2D u_texture;
uniform float u_opacity;

out vec4 fragColor;

void main() {
    vec4 texel = texture(u_texture, v_texCoord);
    fragColor = vec4(texel.rgb, texel.a * u_opacity);
}
`;

// ── Quad Geometry ───────────────────────────────────────────────────────────
// Unit quad centered at origin, facing -Z (consistent with panel normal).
// Vertices: position (x, y, z) + texcoord (u, v)
// The quad spans [-0.5, 0.5] in X and Y; the model matrix scales it to
// the panel's widthM × heightM.

// prettier-ignore
const QUAD_VERTICES = new Float32Array([
	// pos x,  pos y,  pos z,  tex u,  tex v
	-0.5,
	0.5,
	0.0,
	0.0,
	0.0, // top-left
	-0.5,
	-0.5,
	0.0,
	0.0,
	1.0, // bottom-left
	0.5,
	0.5,
	0.0,
	1.0,
	0.0, // top-right
	0.5,
	-0.5,
	0.0,
	1.0,
	1.0, // bottom-right
]);

// Two triangles forming the quad (counter-clockwise winding)
const QUAD_INDICES = new Uint16Array([0, 1, 2, 2, 1, 3]);

// Vertex stride: 5 floats × 4 bytes = 20 bytes
const VERTEX_STRIDE = 5 * 4;

// Attribute offsets
const POSITION_OFFSET = 0;
const TEXCOORD_OFFSET = 3 * 4; // 12 bytes

// ── XRQuadRenderer ──────────────────────────────────────────────────────────

/**
 * WebGL2 renderer for XR panel quads.
 *
 * Draws textured quads in 3D XR space. Each panel's DOM content has been
 * rasterized to a WebGL texture by the XRPanelManager; this renderer
 * places those textures in the scene at the correct position and rotation.
 *
 * Lifecycle:
 *   1. `new XRQuadRenderer(gl)` — construct
 *   2. `initialize()` — compile shaders, create buffers
 *   3. Per frame: `beginFrame()` → per view: `setView()` → per panel: `drawPanel()` → `endFrame()`
 *   4. `destroy()` — release GL resources
 */
export class XRQuadRenderer {
	/** The WebGL2 rendering context. */
	private _gl: WebGL2RenderingContext;

	/** Compiled and linked shader program. */
	private _program: WebGLProgram | null = null;

	/** Vertex Array Object for the quad geometry. */
	private _vao: WebGLVertexArrayObject | null = null;

	/** Vertex buffer for quad positions + texcoords. */
	private _vbo: WebGLBuffer | null = null;

	/** Index buffer for quad triangles. */
	private _ebo: WebGLBuffer | null = null;

	// ── Uniform locations ─────────────────────────────────────────────

	private _uModel: WebGLUniformLocation | null = null;
	private _uView: WebGLUniformLocation | null = null;
	private _uProjection: WebGLUniformLocation | null = null;
	private _uTexture: WebGLUniformLocation | null = null;
	private _uOpacity: WebGLUniformLocation | null = null;

	/** Whether `initialize()` has been called successfully. */
	private _initialized = false;

	/** Saved GL state for restoration in endFrame(). */
	private _savedState: {
		depthTest: boolean;
		blend: boolean;
		cullFace: boolean;
	} | null = null;

	constructor(gl: WebGL2RenderingContext) {
		this._gl = gl;
	}

	/** Whether the renderer has been initialized. */
	get initialized(): boolean {
		return this._initialized;
	}

	// ── Initialization ────────────────────────────────────────────────

	/**
	 * Compile shaders, link program, create quad geometry buffers.
	 *
	 * Must be called once before any rendering. Safe to call multiple
	 * times (subsequent calls are no-ops).
	 *
	 * @throws If shader compilation or program linking fails.
	 */
	initialize(): void {
		if (this._initialized) return;

		const gl = this._gl;

		// 1. Compile shaders
		const vs = this.compileShader(gl.VERTEX_SHADER, VERTEX_SHADER_SOURCE);
		const fs = this.compileShader(gl.FRAGMENT_SHADER, FRAGMENT_SHADER_SOURCE);

		// 2. Link program
		const program = gl.createProgram();
		if (!program) {
			gl.deleteShader(vs);
			gl.deleteShader(fs);
			throw new Error("XRQuadRenderer: failed to create shader program");
		}

		gl.attachShader(program, vs);
		gl.attachShader(program, fs);
		gl.linkProgram(program);

		// Shaders can be deleted after linking — the program retains them.
		gl.deleteShader(vs);
		gl.deleteShader(fs);

		if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
			const info = gl.getProgramInfoLog(program);
			gl.deleteProgram(program);
			throw new Error(`XRQuadRenderer: program link failed: ${info}`);
		}

		this._program = program;

		// 3. Get uniform locations
		this._uModel = gl.getUniformLocation(program, "u_model");
		this._uView = gl.getUniformLocation(program, "u_view");
		this._uProjection = gl.getUniformLocation(program, "u_projection");
		this._uTexture = gl.getUniformLocation(program, "u_texture");
		this._uOpacity = gl.getUniformLocation(program, "u_opacity");

		// 4. Create quad geometry
		this._vao = gl.createVertexArray();
		if (!this._vao) {
			throw new Error("XRQuadRenderer: failed to create VAO");
		}

		gl.bindVertexArray(this._vao);

		// Vertex buffer
		this._vbo = gl.createBuffer();
		if (!this._vbo) {
			throw new Error("XRQuadRenderer: failed to create VBO");
		}
		gl.bindBuffer(gl.ARRAY_BUFFER, this._vbo);
		gl.bufferData(gl.ARRAY_BUFFER, QUAD_VERTICES, gl.STATIC_DRAW);

		// Position attribute (location 0): vec3
		gl.enableVertexAttribArray(0);
		gl.vertexAttribPointer(
			0,
			3,
			gl.FLOAT,
			false,
			VERTEX_STRIDE,
			POSITION_OFFSET,
		);

		// TexCoord attribute (location 1): vec2
		gl.enableVertexAttribArray(1);
		gl.vertexAttribPointer(
			1,
			2,
			gl.FLOAT,
			false,
			VERTEX_STRIDE,
			TEXCOORD_OFFSET,
		);

		// Index buffer
		this._ebo = gl.createBuffer();
		if (!this._ebo) {
			throw new Error("XRQuadRenderer: failed to create EBO");
		}
		gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this._ebo);
		gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, QUAD_INDICES, gl.STATIC_DRAW);

		// Unbind
		gl.bindVertexArray(null);
		gl.bindBuffer(gl.ARRAY_BUFFER, null);
		// Note: do NOT unbind ELEMENT_ARRAY_BUFFER while VAO is unbound —
		// that would detach it from the VAO. The VAO stores the EBO binding.

		this._initialized = true;
	}

	// ── Frame Lifecycle ───────────────────────────────────────────────

	/**
	 * Begin a new render frame.
	 *
	 * Binds the XRWebGLLayer framebuffer and clears it. Saves relevant
	 * GL state that will be modified during rendering so it can be
	 * restored in `endFrame()`.
	 *
	 * @param glLayer    - The XRWebGLLayer to render into.
	 * @param clearColor - RGBA clear color (default: transparent black).
	 */
	beginFrame(
		glLayer: XRWebGLLayerCompat,
		clearColor: [number, number, number, number] = [0, 0, 0, 0],
	): void {
		if (!this._initialized) {
			throw new Error(
				"XRQuadRenderer: not initialized — call initialize() first",
			);
		}

		const gl = this._gl;

		// Save GL state
		this._savedState = {
			depthTest: gl.isEnabled(gl.DEPTH_TEST),
			blend: gl.isEnabled(gl.BLEND),
			cullFace: gl.isEnabled(gl.CULL_FACE),
		};

		// Bind the XR framebuffer
		gl.bindFramebuffer(gl.FRAMEBUFFER, glLayer.framebuffer);

		// Set up the full framebuffer viewport for clearing
		gl.viewport(0, 0, glLayer.framebufferWidth, glLayer.framebufferHeight);

		// Clear
		gl.clearColor(clearColor[0], clearColor[1], clearColor[2], clearColor[3]);
		gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);

		// Set rendering state for panel quads
		gl.enable(gl.DEPTH_TEST);
		gl.depthFunc(gl.LESS);

		// Enable alpha blending for transparent panels
		gl.enable(gl.BLEND);
		gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

		// Disable backface culling — panels should be visible from both sides
		// (user may look at a panel from behind)
		gl.disable(gl.CULL_FACE);

		// Use our shader program
		gl.useProgram(this._program);

		// Bind texture unit 0
		gl.activeTexture(gl.TEXTURE0);
		gl.uniform1i(this._uTexture, 0);
	}

	/**
	 * Set up the view and projection matrices for a specific XR view.
	 *
	 * In stereo rendering, this is called twice per frame — once for
	 * each eye. The viewport is set from the XRWebGLLayer's getViewport().
	 *
	 * @param view    - The XRView for this eye (contains projection + transform).
	 * @param glLayer - The XRWebGLLayer (for viewport lookup).
	 */
	setView(view: XRViewCompat, glLayer: XRWebGLLayerCompat): void {
		const gl = this._gl;

		// Set the viewport for this view (each eye gets half the framebuffer)
		const vp = glLayer.getViewport(view);
		if (vp) {
			gl.viewport(vp.x, vp.y, vp.width, vp.height);
		}

		// Set projection matrix from the XR view
		gl.uniformMatrix4fv(this._uProjection, false, view.projectionMatrix);

		// Set view matrix from the XR view's inverse transform
		// XRView.transform is the pose of the eye in reference space;
		// the view matrix is the inverse of that.
		gl.uniformMatrix4fv(this._uView, false, view.transform.inverse.matrix);
	}

	/**
	 * Draw a single panel quad.
	 *
	 * The panel must have a valid `glTexture` (call `panel.uploadTexture()`
	 * first). The panel's model matrix is computed from its position,
	 * rotation, and physical dimensions.
	 *
	 * @param panel   - The XR panel to draw.
	 * @param opacity - Panel opacity, 0..1 (default: 1.0).
	 */
	drawPanel(panel: XRPanel, opacity = 1.0): void {
		if (!panel.glTexture) return;
		if (!panel.state.visible) return;

		const gl = this._gl;

		// Set model matrix
		const modelMatrix = panel.getModelMatrix();
		gl.uniformMatrix4fv(this._uModel, false, modelMatrix);

		// Set opacity
		gl.uniform1f(this._uOpacity, opacity);

		// Bind panel texture
		gl.bindTexture(gl.TEXTURE_2D, panel.glTexture);

		// Draw the quad
		gl.bindVertexArray(this._vao);
		gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_SHORT, 0);
		gl.bindVertexArray(null);
	}

	/**
	 * Convenience: draw all visible panels from an iterable.
	 *
	 * @param panels  - Iterable of XR panels.
	 * @param opacity - Opacity for all panels (default: 1.0).
	 */
	drawPanels(panels: Iterable<XRPanel>, opacity = 1.0): void {
		for (const panel of panels) {
			this.drawPanel(panel, opacity);
		}
	}

	/**
	 * Render a complete frame for a viewer pose.
	 *
	 * This is a convenience method that handles the per-view loop:
	 * for each view in the viewer pose, sets the view matrices and
	 * draws all panels.
	 *
	 * @param viewerPose - The XR viewer pose (null if tracking lost — skips rendering).
	 * @param glLayer    - The XRWebGLLayer.
	 * @param panels     - Iterable of panels to draw.
	 * @param opacity    - Global opacity for all panels (default: 1.0).
	 */
	renderAllViews(
		viewerPose: XRViewerPoseCompat | null,
		glLayer: XRWebGLLayerCompat,
		panels: Iterable<XRPanel>,
		opacity = 1.0,
	): void {
		if (!viewerPose) return;

		for (const view of viewerPose.views) {
			this.setView(view, glLayer);
			this.drawPanels(panels, opacity);
		}
	}

	/**
	 * End the current render frame.
	 *
	 * Unbinds the program and VAO, restores saved GL state.
	 */
	endFrame(): void {
		const gl = this._gl;

		// Unbind resources
		gl.useProgram(null);
		gl.bindTexture(gl.TEXTURE_2D, null);
		gl.bindFramebuffer(gl.FRAMEBUFFER, null);

		// Restore GL state
		if (this._savedState) {
			if (this._savedState.depthTest) {
				gl.enable(gl.DEPTH_TEST);
			} else {
				gl.disable(gl.DEPTH_TEST);
			}

			if (this._savedState.blend) {
				gl.enable(gl.BLEND);
			} else {
				gl.disable(gl.BLEND);
			}

			if (this._savedState.cullFace) {
				gl.enable(gl.CULL_FACE);
			} else {
				gl.disable(gl.CULL_FACE);
			}

			this._savedState = null;
		}
	}

	// ── Cursor / Pointer Visualization ────────────────────────────────

	/**
	 * Draw a small dot on a panel at the given UV coordinates.
	 *
	 * This provides visual feedback for where the XR controller ray
	 * intersects a panel. It renders a small circle at the hit point
	 * by drawing a tiny quad slightly in front of the panel surface.
	 *
	 * Note: This is a simple implementation. A more polished version
	 * would use a cursor texture or a screen-space circle shader.
	 *
	 * @param panel       - The panel to draw the cursor on.
	 * @param u           - Horizontal UV coordinate [0, 1].
	 * @param v           - Vertical UV coordinate [0, 1].
	 * @param cursorSize  - Cursor diameter in meters (default: 0.005).
	 */
	drawCursor(panel: XRPanel, u: number, v: number, cursorSize = 0.005): void {
		// The cursor is positioned by offsetting from the panel's model space.
		// In panel-local space, (u, v) maps to:
		//   x = (u - 0.5) * widthM
		//   y = (0.5 - v) * heightM   (v is top-down, local Y is up)
		//   z = -0.001                 (slightly in front of the panel)
		const localX = (u - 0.5) * panel.config.widthM;
		const localY = (0.5 - v) * panel.config.heightM;

		// Build a model matrix for the cursor quad:
		// Start with the panel's model matrix, then translate in local space
		// and shrink to cursor size.
		// For simplicity, we construct a fresh model matrix.
		const { x: qx, y: qy, z: qz, w: qw } = panel.rotation;
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

		// Panel's local axes (rotation matrix columns, not scaled)
		const rx = 1 - (yy + zz),
			ry = xy + wz,
			rz = xz - wy;
		const ux = xy - wz,
			uy = 1 - (xx + zz),
			uz = yz + wx;
		const fx = xz + wy,
			fy = yz - wx,
			fz = 1 - (xx + yy);

		// World-space cursor position: panel.position + localX * right + localY * up + offset * forward
		const forwardOffset = -0.001; // Slightly in front of panel
		const cx =
			panel.position.x + rx * localX + ux * localY + fx * forwardOffset;
		const cy =
			panel.position.y + ry * localX + uy * localY + fy * forwardOffset;
		const cz =
			panel.position.z + rz * localX + uz * localY + fz * forwardOffset;

		// Build cursor model matrix (same rotation as panel, scaled to cursorSize)
		const m = new Float32Array(16);
		m[0] = (1 - (yy + zz)) * cursorSize;
		m[1] = (xy + wz) * cursorSize;
		m[2] = (xz - wy) * cursorSize;
		m[3] = 0;
		m[4] = (xy - wz) * cursorSize;
		m[5] = (1 - (xx + zz)) * cursorSize;
		m[6] = (yz + wx) * cursorSize;
		m[7] = 0;
		m[8] = xz + wy;
		m[9] = yz - wx;
		m[10] = 1 - (xx + yy);
		m[11] = 0;
		m[12] = cx;
		m[13] = cy;
		m[14] = cz;
		m[15] = 1;

		const gl = this._gl;
		gl.uniformMatrix4fv(this._uModel, false, m);
		gl.uniform1f(this._uOpacity, 0.8);

		// Use a white 1x1 texture for the cursor (or reuse panel texture for now)
		// For a proper implementation, you'd bind a cursor texture here.
		// For now, we just draw the quad — it will show a small piece of the
		// last-bound texture, which acts as a simple dot.
		gl.bindVertexArray(this._vao);
		gl.drawElements(gl.TRIANGLES, 6, gl.UNSIGNED_SHORT, 0);
		gl.bindVertexArray(null);
	}

	// ── Resource Cleanup ──────────────────────────────────────────────

	/**
	 * Release all GL resources.
	 *
	 * After calling destroy(), the renderer cannot be used again.
	 * Safe to call multiple times (subsequent calls are no-ops).
	 */
	destroy(): void {
		if (!this._initialized) return;

		const gl = this._gl;

		if (this._vao) {
			gl.deleteVertexArray(this._vao);
			this._vao = null;
		}
		if (this._vbo) {
			gl.deleteBuffer(this._vbo);
			this._vbo = null;
		}
		if (this._ebo) {
			gl.deleteBuffer(this._ebo);
			this._ebo = null;
		}
		if (this._program) {
			gl.deleteProgram(this._program);
			this._program = null;
		}

		this._uModel = null;
		this._uView = null;
		this._uProjection = null;
		this._uTexture = null;
		this._uOpacity = null;

		this._initialized = false;
	}

	// ── Internal: Shader Compilation ──────────────────────────────────

	/**
	 * Compile a shader from source.
	 *
	 * @param type   - `gl.VERTEX_SHADER` or `gl.FRAGMENT_SHADER`.
	 * @param source - GLSL source code.
	 * @returns The compiled shader.
	 * @throws If compilation fails.
	 */
	private compileShader(type: number, source: string): WebGLShader {
		const gl = this._gl;

		const shader = gl.createShader(type);
		if (!shader) {
			throw new Error(
				`XRQuadRenderer: failed to create ${type === gl.VERTEX_SHADER ? "vertex" : "fragment"} shader`,
			);
		}

		gl.shaderSource(shader, source);
		gl.compileShader(shader);

		if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
			const info = gl.getShaderInfoLog(shader);
			gl.deleteShader(shader);
			const typeName = type === gl.VERTEX_SHADER ? "vertex" : "fragment";
			throw new Error(
				`XRQuadRenderer: ${typeName} shader compilation failed:\n${info}`,
			);
		}

		return shader;
	}
}
