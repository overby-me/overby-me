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

/// How many cells a new canvas is across and down.
///
/// Thirty-two, not sixty-four, because a board scales to the width of a phone: at
/// 64 a cell is about five pixels on a 360px screen, which is not something a
/// finger can aim at. A canvas can still be made larger — the size lives in the
/// node's `data` — but the default should be paintable on the device most people
/// will have in the hall.
pub const DEFAULT_SIDE: u32 = 32;

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

/// The board's size on screen, in CSS pixels.
///
/// Not the cell count times a constant: the stylesheet scales the board down to
/// fit a phone, and a click mapped against the unscaled size would paint a cell
/// up to twice as far across as the one under the finger.
fn board_size(dom_id: &str) -> Option<(f64, f64)> {
    let el = web_sys::window()?.document()?.get_element_by_id(dom_id)?;
    let rect = el.get_bounding_client_rect();
    (rect.width() > 0.0 && rect.height() > 0.0).then(|| (rect.width(), rect.height()))
}

/// Paint one cell into the bitmap. The canvas is sized in CELLS, so a cell is
/// literally one pixel and CSS does the magnifying.
fn draw_cell(ctx: &web_sys::CanvasRenderingContext2d, x: u32, y: u32, colour: u8) {
    ctx.set_fill_style_str(hex_of(colour));
    ctx.fill_rect(x as f64, y as f64, 1.0, 1.0);
}

/// Paint the whole board, for the initial load and for a tab returning to life.
pub(crate) fn draw_all_of(dom_id: &str, cols: u32, rows: u32, cells: &HashMap<(u32, u32), u8>) {
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
        read("w", DEFAULT_SIDE).clamp(1, graphql::MAX_CANVAS_SIDE),
        read("h", DEFAULT_SIDE).clamp(1, graphql::MAX_CANVAS_SIDE),
        read("cooldown", 60),
    )
}

/// Which cell a click at `(px, py)` inside a `w x h` box of `cols x rows` hits.
///
/// Pure, and worth its own test: an off-by-one here paints the wrong cell, which
/// is the one bug in this app a user would find hilarious rather than annoying.
pub fn cell_at(
    px: f64,
    py: f64,
    box_w: f64,
    box_h: f64,
    cols: u32,
    rows: u32,
) -> Option<(u32, u32)> {
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
            draw_all_of(&draw_id, cols, rows_n, &cells.read());
        }
    });

    // How long this device must still wait, asked of the server. A refusal from
    // the trigger does not carry a readable reason outside dev mode, and a fresh
    // tab has no memory of a placement made a minute ago, so the wait is derived
    // from when this person last painted rather than from an error.
    let wait_id = canvas_id.clone();
    let wait_token = session.read().access_token.clone();
    let wait_user = session.read().user.as_ref().map(|u| u.id.clone());
    let last_paint = crate::use_data_resource!(|(wait_id, wait_token, wait_user)| async move {
        let uid = wait_user?;
        graphql::my_last_paint(wait_token.as_deref(), &wait_id, &uid).await
    });
    use_effect(move || {
        let Some(Some(iso)) = last_paint.read().clone() else {
            return;
        };
        let then = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(&iso)).get_time();
        let elapsed = (js_sys::Date::now() - then) / 1000.0;
        let left = (cooldown as f64 - elapsed).max(0.0) as u32;
        if left > 0 {
            cooling.set(left);
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
    let stream =
        crate::subscription::use_graphql_subscription(graphql::canvas_stream(&canvas_id, &since));
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
    let undo_id = dom_id.clone();
    let paint_ctx = context_id.clone();

    let on_click = move |evt: Event<MouseData>| {
        if !can_paint || cooling() > 0 || busy() {
            return;
        }
        // The click arrives in element space, and the element IS the board — but
        // its size on screen is whatever CSS gave it, which on a phone is not the
        // number of cells times twelve. Asking the element means a tap lands on
        // the cell under the finger at any width, including after a pinch zoom.
        let coords = evt.data().element_coordinates();
        let (box_w, box_h) = match board_size(&click_id) {
            Some(size) => size,
            None => (board_px(cols) as f64, board_px(rows_n) as f64),
        };
        let Some((x, y)) = cell_at(coords.x, coords.y, box_w, box_h, cols, rows_n) else {
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
        let undo = undo_id.clone();
        spawn(async move {
            let result = graphql::paint_cell(token.as_deref(), &cv, &ctx, x, y, c).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    log::info!("painted {x},{y}");
                    cooling.set(cooldown)
                }
                Err(e) => {
                    // The overwhelmingly common refusal is the cooldown, and the
                    // database does not get to explain itself: Hasura answers
                    // "database query error" and hides the reason outside dev
                    // mode. So a refused placement is undone rather than reported
                    // as a fault, and the countdown starts. It is logged, not
                    // filed, because "you were too quick" is not a bug.
                    log::info!("paint {x},{y} refused: {e}");
                    // Take the optimistic cell back: it is not on the board.
                    cells.write().remove(&(x, y));
                    draw_all_of(&undo, cols, rows_n, &cells.read());
                    // A refusal that knows when is not a failure to report; it is
                    // an instruction. Anything else is a real error and is already
                    // logged and filed by `execute_raw_vars`.
                    match retry_after_seconds(&e) {
                        Some(secs) => cooling.set(secs),
                        // Unreadable, and the overwhelmingly likely cause is the
                        // cooldown: the client only offers the board when its own
                        // countdown is done, so the disagreement is the server
                        // knowing about a placement this tab does not.
                        None => cooling.set(cooldown),
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
            // Sized by CSS, not by a fixed pixel count: the canvas element's own
            // width/height attributes are the CELL grid, so the browser keeps the
            // aspect ratio and a phone gets the whole board scaled to fit rather
            // than a corner of it.
            canvas {
                id: "{dom_id}",
                class: "pixel-board",
                width: "{cols}",
                height: "{rows_n}",
                style: "max-width: {board_px(cols)}px;",
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

/// The widest the board is allowed to draw, for a given number of cells.
///
/// A ceiling, not a size: the board is `width: 100%` and shrinks to whatever it
/// is given, so this only stops a small canvas from being blown up across a
/// desktop monitor. Sixteen pixels a cell keeps a 32-cell board a comfortable
/// 512px there, while the same board fills the width of a phone.
fn board_px(cells: u32) -> u32 {
    (cells * 16).clamp(240, 1024)
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
        // The SAME board on a phone, scaled by CSS to 320px: five pixels a cell,
        // and a tap still lands where the finger is. Mapping against the unscaled
        // width would have painted roughly twice as far across.
        assert_eq!(cell_at(0.0, 0.0, 320.0, 320.0, 64, 64), Some((0, 0)));
        assert_eq!(cell_at(4.9, 4.9, 320.0, 320.0, 64, 64), Some((0, 0)));
        assert_eq!(cell_at(5.0, 0.0, 320.0, 320.0, 64, 64), Some((1, 0)));
        assert_eq!(cell_at(160.0, 160.0, 320.0, 320.0, 64, 64), Some((32, 32)));
        assert_eq!(cell_at(319.9, 319.9, 320.0, 320.0, 64, 64), Some((63, 63)));
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
        assert_eq!(board_px(1), 240, "a tiny canvas is still worth looking at");
        assert_eq!(board_px(DEFAULT_SIDE), 512);
        assert_eq!(board_px(64), 1024);
        assert_eq!(board_px(1000), 1024, "and never wider than a screen");
    }
}

/// The canvases of a context, reached from the app rail (`?app=pixel`).
///
/// The context owner chooses which canvas the room is on, the way the chair
/// chooses what the projector shows, and everyone else simply gets that one. A
/// canvas is a hidden mime, so this is the way in; the list below it is the
/// owner's, for switching, adding and clearing away.
#[component]
pub fn PixelCanvasesApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_owner = node.is_context_owner.unwrap_or(false) || node.is_owner.unwrap_or(false);
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let canvases: Vec<_> = node
        .children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("pixel/canvas"))
        .cloned()
        .collect();

    // Which one the owner put the room on.
    let focus_ctx = context_id.clone();
    let focus_token = session.read().access_token.clone();
    let focused = crate::use_data_resource!(|(focus_ctx, focus_token)| async move {
        graphql::focused_canvas(focus_token.as_deref(), &focus_ctx).await
    });
    let focused_id = focused.read().clone().flatten();
    // Falling back to the only canvas there is: a context with one canvas and no
    // choice made should show it rather than an empty screen and a shrug.
    let showing = focused_id
        .clone()
        .filter(|id| canvases.iter().any(|c| &c.id.0 == id))
        .or_else(|| (canvases.len() == 1).then(|| canvases[0].id.0.clone()));

    // The board needs the canvas NODE (its geometry and open state), which the
    // context's child list does not carry.
    //
    // A resource keyed on `showing`, NOT an effect: an effect that reads no signal
    // runs once at mount, and at mount the focus has not resolved yet, so the
    // board never loaded and an owner saw an empty app with a list under it.
    let board_dep = showing.clone();
    let board_token = session.read().access_token.clone();
    let board = crate::use_data_resource!(|(board_dep, board_token)| async move {
        let id = board_dep?;
        graphql::query_node_by_id(board_token.as_deref(), &id)
            .await
            .ok()
            .flatten()
    });

    rsx! {
        div { class: "stack stack-v",
            if let Some(canvas) = board.read().clone().flatten() {
                PixelApp { key: "{canvas.id.0}", node: canvas }
            } else if !is_owner {
                div { class: "card",
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "grid_on" }
                        }
                        p { class: "empty-state-body", "{t(\"pixel.noCanvases\")}" }
                    }
                }
            }

            if is_owner {
                div { class: "stack stack-h stack-end",
                    AddCanvasButton { context_id: context_id.clone() }
                }
                if !canvases.is_empty() {
                    div { class: "list",
                        for canvas in canvases {
                            CanvasRow {
                                key: "{canvas.id.0}",
                                canvas_id: canvas.id.0.clone(),
                                name: canvas.name.clone(),
                                context_id: context_id.clone(),
                                is_showing: showing.as_deref() == Some(canvas.id.0.as_str()),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One canvas in the owner's list: show it to the room, or clear it away.
#[component]
fn CanvasRow(canvas_id: String, name: String, context_id: String, is_showing: bool) -> Element {
    let session = use_session();
    let mut confirm = use_signal(|| false);
    let mut busy = use_signal(|| false);

    let show_id = canvas_id.clone();
    let show_ctx = context_id.clone();
    let on_show = move |_| {
        let (id, ctx) = (show_id.clone(), show_ctx.clone());
        let token = session.read().access_token.clone();
        spawn(async move {
            if let Err(e) = graphql::set_focused_canvas(token.as_deref(), &ctx, Some(&id)).await {
                log::error!("focus canvas failed: {e}");
                return;
            }
            crate::session::bump_data_version();
        });
    };

    let del_id = canvas_id.clone();
    let on_delete = move |_| {
        if busy() {
            return;
        }
        busy.set(true);
        let id = del_id.clone();
        let token = session.read().access_token.clone();
        let actor = session.read().user.as_ref().map(|u| u.id.clone());
        spawn(async move {
            // The bin, not a hard delete: a canvas is recoverable like any other
            // content, and its cells go with it.
            match graphql::bin_node(token.as_deref(), &id, None, actor.as_deref()).await {
                Ok(_) => {
                    crate::session::bump_data_version();
                    busy.set(false);
                    confirm.set(false);
                }
                Err(e) => {
                    busy.set(false);
                    log::error!("delete canvas failed: {e}");
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        div { class: "list-item",
            span { class: "material-icons", "grid_on" }
            span { class: "list-item-title", "{name}" }
            if is_showing {
                span { class: "chip", "{t(\"pixel.showing\")}" }
            } else {
                button {
                    class: "btn btn-text",
                    r#type: "button",
                    onclick: on_show,
                    "{t(\"pixel.show\")}"
                }
            }
            button {
                class: "btn-icon",
                r#type: "button",
                aria_label: "{t(\"common.delete\")}",
                onclick: move |_| confirm.set(true),
                span { class: "material-icons", "delete" }
            }
            crate::components::widgets::Dialog {
                open: confirm(),
                on_dismiss: move |_| confirm.set(false),
                headline: t("pixel.deleteCanvas"),
                icon: "delete".to_string(),
                actions: rsx! {
                    button {
                        class: "btn btn-outlined",
                        onclick: move |_| confirm.set(false),
                        "{t(\"common.cancel\")}"
                    }
                    button {
                        class: "btn btn-primary",
                        disabled: busy(),
                        onclick: on_delete,
                        "{t(\"common.delete\")}"
                    }
                },
                p { "{t(\"pixel.deleteExplain\")}" }
            }
        }
    }
}

/// Owner-only: create a canvas in this context, the way a speaker list is made.
#[component]
fn AddCanvasButton(context_id: String) -> Element {
    let session = use_session();
    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let submit = move |_| {
        let title = name.read().trim().to_string();
        if title.is_empty() || *busy.read() {
            return;
        }
        let ctx = context_id.clone();
        let token = session.read().access_token.clone();
        busy.set(true);
        spawn(async move {
            match graphql::create_canvas(token.as_deref(), &ctx, &title, DEFAULT_SIDE, DEFAULT_SIDE, 60).await {
                Ok(_) => {
                    crate::session::bump_data_version();
                    busy.set(false);
                    open.set(false);
                    name.set(String::new());
                }
                Err(e) => {
                    busy.set(false);
                    log::error!("create canvas failed: {e}");
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        button {
            class: "btn btn-tonal",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "add" }
            " {t(\"pixel.newCanvas\")}"
        }
        crate::components::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("pixel.newCanvas"),
            form: true,
            icon: "grid_on".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: busy(),
                    onclick: submit,
                    "{t(\"common.create\")}"
                }
            },
            div { class: "field",
                input {
                    class: "input",
                    value: "{name}",
                    placeholder: t("pixel.canvasName"),
                    oninput: move |e| name.set(e.value()),
                }
            }
        }
    }
}
