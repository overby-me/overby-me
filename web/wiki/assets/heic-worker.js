// Decodes HEIC off the main thread. Paired with the `heicDecode` glue in
// index.html, which owns this worker and hands out the object URLs.
//
// Everything expensive happens here: the wasm decode, the downscale and the
// JPEG encode. The main thread only ever receives a finished Blob, so a photo
// that takes seconds to decode costs the page no frames at all.
//
// Classic worker (importScripts, not an ES module) because the wasm-bindgen
// `no-modules` output is what the justfile builds, and module workers are the
// one worker feature Firefox shipped late.

importScripts("/heic-decode.js");

// Kicked off at load rather than on the first message, so the fetch and compile
// overlap with the request that woke this worker.
//
// The path is given explicitly, and as an object: the glue can infer it from
// `document.currentScript`, but a worker has no document, and the bare-string
// form is the deprecated one.
const ready = wasm_bindgen({ module_or_path: "/heic-decode_bg.wasm" });

// The longest edge a decoded photo is reduced to. Above a retina phone's own
// screen, so opening a picture still shows more than the page did, and far
// below the eleven megapixels a phone camera writes, which nothing displays and
// which would be tens of megabytes of JPEG to hand back.
const MAX_EDGE = 2048;

self.onmessage = async (event) => {
	const { id, bytes } = event.data;
	try {
		await ready;
		const decoded = wasm_bindgen.decode(bytes);
		if (!decoded) {
			self.postMessage({ id, error: "not a HEIC this decoder reads" });
			return;
		}
		const { width, height, rgba } = decoded;
		const scale = Math.min(1, MAX_EDGE / Math.max(width, height));
		const w = Math.max(1, Math.round(width * scale));
		const h = Math.max(1, Math.round(height * scale));

		// Full size first, because putImageData cannot scale; drawImage then
		// resamples into the canvas we actually encode. The large one is
		// dropped as soon as this scope ends.
		const full = new OffscreenCanvas(width, height);
		full.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(rgba.buffer), width, height), 0, 0);

		let out = full;
		if (scale < 1) {
			out = new OffscreenCanvas(w, h);
			out.getContext("2d").drawImage(full, 0, 0, w, h);
		}

		const blob = await out.convertToBlob({ type: "image/jpeg", quality: 0.85 });
		self.postMessage({ id, blob });
	} catch (err) {
		self.postMessage({ id, error: String(err) });
	}
};
