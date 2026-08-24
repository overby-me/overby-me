//! WebCodecs decoding for surfaces in video mode: one VideoDecoder per
//! window, painting decoded frames onto its 2D canvas so every consumer
//! (flat desk, 3D textures) stays oblivious.
//!
//! Driven through dynamic JS: web-sys keeps WebCodecs behind unstable cfg.

use std::cell::RefCell;
use std::collections::BTreeMap;

use dioxus::logger::tracing;
use js_sys::{Function, Object, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::HtmlCanvasElement;
use webxr_compositor_protocol as protocol;

struct Decoder {
    js: JsValue,
    /// Chunks before the first keyframe cannot decode; drop them.
    primed: bool,
    timestamp: f64,
}

thread_local! {
    static DECODERS: RefCell<BTreeMap<protocol::WindowId, Decoder>> =
        const { RefCell::new(BTreeMap::new()) };
}

pub fn supported() -> bool {
    Reflect::has(&js_sys::global(), &"VideoDecoder".into()).unwrap_or(false)
}

/// Feed one encoded frame to the window's decoder, creating it on demand.
pub fn frame(id: protocol::WindowId, keyframe: bool, data: &[u8]) {
    DECODERS.with(|cell| {
        let mut map = cell.borrow_mut();
        if let std::collections::btree_map::Entry::Vacant(slot) = map.entry(id) {
            let Some(js) = create_decoder(id) else {
                return;
            };
            slot.insert(Decoder {
                js,
                primed: false,
                timestamp: 0.0,
            });
        }
        let Some(decoder) = map.get_mut(&id) else {
            return;
        };
        if !decoder.primed {
            if !keyframe {
                return;
            }
            decoder.primed = true;
        }
        if let Err(error) = decode_chunk(decoder, keyframe, data) {
            tracing::warn!(?error, "video decode failed for window {id}");
            let dead = map.remove(&id);
            if let Some(dead) = dead {
                close_js(&dead.js);
            }
        }
    });
}

/// A plain Frame means the stream ended (or never applied); drop state.
pub fn close(id: protocol::WindowId) {
    DECODERS.with(|cell| {
        if let Some(decoder) = cell.borrow_mut().remove(&id) {
            close_js(&decoder.js);
        }
    });
}

pub fn close_all() {
    DECODERS.with(|cell| {
        for (_, decoder) in std::mem::take(&mut *cell.borrow_mut()) {
            close_js(&decoder.js);
        }
    });
}

fn create_decoder(id: protocol::WindowId) -> Option<JsValue> {
    let global = js_sys::global();
    let ctor: Function = Reflect::get(&global, &"VideoDecoder".into())
        .ok()?
        .dyn_into()
        .ok()?;

    let output = Closure::<dyn FnMut(JsValue)>::new(move |frame: JsValue| {
        paint(id, &frame);
        let _ = Reflect::get(&frame, &"close".into())
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok())
            .map(|f| f.call0(&frame));
    });
    let error = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
        tracing::warn!("video decoder error for window {id}: {e:?}");
    });

    let init = Object::new();
    Reflect::set(&init, &"output".into(), output.as_ref()).ok()?;
    Reflect::set(&init, &"error".into(), error.as_ref()).ok()?;
    output.forget();
    error.forget();

    let decoder = Reflect::construct(&ctor, &js_sys::Array::of1(&init)).ok()?;

    let config = Object::new();
    // Annex B without a description, which is exactly what openh264 emits.
    Reflect::set(&config, &"codec".into(), &"avc1.42E01E".into()).ok()?;
    Reflect::set(&config, &"optimizeForLatency".into(), &JsValue::TRUE).ok()?;
    let configure: Function = Reflect::get(&decoder, &"configure".into())
        .ok()?
        .dyn_into()
        .ok()?;
    configure.call1(&decoder, &config).ok()?;

    Some(decoder)
}

fn decode_chunk(decoder: &mut Decoder, keyframe: bool, data: &[u8]) -> Result<(), JsValue> {
    let global = js_sys::global();
    let ctor: Function = Reflect::get(&global, &"EncodedVideoChunk".into())?.dyn_into()?;
    let init = Object::new();
    let kind = if keyframe { "key" } else { "delta" };
    Reflect::set(&init, &"type".into(), &kind.into())?;
    Reflect::set(&init, &"timestamp".into(), &decoder.timestamp.into())?;
    Reflect::set(&init, &"data".into(), &Uint8Array::from(data).into())?;
    decoder.timestamp += 16_666.0;
    let chunk = Reflect::construct(&ctor, &js_sys::Array::of1(&init))?;
    let decode: Function = Reflect::get(&decoder.js, &"decode".into())?.dyn_into()?;
    decode.call1(&decoder.js, &chunk)?;
    Ok(())
}

/// Draw a decoded VideoFrame onto the window's canvas.
fn paint(id: protocol::WindowId, frame: &JsValue) {
    let number = |name: &str| {
        Reflect::get(frame, &name.into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let width = number("displayWidth") as u32;
    let height = number("displayHeight") as u32;
    let Some(canvas) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(&format!("win-{id}")))
        .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
    else {
        return;
    };
    if width > 0 && (canvas.width() != width || canvas.height() != height) {
        canvas.set_width(width);
        canvas.set_height(height);
    }
    let Ok(Some(context)) = canvas.get_context("2d") else {
        return;
    };
    let context: JsValue = context.into();
    let _ = Reflect::get(&context, &"drawImage".into())
        .ok()
        .and_then(|f| f.dyn_into::<Function>().ok())
        .map(|f| f.call3(&context, frame, &0.0.into(), &0.0.into()));
}

fn close_js(decoder: &JsValue) {
    let state = Reflect::get(decoder, &"state".into())
        .ok()
        .and_then(|s| s.as_string());
    if state.as_deref() != Some("closed")
        && let Ok(close) = Reflect::get(decoder, &"close".into())
        && let Ok(close) = close.dyn_into::<Function>()
    {
        let _ = close.call0(decoder);
    }
}
