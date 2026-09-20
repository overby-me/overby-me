//! Turn a Windows metafile into a picture a browser can draw.
//!
//! Word and PowerPoint keep pasted figures as EMF, EMF+ or WMF, Windows' own
//! vector formats, which no browser has ever displayed. Carried over from the
//! interim backend (`backend/src/metafile.rs`).
//!
//! It is rendered HERE rather than in the app because the renderer is large:
//! `emfsdk` with its `render` feature compiles to about 400 KB gzipped, a fifth
//! again on top of a wasm bundle that is already the heaviest thing a delegate
//! downloads. On this side it costs the reader nothing, and it costs a request
//! only for the documents that actually contain one.
//!
//! The caller posts the bytes it already has: it parsed the package to find the
//! picture in the first place, so this needs no storage access and grants no
//! access of its own. A session is still required, to keep it from being a free
//! conversion service for anyone who finds the URL.

use crate::session::Caller;
use crate::xrpc::{err, invalid, write_failed};
use axum::body::Body;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};

/// Largest metafile accepted. Comfortably above anything Office produces (the
/// two in the wiki are 27 KB and 22 KB).
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;

/// Width the picture is rendered at, and the ceiling on total pixels.
///
/// A metafile is vector and has no natural pixel size, so something has to
/// choose. Wide enough to read a pasted table on a projector, bounded so a
/// document that claims an enormous canvas cannot turn one request into a
/// gigabyte of RGBA.
const TARGET_WIDTH_PX: u32 = 1600;
const MAX_PIXELS: u32 = 4_000_000;

/// SVG when the emitter could draw it, PNG when only the rasteriser could.
struct Rendered {
    bytes: Vec<u8>,
    content_type: &'static str,
}

fn render(bytes: &[u8]) -> Result<Option<Rendered>, String> {
    // SVG first. It hands text to the browser, which picks the font each record
    // asks for instead of drawing everything in the one generic sans the
    // rasteriser loads, and it scales without resampling. The raster stays as
    // the answer for anything the emitter declines.
    if let Some(svg) = crate::metafile_svg::to_svg(bytes) {
        return Ok(Some(Rendered {
            bytes: svg.into_bytes(),
            content_type: "image/svg+xml",
        }));
    }
    // `..Default::default()` on purpose: the renderer keeps adding opt-in
    // options, and naming every field would turn each into a build break here.
    let options = emfsdk::render::RenderOptions {
        target_width_px: Some(TARGET_WIDTH_PX),
        max_pixels: Some(MAX_PIXELS),
        ..Default::default()
    };
    emfsdk::render::decode_metafile_as_raster_with_options(bytes, None, options)
        .map(|out| {
            out.map(|out| Rendered {
                bytes: out.data,
                content_type: out.content_type,
            })
        })
        .map_err(|e| e.to_string())
}

/// `wiki.radikal.renderMetafile` (procedure): the metafile as the body, a
/// picture back. Any signed-in caller: there is nothing to authorise against,
/// since the bytes come from the request and the caller already holds whatever
/// this could tell them.
pub async fn render_metafile(_caller: Caller, body: Body) -> Response {
    let Ok(bytes) = axum::body::to_bytes(body, MAX_INPUT_BYTES).await else {
        return err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "MetafileTooLarge",
            "the metafile is too large",
        );
    };
    if bytes.is_empty() {
        return invalid("no metafile");
    }
    // Rendering is CPU-bound and the runtime is shared.
    match tokio::task::spawn_blocking(move || render(&bytes)).await {
        Ok(Ok(Some(picture))) => (
            StatusCode::OK,
            [
                (CONTENT_TYPE, picture.content_type),
                (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            picture.bytes,
        )
            .into_response(),
        // Not a metafile at all, or one this renderer does not cover. Both are
        // the caller's answer to give: the viewer falls back to a placeholder.
        Ok(Ok(None)) => invalid("not a renderable metafile"),
        Ok(Err(why)) => invalid(&format!("cannot render: {why}")),
        Err(e) => write_failed("renderMetafile", e),
    }
}

#[cfg(test)]
mod tests {
    use crate::router;
    use crate::xrpc::tests::{seeded_state, token_for};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// The smallest EMF there is: a header, one rectangle, the end record
    /// (MS-EMF 2.3.4.2, 2.3.5.34, 2.3.4.1).
    fn one_rectangle() -> Vec<u8> {
        let header = 88u32;
        let rectangle = 24u32;
        let eof = 20u32;
        let mut emf = Vec::new();
        let mut put = |values: &[u32]| {
            for value in values {
                emf.extend_from_slice(&value.to_le_bytes());
            }
        };
        put(&[1, header]);
        put(&[0, 0, 100, 50]); // bounds, device units
        put(&[0, 0, 2000, 1000]); // frame, 0.01 mm
        put(&[0x464D_4520, 0x0001_0000, header + rectangle + eof, 3]);
        put(&[1, 0, 0, 0]); // handles and reserved, description length and offset, palette
        put(&[1920, 1080, 508, 286]); // device in pixels, then in millimetres
        put(&[0x2B, rectangle, 10, 10, 90, 40]);
        put(&[0x0E, eof, 0, 0, eof]);
        emf
    }

    async fn render(token: Option<&str>, bytes: Vec<u8>) -> (StatusCode, String, Vec<u8>) {
        let state = seeded_state().await;
        let mut req = Request::builder()
            .method("POST")
            .uri("/xrpc/wiki.radikal.renderMetafile");
        let bob;
        if token.is_some() {
            bob = token_for(&state, "did:plc:bob").await;
            req = req.header("authorization", format!("Bearer {bob}"));
        }
        let resp = router(state)
            .oneshot(req.body(Body::from(bytes)).expect("request"))
            .await
            .expect("response");
        let status = resp.status();
        let kind = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = axum::body::to_bytes(resp.into_body(), 1 << 22)
            .await
            .expect("body");
        (status, kind, body.to_vec())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_metafile_comes_back_as_a_picture_a_browser_can_draw() {
        let (status, kind, body) = render(Some("bob"), one_rectangle()).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(kind, "image/svg+xml");
        let svg = String::from_utf8(body).expect("utf-8");
        assert!(svg.starts_with("<svg") && svg.contains("<rect"), "{svg}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_is_not_a_metafile_is_the_callers_to_handle() {
        let (status, _, _) = render(Some("bob"), b"%PDF-1.7 not a metafile".to_vec()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            render(Some("bob"), Vec::new()).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            render(None, one_rectangle()).await.0,
            StatusCode::UNAUTHORIZED,
            "a free conversion service for whoever finds the URL"
        );
    }
}
