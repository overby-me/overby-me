//! The pixel canvas: a shared grid a room paints together, one cell at a time.
//!
//! Drawn into a single `<canvas>` sized in CELLS and scaled up by CSS, so a
//! 64x64 board is a 64x64 bitmap the browser stretches with
//! `image-rendering: pixelated`. Four thousand elements diffed by Dioxus on
//! every placement would not survive a phone; one bitmap is one texture upload,
//! and zoom costs nothing because it is a CSS width.
//!
//! Placements arrive over a STREAMING subscription, so each one is a frame
//! carrying the cell that changed rather than the whole board re-sent to
//! everybody. The cooldown between placements is enforced by a database trigger
//! (`migrations/0007`), not here: this only says when, and says it kindly.

use dioxus::prelude::*;
use std::collections::HashMap;
use wasm_bindgen::JsCast;

use crate::graphql;
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::session::use_session;

/// The palette, as CSS colours. The stored value is the INDEX, so a cell costs a
/// single digit in the database and the palette can be restyled later without
/// rewriting every row.
pub const PALETTE: &[&str] = &[
    "#ffffff", "#e4e4e4", "#888888", "#222222", "#ffa7d1", "#e50000", "#e59500", "#a06a42",
    "#e5d900", "#94e044", "#02be01", "#00d3dd", "#0083c7", "#0000ea", "#cf6ee4", "#820080",
];

/// The palette entry for a stored index, falling back rather than panicking on a
/// colour some future build wrote and this one does not know.
fn hex_of(index: u8) -> &'static str {
    PALETTE.get(index as usize).copied().unwrap_or(PALETTE[0])
}

/// The board's 2D context, by element id.
///
/// Looked up per draw rather than held: the element belongs to Dioxus, which may
/// replace it, and a stale handle would draw into a canvas nobody can see.
fn board_context(dom_id: &str) -> Option<web_sys::CanvasRenderingContext2d> {
    let canvas = web_sys::window()?
        .document()?
        .get_element_by_id(dom_id)?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .ok()?;
    canvas
        .get_context("2d")
        .ok()??
        .dyn_into::<web_sys::CanvasRenderingContext2d>()
        .ok()
}

/// Paint one cell into the bitmap. The canvas is sized in CELLS, so a cell is
/// literally one pixel and CSS does the magnifying.
fn draw_cell(ctx: &web_sys::CanvasRenderingContext2d, x: u32, y: u32, colour: u8) {
    ctx.set_fill_style_str(hex_of(colour));
    ctx.fill_rect(x as f64, y as f64, 1.0, 1.0);
}

/// Paint the whole board, for the initial load and for a tab returning to life.
fn draw_all(dom_id: &str, cols: u32, rows: u32, cells: &HashMap<(u32, u32), u8>) {
    let Some(ctx) = board_context(dom_id) else {
        return;
    };
    ctx.set_fill_style_str(PALETTE[0]);
    ctx.fill_rect(0.0, 0.0, cols as f64, rows as f64);
    for (&(x, y), &c) in cells.iter() {
        if x < cols && y < rows {
            draw_cell(&ctx, x, y, c);
        }
    }
}

/// The canvas geometry, from the node's `data`, clamped to something a browser
/// and a database can both survive.
fn geometry(node: &NodeWithChildren) -> (u32, u32, u32) {
    let read = |key: &str, fallback: u32| -> u32 {
        node.data
            .as_ref()
            .and_then(|d| d.0.get(key))
            .and_then(|v| v.as_u64())
            .unwrap_or(fallback as u64) as u32
    };
    (
        read("w", 64).clamp(1, graphql::MAX_CANVAS_SIDE),
        read("h", 64).clamp(1, graphql::MAX_CANVAS_SIDE),
        read("cooldown", 60),
    )
}

/// Which cell a click at `(px, py)` inside a `w x h` box of `cols x rows` hits.
///
/// Pure, and worth its own test: an off-by-one here paints the wrong cell, which
/// is the one bug in this app a user would find hilarious rather than annoying.
pub fn cell_at(px: f64, py: f64, box_w: f64, box_h: f64, cols: u32, rows: u32) -> Option<(u32, u32)> {
    if box_w <= 0.0 || box_h <= 0.0 || px < 0.0 || py < 0.0 || px >= box_w || py >= box_h {
        return None;
    }
    let x = ((px / box_w) * cols as f64).floor() as u32;
    let y = ((py / box_h) * rows as f64).floor() as u32;
    Some((x.min(cols - 1), y.min(rows - 1)))
}

/// Seconds still to wait, from the `retry_after_ms=…` a rate-limited insert
/// raises. `None` when the failure was something else.
pub fn retry_after_seconds(error: &str) -> Option<u32> {
    let rest = error.split("retry_after_ms=").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let ms: u64 = digits.parse().ok()?;
    Some(ms.div_ceil(1000) as u32)
}

#[component]
pub fn PixelApp(node: NodeWithChildren, #[props(default)] projector: bool) -> Element {
    let session = use_session();
    let (cols, rows_n, cooldown) = geometry(&node);
    let canvas_id = node.id.0.clone();
    let context_id = node
        .context_id
        .as_ref()
        .map(|c| c.0.clone())
        .unwrap_or_else(|| canvas_id.clone());
    let open = node.mutable;

    let mut cells = use_signal(HashMap::<(u32, u32), u8>::new);
    let mut colour = use_signal(|| 3u8);
    let mut cooling = use_signal(|| 0u32);
    let mut busy = use_signal(|| false);

    // The board as it stands, once. Everything after this arrives as a delta.
    let load_id = canvas_id.clone();
    let load_token = session.read().access_token.clone();
    let loaded = crate::use_data_resource!(|(load_id, load_token)| async move {
        graphql::load_canvas(load_token.as_deref(), &load_id)
            .await
            .unwrap_or_default()
    });
    let dom_id = use_hook(|| format!("pixel-board-{}", js_sys::Date::now() as u64));
    let draw_id = dom_id.clone();
    use_effect(move || {
        if let Some(rows) = loaded.read().clone() {
            {
                let mut map = cells.write();
                for ((x, y), c) in rows {
                    map.insert((x, y), c);
                }
            }
            draw_all(&draw_id, cols, rows_n, &cells.read());
        }
    });

    // Only what changed, as it changes. A stream carries nothing until something
    // happens, so the cursor starts at mount: the load above covers the past.
    let since = use_hook(|| {
        js_sys::Date::new_0()
            .to_iso_string()
            .as_string()
            .unwrap_or_default()
    });
    let stream_id = dom_id.clone();
    let stream = crate::subscription::use_graphql_subscription(graphql::canvas_stream(
        &canvas_id, &since,
    ));
    use_effect(move || {
        let Some(payload) = stream.read().clone() else {
            return;
        };
        let rows = payload
            .get("nodes_stream")
            .and_then(|n| n.as_array())
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            return;
        }
        let ctx = board_context(&stream_id);
        let mut map = cells.write();
        for row in rows.iter() {
            if let Some(((x, y), c)) = graphql::parse_cell(row) {
                map.insert((x, y), c);
                // One cell, not a re-render: this is what makes a busy canvas
                // cost the same as a quiet one.
                if let Some(ctx) = ctx.as_ref() {
                    draw_cell(ctx, x, y, c);
                }
            }
        }
    });

    // The cooldown ticks down locally. The database is the authority; this is so
    // the button can say how long rather than only that it is too soon.
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1000).await;
            let left = cooling();
            if left > 0 {
                cooling.set(left - 1);
            }
        }
    });

    let can_paint = session.read().is_authenticated() && open && !projector;
    let paint_canvas = canvas_id.clone();
    let click_id = dom_id.clone();
    let paint_ctx = context_id.clone();

    let on_click = move |evt: Event<MouseData>| {
        if !can_paint || cooling() > 0 || busy() {
            return;
        }
        // The click arrives in element space, and the element IS the board.
        let coords = evt.data().element_coordinates();
        let Some((x, y)) = cell_at(
            coords.x,
            coords.y,
            board_px(cols) as f64,
            board_px(rows_n) as f64,
            cols,
            rows_n,
        ) else {
            return;
        };
        let c = colour();
        // Optimistic: the cell is painted now and corrected by the stream if the
        // server disagrees, so a hall on slow wifi still feels immediate.
        cells.write().insert((x, y), c);
        if let Some(ctx) = board_context(&click_id) {
            draw_cell(&ctx, x, y, c);
        }
        busy.set(true);
        let token = session.read().access_token.clone();
        let (cv, ctx) = (paint_canvas.clone(), paint_ctx.clone());
        spawn(async move {
            let result = graphql::paint_cell(token.as_deref(), &cv, &ctx, x, y, c).await;
            busy.set(false);
            match result {
                Ok(()) => cooling.set(cooldown),
                Err(e) => {
                    // A refusal that knows when is not a failure to report; it is
                    // an instruction. Anything else is a real error and is already
                    // logged and filed by `execute_raw_vars`.
                    if let Some(secs) = retry_after_seconds(&e) {
                        cooling.set(secs);
                    }
                }
            }
        });
    };

    let cells_now = cells.read().clone();
    let painted = cells_now.len();

    rsx! {
        div { class: "pixel-app",
            div { class: "pixel-head",
                h2 { class: "pixel-title", "{node.name}" }
                span { class: "pixel-count", "{painted} / {cols * rows_n}" }
                if !open {
                    span { class: "chip pixel-closed", "{t(\"pixel.closed\")}" }
                }
            }

            // The board. Sized in cells, scaled by CSS: see the module comment.
            canvas {
                id: "{dom_id}",
                class: "pixel-board",
                width: "{cols}",
                height: "{rows_n}",
                style: "width: {board_px(cols)}px; height: {board_px(rows_n)}px;",
                onclick: on_click,
            }

            if can_paint {
                div { class: "pixel-palette",
                    for (i , hex) in PALETTE.iter().enumerate() {
                        button {
                            class: if colour() == i as u8 { "pixel-swatch is-picked" } else { "pixel-swatch" },
                            key: "s{i}",
                            style: "background: {hex};",
                            r#type: "button",
                            aria_label: "{t(\"pixel.colour\")} {i + 1}",
                            onclick: move |_| colour.set(i as u8),
                        }
                    }
                }
                div { class: "pixel-status",
                    if cooling() > 0 {
                        span { class: "pixel-wait", "{t(\"pixel.waitSeconds\")} {cooling()}" }
                    } else {
                        span { "{t(\"pixel.yourTurn\")}" }
                    }
                }
            }
        }
    }
}

/// The board's on-screen size for a given number of cells.
///
/// Kept modest so a 128-wide board still fits a phone with the page zoomed out;
/// the CSS lets it scroll rather than shrinking a cell below a fingertip.
fn board_px(cells: u32) -> u32 {
    (cells * 12).clamp(120, 1536)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A click lands on the cell under the finger, including at the very edges.
    #[test]
    fn a_click_maps_to_the_cell_under_it() {
        // A 64-cell board drawn 640px wide: 10px per cell.
        assert_eq!(cell_at(0.0, 0.0, 640.0, 640.0, 64, 64), Some((0, 0)));
        assert_eq!(cell_at(9.9, 0.0, 640.0, 640.0, 64, 64), Some((0, 0)));
        assert_eq!(cell_at(10.0, 0.0, 640.0, 640.0, 64, 64), Some((1, 0)));
        assert_eq!(cell_at(639.9, 639.9, 640.0, 640.0, 64, 64), Some((63, 63)));
        // Outside the board paints nothing at all.
        assert_eq!(cell_at(-1.0, 5.0, 640.0, 640.0, 64, 64), None);
        assert_eq!(cell_at(640.0, 5.0, 640.0, 640.0, 64, 64), None);
        assert_eq!(cell_at(5.0, 5.0, 0.0, 0.0, 64, 64), None);
    }

    /// A rate-limited refusal tells the user WHEN, which is the whole point of
    /// carrying `retry_after_ms` out of the trigger.
    #[test]
    fn a_refusal_says_how_long_to_wait() {
        assert_eq!(
            retry_after_seconds("rate limited: retry_after_ms=59945"),
            Some(60)
        );
        assert_eq!(retry_after_seconds("retry_after_ms=1"), Some(1));
        assert_eq!(retry_after_seconds("retry_after_ms=0"), Some(0));
        // Any other failure is not a cooldown and must not be shown as one.
        assert_eq!(retry_after_seconds("permission denied"), None);
        assert_eq!(retry_after_seconds("retry_after_ms=abc"), None);
    }

    /// The board never asks the browser for a size it cannot draw.
    #[test]
    fn the_board_stays_a_sane_size() {
        assert_eq!(board_px(1), 120);
        assert_eq!(board_px(64), 768);
        assert_eq!(board_px(1000), 1536);
    }
}
