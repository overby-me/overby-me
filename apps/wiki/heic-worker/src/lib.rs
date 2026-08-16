//! HEIC decoding, off the main thread.
//!
//! A decode of a current iPhone photo (eleven megapixels) takes on the order of
//! two seconds. Run on the main thread that is two seconds in which the page
//! cannot scroll, animate or respond to a tap -- and a feed with three such
//! photos froze for the sum of them. Nothing about it is I/O, so `async` on the
//! main thread does not help: there is no await point inside a decode to yield
//! at. It has to be a different thread, which on the web means a Worker.
//!
//! This crate is only the pixels. Scaling and JPEG encoding happen in
//! `assets/heic-worker.js` on an `OffscreenCanvas`, so the main thread receives
//! a finished Blob and does no image work at all.

use wasm_bindgen::prelude::*;

/// Decode HEIF/HEIC bytes to interleaved RGBA.
///
/// Returns `{width, height, rgba}`, or null if the bytes are not a HEIC this
/// decoder reads (AVIF and JPEG-in-HEIF among them). A plain object rather than
/// an exported struct: the caller is hand-written JS, and this keeps the whole
/// result to one postMessage-able value.
///
/// `rgba` is copied into a fresh `Uint8Array` rather than handed out as a view
/// into wasm memory. A view is invalidated the moment the allocator grows the
/// heap, and eleven megapixels is 43 MB that will do exactly that.
#[wasm_bindgen]
pub fn decode(data: &[u8]) -> JsValue {
    let Ok(image) = heif_oxide::decode_bytes(data) else {
        return JsValue::NULL;
    };
    let rgba = image.to_rgba8();
    let out = js_sys::Object::new();
    let set = |k: &str, v: &JsValue| {
        let _ = js_sys::Reflect::set(&out, &JsValue::from_str(k), v);
    };
    set("width", &JsValue::from(image.width));
    set("height", &JsValue::from(image.height));
    set("rgba", &js_sys::Uint8Array::from(&rgba[..]));
    out.into()
}
