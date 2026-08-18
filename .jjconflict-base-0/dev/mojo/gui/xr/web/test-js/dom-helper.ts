// DOM helper for XR web runtime tests.
//
// Provides a headless DOM environment via `linkedom` for testing
// XR panel management, raycasting, mutation interpretation, and
// input handling without a real browser.
//
// Usage:
//
//   import { createDOM, createDOMWithCanvas } from "./dom-helper.ts";
//
//   const { document, window, root } = createDOM();
//   // Use `document` as you would in a browser.

import { parseHTML } from "npm:linkedom";

/** Minimal DOM environment returned by `createDOM()`. */
export interface DOMEnvironment {
	/** The headless Document instance. */
	document: Document;

	/** The headless Window instance. */
	window: Window;

	/** A `<div id="root">` element inside `<body>`. */
	root: HTMLDivElement;

	/** The `<body>` element. */
	body: HTMLElement;
}

/**
 * Create a minimal headless DOM environment for testing.
 *
 * Returns a `document`, `window`, a `<div id="root">`, and `<body>`.
 * The document has a standard HTML5 structure with an empty root div.
 */
export function createDOM(): DOMEnvironment {
	const { document, window } = parseHTML(
		'<!DOCTYPE html><html><head></head><body><div id="root"></div></body></html>',
	);
	const root = document.getElementById("root") as unknown as HTMLDivElement;
	const body = document.body as unknown as HTMLElement;
	return {
		document: document as unknown as Document,
		window: window as unknown as Window,
		root,
		body,
	};
}

/**
 * Stub for `HTMLCanvasElement.getContext("2d")`.
 *
 * linkedom does not implement `<canvas>` or `CanvasRenderingContext2D`.
 * This provides a minimal stub that satisfies XRPanel's constructor
 * requirements without real pixel operations.
 */
export interface StubCanvasContext2D {
	fillStyle: string;
	font: string;
	textBaseline: string;
	fillRect(x: number, y: number, w: number, h: number): void;
	clearRect(x: number, y: number, w: number, h: number): void;
	fillText(text: string, x: number, y: number): void;
	measureText(text: string): { width: number };
	drawImage(img: unknown, dx: number, dy: number, dw: number, dh: number): void;
}

/**
 * Create a stub `CanvasRenderingContext2D` for testing.
 *
 * Records calls for optional inspection but performs no real drawing.
 */
export function createStubContext2D(): StubCanvasContext2D {
	return {
		fillStyle: "#000000",
		font: "16px sans-serif",
		textBaseline: "top",
		fillRect(_x: number, _y: number, _w: number, _h: number): void {
			// no-op
		},
		clearRect(_x: number, _y: number, _w: number, _h: number): void {
			// no-op
		},
		fillText(_text: string, _x: number, _y: number): void {
			// no-op
		},
		measureText(text: string): { width: number } {
			// Approximate: 8px per character
			return { width: text.length * 8 };
		},
		drawImage(
			_img: unknown,
			_dx: number,
			_dy: number,
			_dw: number,
			_dh: number,
		): void {
			// no-op
		},
	};
}

/**
 * Patch a document so that `createElement("canvas")` returns elements
 * with a working `getContext("2d")` stub.
 *
 * This is necessary because linkedom's `<canvas>` elements do not
 * implement `getContext()`. XRPanel's constructor calls
 * `doc.createElement("canvas").getContext("2d", ...)`, which would
 * return null without this patch.
 *
 * Call this once before creating any XRPanel instances.
 *
 * @param doc - The headless Document to patch.
 */
export function patchCanvasSupport(doc: Document): void {
	const origCreateElement = doc.createElement.bind(doc);

	// biome-ignore lint/suspicious/noExplicitAny: patching linkedom's createElement requires any cast
	(doc as any).createElement = (
		tagName: string,
		options?: ElementCreationOptions,
	): HTMLElement => {
		const el = origCreateElement(tagName, options);

		if (tagName.toLowerCase() === "canvas") {
			// Add width/height properties that XRPanel sets
			let _width = 300;
			let _height = 150;

			Object.defineProperty(el, "width", {
				get: () => _width,
				set: (v: number) => {
					_width = v;
				},
				configurable: true,
			});

			Object.defineProperty(el, "height", {
				get: () => _height,
				set: (v: number) => {
					_height = v;
				},
				configurable: true,
			});

			// Stub getContext
			// biome-ignore lint/suspicious/noExplicitAny: patching linkedom canvas element requires any cast
			(el as any).getContext = (
				_type: string,
				_attrs?: Record<string, unknown>,
			) => {
				return createStubContext2D();
			};
		}

		return el as HTMLElement;
	};
}

/**
 * Create a DOM environment with canvas support patched in.
 *
 * Convenience wrapper combining `createDOM()` + `patchCanvasSupport()`.
 * Use this for any test that creates XRPanel instances.
 */
export function createDOMWithCanvas(): DOMEnvironment {
	const env = createDOM();
	patchCanvasSupport(env.document);
	return env;
}

/**
 * Stub WebGL2RenderingContext for testing texture upload.
 *
 * Provides no-op implementations of the WebGL2 calls used by
 * `XRPanel.uploadTexture()` and `XRQuadRenderer`.
 */
export interface StubWebGL2 {
	TEXTURE_2D: number;
	RGBA: number;
	UNSIGNED_BYTE: number;
	TEXTURE_MIN_FILTER: number;
	TEXTURE_MAG_FILTER: number;
	TEXTURE_WRAP_S: number;
	TEXTURE_WRAP_T: number;
	LINEAR: number;
	CLAMP_TO_EDGE: number;
	DEPTH_TEST: number;
	BLEND: number;
	CULL_FACE: number;
	SRC_ALPHA: number;
	ONE_MINUS_SRC_ALPHA: number;

	/** Track how many textures were created. */
	_textureCount: number;
	/** Track how many texImage2D calls were made. */
	_texImage2DCount: number;
	/** Track how many texSubImage2D calls were made. */
	_texSubImage2DCount: number;
	/** Track how many deleteTexture calls were made. */
	_deleteTextureCount: number;

	createTexture(): { _id: number } | null;
	bindTexture(target: number, texture: unknown): void;
	texImage2D(
		target: number,
		level: number,
		internalFormat: number,
		format: number,
		type: number,
		source: unknown,
	): void;
	texSubImage2D(
		target: number,
		level: number,
		xOffset: number,
		yOffset: number,
		format: number,
		type: number,
		source: unknown,
	): void;
	texParameteri(target: number, pname: number, param: number): void;
	deleteTexture(texture: unknown): void;
	enable(cap: number): void;
	disable(cap: number): void;
	blendFunc(sfactor: number, dfactor: number): void;
	isEnabled(cap: number): boolean;
	getParameter(pname: number): unknown;
}

/**
 * Create a stub WebGL2RenderingContext for testing.
 */
export function createStubWebGL2(): StubWebGL2 {
	let textureIdCounter = 0;

	const stub: StubWebGL2 = {
		TEXTURE_2D: 0x0de1,
		RGBA: 0x1908,
		UNSIGNED_BYTE: 0x1401,
		TEXTURE_MIN_FILTER: 0x2801,
		TEXTURE_MAG_FILTER: 0x2800,
		TEXTURE_WRAP_S: 0x2802,
		TEXTURE_WRAP_T: 0x2803,
		LINEAR: 0x2601,
		CLAMP_TO_EDGE: 0x812f,
		DEPTH_TEST: 0x0b71,
		BLEND: 0x0be2,
		CULL_FACE: 0x0b44,
		SRC_ALPHA: 0x0302,
		ONE_MINUS_SRC_ALPHA: 0x0303,

		_textureCount: 0,
		_texImage2DCount: 0,
		_texSubImage2DCount: 0,
		_deleteTextureCount: 0,

		createTexture() {
			stub._textureCount++;
			return { _id: ++textureIdCounter };
		},
		bindTexture(_target, _texture) {
			// no-op
		},
		texImage2D(_target, _level, _internalFormat, _format, _type, _source) {
			stub._texImage2DCount++;
		},
		texSubImage2D(
			_target,
			_level,
			_xOffset,
			_yOffset,
			_format,
			_type,
			_source,
		) {
			stub._texSubImage2DCount++;
		},
		texParameteri(_target, _pname, _param) {
			// no-op
		},
		deleteTexture(_texture) {
			stub._deleteTextureCount++;
		},
		enable(_cap) {
			// no-op
		},
		disable(_cap) {
			// no-op
		},
		blendFunc(_sfactor, _dfactor) {
			// no-op
		},
		isEnabled(_cap) {
			return false;
		},
		getParameter(_pname) {
			return null;
		},
	};

	return stub;
}
