//! Turn a Windows metafile into a picture a browser can draw.
//!
//! Word and PowerPoint keep pasted figures as EMF, EMF+ or WMF, Windows' own
//! vector formats, which no browser has ever displayed. The viewer used to draw
//! a labelled placeholder in the shape's box and say so.
//!
//! It is rendered HERE rather than in the app because the renderer is large:
//! `emfsdk` with its `render` feature compiles to about 400 KB gzipped, a fifth
//! again on top of a wasm bundle that is already the heaviest thing a delegate
//! downloads. On this side it costs the reader nothing, and it costs a request
//! only for the documents that actually contain one, which today is one of the
//! 322 Office files in the wiki.
//!
//! The caller posts the bytes it already has: it parsed the package to find the
//! picture in the first place, so this needs no storage access and grants no
//! access of its own. A token is still required, to keep it from being a free
//! conversion service for anyone who finds the URL.

use axum::body::{Body, Bytes};
use axum::http::{Request, Response, StatusCode};

use crate::error::AppError;
use crate::oauth::Config;

/// Largest metafile accepted. Comfortably above anything Office produces (the
/// two in the wiki are 27 KB and 22 KB) and far below anything that could hurt
/// a 256 MB container.
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;

/// Width the picture is rendered at, and the ceiling on total pixels.
///
/// A metafile is vector and has no natural pixel size, so something has to
/// choose. Wide enough to read a pasted table on a projector, bounded so a
/// document that claims an enormous canvas cannot turn one request into a
/// gigabyte of RGBA.
const TARGET_WIDTH_PX: u32 = 1600;
const MAX_PIXELS: u32 = 4_000_000;

/// Takes the whole request, like the roster upload does: the metafile arrives as
/// the body, and the dispatcher cannot hand out both a body and a borrowed query.
pub async fn render(
    cfg: &Config,
    client: &reqwest::Client,
    req: Request<Body>,
    bearer: Option<&str>,
) -> Response<Body> {
    let query = req.uri().query().map(str::to_string);
    let body = match axum::body::to_bytes(req.into_body(), MAX_INPUT_BYTES + 1).await {
        Ok(b) => b,
        Err(_) => {
            return AppError::BadRequest("metafile too large".into()).respond("metafile render")
        }
    };
    match render_inner(cfg, client, query.as_deref(), bearer, body).await {
        Ok(png) => png_response(png),
        Err(e) => e.respond("metafile render"),
    }
}

async fn render_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
    body: Bytes,
) -> Result<Vec<u8>, AppError> {
    // Any signed-in caller. There is nothing here to authorise against: the
    // bytes come from the request, not from storage, so the caller is already
    // holding whatever this could tell them.
    crate::auth::caller(cfg, client, query, bearer).await?;

    if body.is_empty() {
        return Err(AppError::BadRequest("no metafile".into()));
    }
    if body.len() > MAX_INPUT_BYTES {
        return Err(AppError::BadRequest("metafile too large".into()));
    }

    // `..Default::default()` on purpose: the renderer keeps adding opt-in
    // options (transparent backgrounds, pattern-brush filtering) and naming
    // every field would turn each of those into a build break here.
    let options = emfsdk::render::RenderOptions {
        target_width_px: Some(TARGET_WIDTH_PX),
        max_pixels: Some(MAX_PIXELS),
        ..Default::default()
    };
    // Rendering is CPU-bound and the runtime is shared, so it goes to a blocking
    // thread rather than stalling every other request on this container.
    let bytes = body.to_vec();
    let decoded = tokio::task::spawn_blocking(move || {
        emfsdk::render::decode_metafile_as_raster_with_options(&bytes, None, options)
    })
    .await
    .map_err(|e| AppError::Upstream(format!("render task failed: {e}")))?;

    match decoded {
        Ok(Some(out)) => Ok(out.data),
        // Not a metafile at all, or one this renderer does not cover. Both are
        // the caller's answer to give: the viewer falls back to the placeholder
        // it drew before this existed.
        Ok(None) => Err(AppError::BadRequest("not a renderable metafile".into())),
        Err(e) => Err(AppError::BadRequest(format!("cannot render: {e}"))),
    }
}

fn png_response(png: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "image/png")
        // A metafile's rendering is a pure function of its bytes, and the bytes
        // live in an immutable uploaded document.
        .header("cache-control", "public, max-age=31536000, immutable")
        // The app is on another origin, so without this the browser discards a
        // perfectly good picture it has already downloaded. The error path gets
        // this for free through `crate::json`; a hand-built response does not.
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(png))
        .unwrap_or_else(|e| AppError::Upstream(e.to_string()).respond("metafile response"))
}
