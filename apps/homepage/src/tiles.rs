//! Fetching map tiles for `mapscroller`.
//!
//! Upstream forks a perl helper for this and says why: "doing https from C
//! code is untenable". A browser is the one place where that is not true, so
//! this is a `fetch` and a decode.
//!
//! Two things make it work without a proxy. openstreetmap.org's tiles send
//! `access-control-allow-origin: *`, so the canvas they are drawn into is not
//! tainted and can be read back; and the browser's own HTTP cache does the job
//! upstream's on-disk tile cache was written for, so there is nothing here to
//! manage or expire.
//!
//! The tile servers ask that clients identify themselves and not hammer them.
//! Requests are capped in flight for that reason, and because a saver that
//! asks for sixty tiles at once on a slow link is better served by getting the
//! middle of the screen first anyway.

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageBitmap, Response};
use xscreensaver::runtime::{XImage, color::rgb};

/// How many tiles to have in flight at once.
const MAX_IN_FLIGHT: usize = 6;

thread_local! {
    static IN_FLIGHT: RefCell<usize> = const { RefCell::new(0) };
}

/// Whether there is room to start another fetch.
pub fn can_start() -> bool {
    IN_FLIGHT.with(|n| *n.borrow() < MAX_IN_FLIGHT)
}

/// Fetch one tile and decode it, or `None` if it could not be had.
///
/// A failure is an ordinary answer rather than an error: the saver marks that
/// tile with a cross and carries on, exactly as upstream does with a 404.
pub async fn fetch(url: String) -> Option<XImage> {
    IN_FLIGHT.with(|n| *n.borrow_mut() += 1);
    let out = decode(&url).await;
    IN_FLIGHT.with(|n| {
        let mut n = n.borrow_mut();
        *n = n.saturating_sub(1);
    });
    match out {
        Ok(img) => Some(img),
        Err(e) => {
            log::warn!("screensaver tiles: {e}");
            None
        }
    }
}

async fn decode(url: &str) -> Result<XImage, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|_| format!("network error fetching {url}"))?
        .dyn_into()
        .map_err(|_| "unexpected fetch result".to_string())?;
    if !response.ok() {
        return Err(format!("HTTP {} from {url}", response.status()));
    }
    let blob = JsFuture::from(response.blob().map_err(|_| "no body")?)
        .await
        .map_err(|_| "could not read the body".to_string())?;

    // Decoding from a blob we already fetched keeps this same-origin as far as
    // the canvas is concerned, so `getImageData` works even though the bytes
    // came from another host.
    let bitmap: ImageBitmap = JsFuture::from(
        window
            .create_image_bitmap_with_blob(&blob.dyn_into().map_err(|_| "not a blob")?)
            .map_err(|_| "could not decode the tile".to_string())?,
    )
    .await
    .map_err(|_| "could not decode the tile".to_string())?
    .dyn_into()
    .map_err(|_| "not an image".to_string())?;

    let (w, h) = (bitmap.width() as i32, bitmap.height() as i32);
    let document = window.document().ok_or("no document")?;
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|_| "no canvas")?
        .dyn_into()
        .map_err(|_| "not a canvas")?;
    canvas.set_width(w as u32);
    canvas.set_height(h as u32);
    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|_| "no 2d context")?
        .ok_or("no 2d context")?
        .dyn_into()
        .map_err(|_| "not a 2d context")?;
    ctx.draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
        .map_err(|_| "could not draw the tile".to_string())?;
    bitmap.close();

    let data = ctx
        .get_image_data(0.0, 0.0, f64::from(w), f64::from(h))
        .map_err(|_| "the canvas was tainted, so the tile cannot be read".to_string())?
        .data();

    let mut img = XImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            img.put_pixel(x, y, rgb(data[i], data[i + 1], data[i + 2]));
        }
    }
    Ok(img)
}
