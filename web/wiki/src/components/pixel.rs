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
use crate::route::Route;
use crate::session::use_session;

/// How many cells a new canvas is across and down, unless the person making it
/// says otherwise.
///
/// Sixty-four. It was thirty-two, on the argument that a board scales to the
/// width of a phone and a 64-cell board gives a cell about five pixels on a
/// 360px screen, which is not much to aim at. That argument is still true and
/// is answered rather than ignored: the board can be pinched and zoomed, and a
/// tap that lands on the wrong cell costs one turn of a ten-second cooldown.
/// Four times the cells is a picture a hall can actually make something of, and
/// the size is a field on the dialog now, so a room that would rather have big
/// squares can still ask for them.
pub const DEFAULT_SIDE: u32 = 64;

/// How long a painter waits between placements.
///
/// Ten seconds. Short enough that a board is worth standing in front of, and
/// still long enough that it takes a room rather than a person: a whole session
/// of one determined painter is a few hundred cells of four thousand, and
/// anybody else can paint over them at the same rate.
///
/// Written onto the context's `canvas/pixel` permission when a canvas is made,
/// which is where the trigger reads it (`migrations/0007`), and onto the canvas
/// itself, which is where the countdown reads it.
pub const DEFAULT_COOLDOWN: u32 = 10;

/// How far the board may be zoomed in.
///
/// Four steps, because four is what the smallest screen needs: a 64-cell board
/// on a 360px phone gives a cell about five pixels, and four times that is
/// twenty-two -- comfortably bigger than the 44px target a finger wants, once
/// the cell is the thing being aimed at rather than the whole board.
pub const MAX_ZOOM: u32 = 4;

/// The palette, as CSS colours. The stored value is the INDEX, so a cell costs a
/// single digit in the database and the palette can be restyled later without
/// rewriting every row.
///
/// The last two are the party's own, the seeds this app's whole colour scheme is
/// generated from (`scripts/gen-theme.ts`): Radikale grøn and Radikale magenta.
/// A board painted by this organisation should be able to spell its own name in
/// its own colours, and neither is reachable by mixing the sixteen above.
///
/// The eight after those close the holes the first sixteen leave, chosen by
/// measuring them: every colour a room here would reach for — the flags, faces,
/// the ordinary business of drawing — was scored against the palette in CIE Lab,
/// and these are the additions that shrink the average miss most. It falls from
/// 27 to 12, which is the difference between "near enough" and "that is the
/// colour". What they answer, in order:
///
/// * **Pale warm tones.** Light skin resolved to light GREY, and blond hair to
///   yellow. A face could not be drawn at all.
/// * **A flag blue.** The palette leapt from azure straight to `#0000ea` with
///   nothing between, so EU blue landed on PURPLE and the Nordic blues on
///   near-black. This one had already bitten: the EU flag on the assembly's
///   board is vivid blue because that band did not exist.
/// * **Shades.** `#222222` was the only dark colour, so a dark red or a teal
///   collapsed onto black or onto a brighter version of itself, and nothing
///   could be shaded in its own hue.
///
/// **Appended, never inserted.** The index IS the stored value, so putting a
/// colour anywhere but the end would repaint every cell already placed. That is
/// why these sit apart from their relatives on the swatch row rather than beside
/// them; a tidier order would cost every board its picture.
pub const PALETTE: &[&str] = &[
    "#ffffff", "#e4e4e4", "#888888", "#222222", "#ffa7d1", "#e50000", "#e59500", "#a06a42",
    "#e5d900", "#94e044", "#02be01", "#00d3dd", "#0083c7", "#0000ea", "#cf6ee4", "#820080",
    "#02944f", "#d2307e", "#ffdbac", "#003399", "#be0039", "#008080", "#004dff", "#fff8b8",
    "#7ec8e3", "#e0ac69",
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

/// How long a finger must rest on a cell before the board says who painted it.
///
/// There is no hover on a touch screen, and the tap is already spoken for: it
/// paints, and painting costs a turn. So the question "who did this?" is asked
/// by holding still, which is not something anyone does by accident and which
/// cannot be confused with aiming. Long enough not to fire while a finger is
/// settling, short enough to feel like an answer rather than a wait.
const HOLD_TO_ASK_MS: u32 = 450;

/// What the board should say about the cell under the pointer, as a translation
/// key and the name to put in it.
///
/// Three states, and they read differently: nothing is pointed at, the cell is
/// blank, or somebody painted it. An unattributed cell — painted by someone this
/// reader may not see — is the last of these WITHOUT a name (`None`), rather
/// than being reported as blank.
///
/// Pure, so the decision is testable: `t` reads a global signal and needs a
/// running renderer, which a unit test has not got.
pub fn painter_says(
    cell: Option<(u32, u32)>,
    owner: Option<&str>,
    name: Option<&str>,
    painted: bool,
) -> Option<(&'static str, Option<String>)> {
    cell?;
    if !painted {
        return Some(("pixel.cellEmpty", None));
    }
    let named = match (owner, name) {
        (Some(_), Some(name)) if !name.trim().is_empty() => Some(name.to_string()),
        _ => None,
    };
    Some(("pixel.painterIs", named))
}

/// Where to put the tooltip for a cell, as an inline style.
///
/// Everything is a PERCENTAGE of the board, so the tip lands on its cell at any
/// width — a phone's scaled board, a desktop's full one, a pinch zoom — without
/// measuring anything in JavaScript. Two decisions beyond that:
///
/// * It sits ABOVE the cell, except in the top fifth of the board, where there
///   is nothing above to sit in and it flips below.
/// * Near an edge it stops centring and aligns its own edge instead, so a tip on
///   the leftmost column does not hang off the side of the board. Three
///   positions rather than a clamp, because the tip's width is not known here
///   and a translate needs no measurement.
pub fn tip_style(cell: (u32, u32), cols: u32, rows: u32) -> String {
    let (x, y) = cell;
    let (cols, rows) = (cols.max(1) as f64, rows.max(1) as f64);
    let cx = (x as f64 + 0.5) / cols;
    let below = (y as f64 / rows) < 0.2;
    // The edge the tip aligns to, and the vertical anchor it grows from.
    let tx = if cx < 0.2 {
        "0"
    } else if cx > 0.8 {
        "-100%"
    } else {
        "-50%"
    };
    let (top, ty) = if below {
        // Under the cell, growing downward from its bottom edge.
        (((y + 1) as f64 / rows) * 100.0, "var(--md-sys-spacing-2)")
    } else {
        // Above the cell, growing upward from its top edge.
        (
            (y as f64 / rows) * 100.0,
            "calc(-100% - var(--md-sys-spacing-2))",
        )
    };
    format!(
        "left: {:.3}%; top: {:.3}%; transform: translate({tx}, {ty});",
        cx * 100.0,
        top
    )
}

/// [`painter_says`], worded.
pub fn painter_label(
    cell: Option<(u32, u32)>,
    owner: Option<&str>,
    name: Option<&str>,
    painted: bool,
) -> Option<String> {
    let (x, y) = cell?;
    let (key, named) = painter_says(cell, owner, name, painted)?;
    let who = named.unwrap_or_else(|| t("pixel.painterUnknown"));
    Some(
        t(key)
            .replace("{name}", &who)
            .replace("{x}", &x.to_string())
            .replace("{y}", &y.to_string()),
    )
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
    // Who painted each cell, as user ids, and when. Separate from `cells` so the
    // drawing path stays a map of colours: the board is redrawn from it on every
    // undo.
    let mut owners = use_signal(HashMap::<(u32, u32), String>::new);
    let mut painted_at = use_signal(HashMap::<(u32, u32), String>::new);
    let mut colour = use_signal(|| 3u8);
    // What the reader is TOLD, and the moment it is actually over. Two signals
    // because they answer different questions: the first is a number on a
    // button, the second is when the database will take a placement. Counting
    // down by decrementing a number was what let the two disagree.
    let mut cooling = use_signal(|| 0u32);
    let mut ready_at = use_signal(|| 0.0f64);
    let mut busy = use_signal(|| false);
    // The cell being asked about: hovered with a mouse, held under a finger.
    let mut asking = use_signal(|| None::<(u32, u32)>);
    // A hold that has already answered, so the tap that ends it does not also
    // paint. Asking who painted a cell must never cost a turn.
    let mut held = use_signal(|| false);
    // Whether a finger is down right now, which is what the hold timer checks.
    let mut pressing = use_signal(|| false);
    // Whether the pointer is inside the tooltip. It holds a link, so leaving the
    // board towards it must not close the thing being reached for.
    let mut over_tip = use_signal(|| false);

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
                let mut who = owners.write();
                let mut when = painted_at.write();
                for cell in rows {
                    map.insert(cell.at, cell.colour);
                    if let Some(owner) = cell.owner {
                        who.insert(cell.at, owner);
                    }
                    if let Some(at) = cell.when {
                        when.insert(cell.at, at);
                    }
                }
            }
            // `peek`, NOT `read`: reading a signal inside an effect subscribes
            // the effect to it, and this effect writes `cells`. Reading it here
            // made every placement re-run the effect, which re-applied the rows
            // from the ORIGINAL load — so painting over an existing cell put the
            // old colour straight back on top of the new one. A cell that was
            // not in the load had nothing to be overwritten by, which is why
            // this only ever happened on top of somebody else's pixel.
            draw_all_of(&draw_id, cols, rows_n, &cells.peek());
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
        let deadline = then + cooldown as f64 * 1000.0;
        if deadline > js_sys::Date::now() {
            ready_at.set(deadline);
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
    let stream = crate::subscription::use_graphql_subscription(graphql::cell_stream(
        graphql::children_of_mime(&canvas_id, "canvas/pixel"),
        &since,
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
        let mut who = owners.write();
        let mut when = painted_at.write();
        for row in rows.iter() {
            if let Some(cell) = graphql::parse_cell_full(row) {
                let ((x, y), c) = (cell.at, cell.colour);
                map.insert((x, y), c);
                match cell.when {
                    Some(at) => {
                        when.insert((x, y), at);
                    }
                    None => {
                        when.remove(&(x, y));
                    }
                }
                match cell.owner {
                    Some(owner) => {
                        who.insert((x, y), owner);
                    }
                    // Repainted by someone this reader cannot see: drop the old
                    // attribution rather than leaving the previous painter's
                    // name on a cell that is no longer theirs.
                    None => {
                        who.remove(&(x, y));
                    }
                }
                // One cell, not a re-render: this is what makes a busy canvas
                // cost the same as a quiet one.
                if let Some(ctx) = ctx.as_ref() {
                    draw_cell(ctx, x, y, c);
                }
            }
        }
    });

    // The number on the button, read off the CLOCK rather than counted down.
    //
    // Decrementing once a second was wrong twice over. The first tick fired at
    // whatever point in the second the wait had begun, so it was worth anything
    // from nothing to a full second; and the number was truncated, so a wait of
    // 3.9 seconds displayed as 3. Between them the button offered the board up
    // to two seconds before the database would take a placement, and a painter
    // who tapped the moment it read zero was refused.
    //
    // Four times a second, so zero arrives promptly once the deadline passes.
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(250).await;
            let left = seconds_left(ready_at(), js_sys::Date::now());
            if cooling() != left {
                cooling.set(left);
            }
        }
    });

    // The people on the board, resolved ONCE for the whole canvas rather than
    // per cell: a thousand cells are a handful of painters, and the rows carry
    // ids. Re-runs only when a painter appears who was not there before, so a
    // busy board does not re-ask on every placement.
    let name_token = session.read().access_token.clone();
    let painter_ids = {
        let mut ids: Vec<String> = owners.read().values().cloned().collect();
        ids.sort();
        ids.dedup();
        ids.join(",")
    };
    let people_res = crate::use_data_resource!(|(name_token, painter_ids)| async move {
        let ids: Vec<String> = painter_ids
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return HashMap::<String, crate::model::Author>::new();
        }
        graphql::query_users_by_ids(name_token.as_deref(), &ids)
            .await
            .into_iter()
            .filter_map(|a| Some((a.user_id.clone()?, a)))
            .collect()
    });

    // Locking is the owner's, and it takes effect for everyone: the board reads
    // `mutable`, and so does the trigger that refuses a placement, so a closed
    // canvas is closed to anything that is not this board as well.
    // How far in the board is drawn, in whole steps.
    //
    // A 64-cell board on a 360px phone gives a cell about five pixels, which is
    // not something a finger can aim at. Pinching works -- nothing disables it --
    // but it zooms the PAGE, so the palette leaves the screen and the thing you
    // came to press goes with it. This zooms the board inside its own scroller
    // and leaves the rest of the app where it was.
    //
    // Nothing else has to change for it: a click is mapped through the board's
    // own bounding rect, so a board drawn three times wider maps a tap to the
    // same cell without any arithmetic knowing about zoom.
    let mut zoom = use_signal(|| 1u32);
    // Whether the "pick a colour, then tap" hint is still needed. It says what
    // to do, which is worth a line of the screen exactly once: after the first
    // tap the reader has demonstrably worked it out, and the line was costing
    // the board height on every visit afterwards.
    let mut hint = use_signal(|| true);
    let mut locking = use_signal(|| false);
    let lock_id = canvas_id.clone();
    let toggle_lock = move |_| {
        if locking() {
            return;
        }
        locking.set(true);
        let (id, want) = (lock_id.clone(), !open);
        let token = session.read().access_token.clone();
        spawn(async move {
            match graphql::set_canvas_open(token.as_deref(), &id, want).await {
                Ok(()) => crate::session::bump_data_version(),
                Err(e) => {
                    crate::errors::log_handled("lock canvas failed", e);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
            locking.set(false);
        });
    };

    let can_paint = session.read().is_authenticated() && open && !projector;
    let paint_canvas = canvas_id.clone();
    let click_id = dom_id.clone();
    let undo_id = dom_id.clone();
    let paint_ctx = context_id.clone();

    // Which cell a pointer event is over, in board coordinates. The element's
    // on-screen size is whatever CSS gave it, so ask the element (as the click
    // path does) rather than assuming the drawn size.
    let hit_id = dom_id.clone();
    let cell_under = move |evt: &Event<PointerData>| -> Option<(u32, u32)> {
        let coords = evt.data().element_coordinates();
        let (box_w, box_h) = match board_size(&hit_id) {
            Some(size) => size,
            None => (board_px(cols) as f64, board_px(rows_n) as f64),
        };
        cell_at(coords.x, coords.y, box_w, box_h, cols, rows_n)
    };

    // A mouse answers by hovering. A touch pointer does not: its "move" events
    // only happen while it is down, which is the drag of a hold, so following
    // them would move the answer around under the finger.
    let on_move = {
        let cell_under = cell_under.clone();
        move |evt: Event<PointerData>| {
            if evt.data().pointer_type() == "touch" {
                return;
            }
            // Only a real cell moves the answer. A move that maps to nothing —
            // the rounding at the very edge of the element — must not take the
            // tooltip away while the pointer is still on the board.
            let Some(hit) = cell_under(&evt) else {
                return;
            };
            if *asking.peek() != Some(hit) {
                asking.set(Some(hit));
            }
        }
    };

    let on_down = {
        let cell_under = cell_under.clone();
        move |evt: Event<PointerData>| {
            if evt.data().pointer_type() != "touch" {
                return;
            }
            let Some(hit) = cell_under(&evt) else {
                return;
            };
            held.set(false);
            pressing.set(true);
            spawn(async move {
                gloo_timers::future::TimeoutFuture::new(HOLD_TO_ASK_MS).await;
                // Still down? A finger that lifted first cleared this, and its
                // tap paints as usual — the answer is only for one that stayed.
                if *pressing.peek() {
                    held.set(true);
                    asking.set(Some(hit));
                }
            });
        }
    };

    // A lift ends the press. If the hold never fired, the tap that follows
    // paints; if it did, `held` stays set for the click handler to consume, and
    // the answer stays on screen until the next thing happens.
    let on_up = move |_evt: Event<PointerData>| pressing.set(false);

    let on_leave = move |_evt: Event<PointerData>| {
        pressing.set(false);
        held.set(false);
        // A short grace period: the pointer leaving the board is usually the end
        // of looking, but it is also how it reaches the tooltip's profile link,
        // which sits just outside the cell it describes.
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(120).await;
            if !*over_tip.peek() {
                asking.set(None);
            }
        });
    };

    let on_click = move |evt: Event<MouseData>| {
        // The tap that ended a hold was a question, not a placement.
        if *held.peek() {
            held.set(false);
            return;
        }
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
        hint.set(false);
        // What is under the finger already, kept so a refusal can put it back.
        let was = cells.read().get(&(x, y)).copied();
        // Optimistic: the cell is painted now and corrected by the stream if the
        // server disagrees, so a hall on slow wifi still feels immediate.
        cells.write().insert((x, y), c);
        if let Some(ctx) = board_context(&click_id) {
            draw_cell(&ctx, x, y, c);
        }
        busy.set(true);
        let token = session.read().access_token.clone();
        // Who is painting, so a repaint takes the cell over rather than leaving
        // the previous painter's name on somebody else's colour.
        let me = session
            .read()
            .user
            .as_ref()
            .map(|u| u.id.clone())
            .unwrap_or_default();
        let (cv, ctx) = (paint_canvas.clone(), paint_ctx.clone());
        let undo = undo_id.clone();
        spawn(async move {
            let result = graphql::paint_cell(token.as_deref(), &cv, &ctx, &me, x, y, c).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    log::info!("painted {x},{y}");
                    // From NOW, which is after the database stamped the row, so
                    // this device waits a little longer than the trigger does
                    // rather than a little less.
                    ready_at.set(js_sys::Date::now() + cooldown as f64 * 1000.0);
                }
                Err(e) => {
                    // The overwhelmingly common refusal is the cooldown, and the
                    // database does not get to explain itself: Hasura answers
                    // "database query error" and hides the reason outside dev
                    // mode. So a refused placement is undone rather than reported
                    // as a fault, and the countdown starts. It is logged, not
                    // filed, because "you were too quick" is not a bug.
                    log::info!("paint {x},{y} refused: {e}");
                    // Take the optimistic cell back, to whatever was under it.
                    undo_placement(&mut cells.write(), (x, y), was);
                    draw_all_of(&undo, cols, rows_n, &cells.read());
                    // A refusal that knows when is not a failure to report; it is
                    // an instruction. Anything else is a real error and is already
                    // logged and filed by `execute_raw_vars`.
                    let now = js_sys::Date::now();
                    let deadline = match retry_after_seconds(&e) {
                        Some(secs) => now + secs as f64 * 1000.0,
                        // Unreadable, so ask the server what it knows instead of
                        // charging a whole cooldown again. A placement refused by
                        // a fraction of a second used to cost the full wait a
                        // second time, which is the same near-miss punished
                        // twice. `max` keeps it moving if this device's clock
                        // disagrees with the database's, so a refusal can never
                        // loop.
                        None => match graphql::my_last_paint(token.as_deref(), &cv, &me).await {
                            Some(iso) => {
                                let then = js_sys::Date::new(
                                    &wasm_bindgen::JsValue::from_str(&iso),
                                )
                                .get_time();
                                (then + cooldown as f64 * 1000.0).max(now + 1000.0)
                            }
                            None => now + cooldown as f64 * 1000.0,
                        },
                    };
                    ready_at.set(deadline);
                }
            }
        });
    };

    let cells_now = cells.read().clone();
    let painted = cells_now.len();

    // What the tooltip says about the cell being pointed at right now.
    let at = asking();
    let owner_id = at.and_then(|c| owners.read().get(&c).cloned());
    let author = owner_id
        .as_deref()
        .and_then(|id| people_res.read().as_ref()?.get(id).cloned());
    let when = at.and_then(|c| painted_at.read().get(&c).cloned());
    let is_painted = at.map(|c| cells_now.contains_key(&c)).unwrap_or(false);
    // The sentence, for a cell with nobody to link to: blank, or painted by
    // somebody this reader cannot see.
    let plain = (author.is_none())
        .then(|| {
            painter_label(
                at,
                owner_id.as_deref(),
                author.as_ref().map(|a| a.name.as_str()),
                is_painted,
            )
        })
        .flatten();

    rsx! {
        // A card with the header every other app uses: the avatar and glyph of
        // what this is, its name, and what kind of thing it is underneath. This
        // was a bare heading of its own invention, which read as a different
        // product from the speaker list sitting next to it in the rail.
        div { class: "card pixel-app",
            div { class: "card-header",
                div {
                    class: if open { "avatar secondary" } else { "avatar" },
                    span { class: "material-icons", "grid_on" }
                }
                div {
                    h3 { class: "title-medium", "{node.name}" }
                    p { class: "body-medium text-muted", "{t(\"mime.canvas\")}" }
                }
                div { class: "flex-grow" }
                span { class: "body-small text-muted", "{painted} / {cols * rows_n}" }
                // How far in. Offered to everyone, painter or not: a board is worth
                // looking at closely as well as painting on, and a projector at the
                // back of a hall wants it too.
                div { class: "pixel-zoom",
                    button {
                        class: "btn-icon",
                        r#type: "button",
                        disabled: zoom() <= 1,
                        aria_label: "{t(\"pixel.zoomOut\")}",
                        title: "{t(\"pixel.zoomOut\")}",
                        onclick: move |_| zoom.set((zoom() - 1).max(1)),
                        span { class: "material-icons", "zoom_out" }
                    }
                    span { class: "body-small text-muted", "{zoom()}\u{00d7}" }
                    button {
                        class: "btn-icon",
                        r#type: "button",
                        disabled: zoom() >= MAX_ZOOM,
                        aria_label: "{t(\"pixel.zoomIn\")}",
                        title: "{t(\"pixel.zoomIn\")}",
                        onclick: move |_| zoom.set((zoom() + 1).min(MAX_ZOOM)),
                        span { class: "material-icons", "zoom_in" }
                    }
                }
                if !open {
                    span { class: "chip", "{t(\"pixel.closed\")}" }
                }
                // The lock, where the board is: this is the canvas a room is
                // looking at, so it is the one an owner means by "close it now".
                if node.is_context_owner.unwrap_or(false) {
                    button {
                        class: "btn-icon",
                        r#type: "button",
                        disabled: locking(),
                        aria_label: if open { t("pixel.lock") } else { t("pixel.unlock") },
                        title: if open { t("pixel.lock") } else { t("pixel.unlock") },
                        onclick: toggle_lock,
                        span { class: "material-icons",
                            if open { "lock_open" } else { "lock" }
                        }
                    }
                }
            }
            div { class: "card-content",

            // The board, with the tooltip positioned inside it. The wrapper is
            // the coordinate system: the tip is placed as a PERCENTAGE of the
            // board, so it lands on the right cell at any width — a phone's
            // scaled-down board, a desktop's full one, or a pinch zoom — with
            // nothing measured in JavaScript.
            div {
                class: "pixel-board-scroll",
                // The zoom for the stylesheet to multiply by, and the board's
                // own shape so the window is cut to it rather than to a square.
                style: "--zoom: {zoom()}; --board-aspect: {cols} / {rows_n};",
            div {
                class: "pixel-board-wrap",
                // The wrapper, not the canvas, carries the zoom: the tooltip is
                // placed as a percentage of THIS box, so growing the box keeps
                // the tip on its cell.

                // Leaving is the WRAPPER's business, not the canvas's. The tip
                // hangs a small gap away from its cell, and that gap belongs to
                // neither element: crossing it to reach the profile link fired
                // the canvas's leave and took the tooltip away mid-reach. The
                // gap is inside the wrapper, so travelling across it never
                // leaves anything.
                onpointerleave: on_leave,
                onpointercancel: on_leave,
                // Sized by CSS, not by a fixed pixel count: the canvas element's
                // own width/height attributes are the CELL grid, so the browser
                // keeps the aspect ratio and a phone gets the whole board scaled
                // to fit rather than a corner of it.
                canvas {
                    id: "{dom_id}",
                    class: "pixel-board",
                    width: "{cols}",
                    height: "{rows_n}",
                    onclick: on_click,
                    onpointermove: on_move,
                    onpointerdown: on_down,
                    // A finger lifting ends the press wherever it happens; the
                    // wrapper above handles leaving.
                    onpointerup: on_up,
                }
                if hint() && can_paint && cooling() == 0 {
                    div { class: "pixel-hint", aria_hidden: "true",
                        span { "{t(\"pixel.yourTurn\")}" }
                    }
                }
                if let Some(cell) = at {
                    div {
                        class: "pixel-tip",
                        style: "{tip_style(cell, cols, rows_n)}",
                        role: "tooltip",
                        aria_live: "polite",
                        // The pointer may travel INTO the tip to reach the
                        // profile link. The tip only records that it is under
                        // the pointer — closing stays the wrapper's decision, so
                        // stepping off the tip back towards the board does not
                        // shut the thing you are still using.
                        onpointerenter: move |_| over_tip.set(true),
                        onpointerleave: move |_| over_tip.set(false),
                        if let Some(author) = author.clone() {
                            // A person to go and look at, so the whole line is a
                            // link: avatar, name, and the cell it is about.
                            Link {
                                class: "pixel-tip-who",
                                to: Route::UserProfile {
                                    id: author.user_id.clone().unwrap_or_default(),
                                },
                                span { class: "avatar small",
                                    {crate::components::loader::user_avatar(
                                        &author.avatar_url,
                                        rsx! { span { class: "material-icons", "person" } },
                                    )}
                                }
                                span { class: "pixel-tip-name", "{author.name}" }
                            }
                            if let Some(when) = when.clone() {
                                span { class: "pixel-tip-when",
                                    {crate::components::loader::relative_time(&when)}
                                }
                            }
                        } else if let Some(text) = plain.clone() {
                            span { class: "pixel-tip-plain", "{text}" }
                        }
                    }
                }
            }
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
                // Only the wait keeps a line of its own: it is a number that
                // changes and a reason the board is not answering.
                if cooling() > 0 {
                    div { class: "pixel-status",
                        span { class: "pixel-wait", "{t(\"pixel.waitSeconds\")} {cooling()}" }
                    }
                }
            }
            }
        }
    }
}

/// How many whole seconds are still to wait, from a deadline.
///
/// Rounded UP. Truncating is what made the board offer itself early: three and
/// nine tenths of a second left displayed as three, ran out three seconds later,
/// and the placement the painter made on zero was refused by a database that
/// still had nine tenths of a second to go.
pub fn seconds_left(ready_at_ms: f64, now_ms: f64) -> u32 {
    let left = (ready_at_ms - now_ms) / 1000.0;
    match left > 0.0 {
        true => left.ceil() as u32,
        false => 0,
    }
}

/// Put a refused placement back the way it was.
///
/// REMOVING the cell is not undoing it. The board is redrawn by filling with
/// `PALETTE[0]` and painting the cells the map holds, so a cell taken out of the
/// map comes back WHITE rather than the colour it was before the tap — which is
/// what a painter saw when the cooldown refused them: the cell they aimed at
/// turned white and stayed white until the next load put the real board back.
///
/// A cell that was never painted still goes, since white IS its state.
pub fn undo_placement(cells: &mut HashMap<(u32, u32), u8>, at: (u32, u32), was: Option<u8>) {
    match was {
        Some(before) => {
            cells.insert(at, before);
        }
        None => {
            cells.remove(&at);
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

    /// Every colour the palette offers is a colour, and no two are the same one.
    ///
    /// The index is the stored value, so a duplicate would waste a swatch and a
    /// malformed entry would paint nothing. Cheap to check, and it catches a
    /// fat-fingered hex in the one place a typo is invisible.
    #[test]
    fn the_palette_is_well_formed() {
        for c in PALETTE {
            assert!(c.len() == 7 && c.starts_with('#'), "{c} is not a hex colour");
            assert!(u32::from_str_radix(&c[1..], 16).is_ok(), "{c} is not hex");
        }
        let mut seen: Vec<&str> = PALETTE.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "the palette repeats a colour");
        // An index is stored in a u8, and `hex_of` falls back rather than
        // panicking above it -- but a palette that outgrew a byte would silently
        // lose its tail.
        assert!(PALETTE.len() <= 256);
    }

    /// The wait is rounded UP, so the button never reads zero while the database
    /// would still refuse.
    ///
    /// This is the fix for "if I click right after time ends, the pixel will not
    /// apply": the old countdown truncated, so a wait with nine tenths of a
    /// second left showed as a whole second fewer and ran out early.
    #[test]
    fn the_wait_is_never_shorter_than_it_is() {
        // 3.9 seconds left is four seconds to wait, not three.
        assert_eq!(seconds_left(13_900.0, 10_000.0), 4);
        // A hair before the deadline still says one, never zero.
        assert_eq!(seconds_left(10_001.0, 10_000.0), 1);
        // Zero only once it has actually passed.
        assert_eq!(seconds_left(10_000.0, 10_000.0), 0);
        assert_eq!(seconds_left(9_000.0, 10_000.0), 0, "and stays there");
        // A whole cooldown reads as the whole cooldown.
        assert_eq!(seconds_left(20_000.0, 10_000.0), 10);
    }

    /// A refused placement puts back what was under it, rather than clearing it.
    ///
    /// The board is redrawn by filling white and painting what the map holds, so
    /// dropping the cell paints it WHITE — which is what a painter reported:
    /// "the pixel was set to white, not blue as I had selected", and the real
    /// colour only came back on the next load.
    #[test]
    fn a_refused_placement_restores_the_cell_under_it() {
        let mut cells = HashMap::new();
        cells.insert((3, 4), 13u8); // the blue of the flag under the finger
        cells.insert((3, 4), 8u8); // the optimistic placement
        undo_placement(&mut cells, (3, 4), Some(13));
        assert_eq!(cells.get(&(3, 4)), Some(&13), "the cell is blue again, not gone");

        // A cell nobody had painted goes, because white IS its state.
        let mut empty = HashMap::new();
        empty.insert((9, 9), 5u8);
        undo_placement(&mut empty, (9, 9), None);
        assert!(!empty.contains_key(&(9, 9)));
    }

    /// The board never asks the browser for a size it cannot draw.
    #[test]
    fn the_board_stays_a_sane_size() {
        assert_eq!(board_px(1), 240, "a tiny canvas is still worth looking at");
        assert_eq!(board_px(32), 512);
        assert_eq!(board_px(DEFAULT_SIDE), 1024, "the default fills its ceiling");
        assert_eq!(board_px(1000), 1024, "and never wider than a screen");
    }

    /// Pointing at nothing says nothing: the line under the board is empty
    /// until a cell is actually being asked about.
    #[test]
    fn nothing_pointed_at_says_nothing() {
        assert_eq!(painter_says(None, None, None, false), None);
        assert_eq!(painter_says(None, Some("u1"), Some("Anna"), true), None);
    }

    /// A cell nobody has painted says so, rather than naming nobody.
    #[test]
    fn an_unpainted_cell_says_it_is_empty() {
        assert_eq!(
            painter_says(Some((3, 4)), None, None, false),
            Some(("pixel.cellEmpty", None))
        );
        // Even if an owner id somehow rides along, blank is blank.
        assert_eq!(
            painter_says(Some((3, 4)), Some("u1"), Some("Anna"), false),
            Some(("pixel.cellEmpty", None))
        );
    }

    /// The name, when it is known.
    #[test]
    fn a_painted_cell_names_its_painter() {
        assert_eq!(
            painter_says(Some((12, 7)), Some("u1"), Some("Anna Hansen"), true),
            Some(("pixel.painterIs", Some("Anna Hansen".to_string())))
        );
    }

    /// A cell painted by somebody this reader may not see is still painted: the
    /// same sentence, with no name in it. It must not read as blank, and a name
    /// that resolved to nothing must not be shown as a name.
    #[test]
    fn an_unattributed_cell_is_still_painted() {
        assert_eq!(
            painter_says(Some((1, 1)), None, None, true),
            Some(("pixel.painterIs", None))
        );
        for empty in ["", "   "] {
            assert_eq!(
                painter_says(Some((1, 1)), Some("u1"), Some(empty), true),
                Some(("pixel.painterIs", None)),
                "{empty:?} is not a name"
            );
        }
    }

    /// The tip is placed in percentages of the board, so it lands on its cell
    /// whatever the board is scaled to.
    #[test]
    fn the_tip_lands_on_its_cell() {
        // Middle of a 10-wide board: the 5th column's centre is 55%.
        assert!(
            tip_style((5, 5), 10, 10).contains("left: 55.000%"),
            "{}",
            tip_style((5, 5), 10, 10)
        );
        // And it hangs above the cell it describes.
        assert!(tip_style((5, 5), 10, 10).contains("top: 50.000%"));
    }

    /// A tip on the top row has nothing above it to sit in, so it flips below.
    #[test]
    fn a_tip_at_the_top_flips_below() {
        let top = tip_style((5, 0), 10, 10);
        assert!(
            top.contains("top: 10.000%"),
            "anchored to the cell's BOTTOM: {top}"
        );
        assert!(!top.contains("-100%"), "and grows downward: {top}");
        let lower = tip_style((5, 9), 10, 10);
        assert!(
            lower.contains("calc(-100%"),
            "elsewhere it grows upward: {lower}"
        );
    }

    /// Near a side it aligns its own edge rather than centring, so it cannot
    /// hang off the board.
    #[test]
    fn a_tip_at_the_edge_aligns_inward() {
        assert!(
            tip_style((0, 5), 10, 10).contains("translate(0,"),
            "left edge"
        );
        assert!(
            tip_style((9, 5), 10, 10).contains("translate(-100%,"),
            "right edge"
        );
        assert!(
            tip_style((5, 5), 10, 10).contains("translate(-50%,"),
            "middle stays centred"
        );
    }

    /// A degenerate board must not divide by zero.
    #[test]
    fn a_board_with_no_cells_does_not_divide_by_zero() {
        let style = tip_style((0, 0), 0, 0);
        assert!(style.contains('%'), "{style}");
        assert!(!style.contains("NaN") && !style.contains("inf"), "{style}");
    }
}

/// The canvases of a context, reached from the app rail (`?app=canvas`).
///
/// The context owner chooses which canvas the room is on, the way the chair
/// chooses what the projector shows, and everyone else simply gets that one. A
/// canvas is a hidden mime, so this is the way in; the list below it is the
/// owner's, for switching, adding and clearing away.
#[component]
pub fn PixelCanvasesApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    // Context owners ONLY. `is_owner` is a different question — it means "you
    // created THIS node" — and it was being accepted here as though it meant
    // "you administer this place", so whoever happened to own the node the app
    // hangs off could add a canvas to somebody else's group.
    let is_owner = node.is_context_owner.unwrap_or(false);
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let canvases: Vec<_> = node
        .children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("canvas/canvas"))
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

    // Whether the app is still working out what to show, as opposed to having
    // worked out that there is nothing. Two questions are outstanding at mount
    // and both are answered over the network: which canvas this room is on, and
    // then that canvas itself. A resource reads `None` until it answers, and
    // `None` was being taken for "no canvas" -- so opening the app said "No
    // canvases here yet" for as long as the round trip took, to a room that has
    // one.
    let deciding = focused.read().is_none();
    let fetching_board = showing.is_some() && board.read().is_none();
    let still_looking = deciding || fetching_board;

    rsx! {
        div { class: "stack stack-v",
            if let Some(canvas) = board.read().clone().flatten() {
                PixelApp { key: "{canvas.id.0}", node: canvas }
            } else if still_looking {
                div { class: "card",
                    div { class: "empty-state empty-state-sm",
                        div { class: "spinner spinner-sm" }
                    }
                }
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

            // The owner's panel, framed like every other app's: a card with a
            // header saying what it is. It used to be a loose button above a
            // loose list, floating on the page background with nothing to say
            // that the two belonged together or to whom.
            if is_owner {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar small",
                            span { class: "material-icons", "grid_on" }
                        }
                        h3 { class: "title-medium", "{t(\"pixel.manageCanvases\")}" }
                        div { class: "flex-grow" }
                        AddCanvasButton { context_id: context_id.clone() }
                    }
                    div { class: "card-content",
                        if canvases.is_empty() && still_looking {
                            div { class: "spinner spinner-sm" }
                        } else if canvases.is_empty() {
                            p { class: "body-medium text-muted", "{t(\"pixel.noCanvases\")}" }
                        } else {
                            div { class: "list",
                                for canvas in canvases {
                                    CanvasRow {
                                        key: "{canvas.id.0}",
                                        canvas_id: canvas.id.0.clone(),
                                        name: canvas.name.clone(),
                                        context_id: context_id.clone(),
                                        is_showing: showing.as_deref() == Some(canvas.id.0.as_str()),
                                        is_open: canvas.mutable,
                                    }
                                }
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
fn CanvasRow(
    canvas_id: String,
    name: String,
    context_id: String,
    is_showing: bool,
    is_open: bool,
) -> Element {
    let session = use_session();
    let mut confirm = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut locking = use_signal(|| false);

    let lock_id = canvas_id.clone();
    let on_lock = move |_| {
        if locking() {
            return;
        }
        locking.set(true);
        let (id, want) = (lock_id.clone(), !is_open);
        let token = session.read().access_token.clone();
        spawn(async move {
            match graphql::set_canvas_open(token.as_deref(), &id, want).await {
                Ok(()) => crate::session::bump_data_version(),
                Err(e) => {
                    crate::errors::log_handled("lock canvas failed", e);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
            locking.set(false);
        });
    };

    let show_id = canvas_id.clone();
    let show_ctx = context_id.clone();
    let on_show = move |_| {
        let (id, ctx) = (show_id.clone(), show_ctx.clone());
        let token = session.read().access_token.clone();
        spawn(async move {
            if let Err(e) = graphql::set_focused_canvas(token.as_deref(), &ctx, Some(&id)).await {
                crate::errors::log_handled("focus canvas failed", e);
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
                    crate::errors::log_handled("delete canvas failed", e);
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
                disabled: locking(),
                aria_label: if is_open { t("pixel.lock") } else { t("pixel.unlock") },
                title: if is_open { t("pixel.lock") } else { t("pixel.unlock") },
                onclick: on_lock,
                span { class: "material-icons",
                    if is_open { "lock_open" } else { "lock" }
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
    // Held as text, not numbers: a field being typed into passes through empty
    // and through half-written values, and a number signal would fight the
    // person editing it. Read once, on submit.
    let mut width = use_signal(|| DEFAULT_SIDE.to_string());
    let mut height = use_signal(|| DEFAULT_SIDE.to_string());
    let mut busy = use_signal(|| false);

    // What was typed, or the default if it was not a number. The clamp is the
    // same one `create_canvas` applies, so the field cannot ask for a board the
    // backend would quietly cut down.
    let side = |typed: &str| {
        typed
            .trim()
            .parse::<u32>()
            .unwrap_or(DEFAULT_SIDE)
            .clamp(1, graphql::MAX_CANVAS_SIDE)
    };

    let submit = move |_| {
        let title = name.read().trim().to_string();
        if title.is_empty() || *busy.read() {
            return;
        }
        let (w, h) = (side(&width.read()), side(&height.read()));
        let ctx = context_id.clone();
        let token = session.read().access_token.clone();
        busy.set(true);
        spawn(async move {
            match graphql::create_canvas(
                token.as_deref(),
                &ctx,
                &title,
                w,
                h,
                DEFAULT_COOLDOWN,
            )
            .await
            {
                Ok(_) => {
                    crate::session::bump_data_version();
                    busy.set(false);
                    open.set(false);
                    name.set(String::new());
                }
                Err(e) => {
                    busy.set(false);
                    crate::errors::log_handled("create canvas failed", e);
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
            // The app's own field, label and all — this was a bare `input` of
            // its own class, which is why it did not look like the box every
            // other dialog asks a question with.
            div { class: "text-field",
                label { "{t(\"pixel.canvasName\")}" }
                input {
                    r#type: "text",
                    maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
            }
            // How big the board is. Asked here because it cannot be changed
            // afterwards without throwing away what has been painted: the cells
            // are addressed by their coordinates, so a narrower board would
            // strand every cell beyond its new edge.
            div { class: "pixel-size-fields",
                div { class: "text-field",
                    label { "{t(\"pixel.canvasWidth\")}" }
                    input {
                        r#type: "number",
                        min: "1",
                        max: "{graphql::MAX_CANVAS_SIDE}",
                        value: "{width}",
                        oninput: move |e| width.set(e.value()),
                    }
                }
                div { class: "text-field",
                    label { "{t(\"pixel.canvasHeight\")}" }
                    input {
                        r#type: "number",
                        min: "1",
                        max: "{graphql::MAX_CANVAS_SIDE}",
                        value: "{height}",
                        oninput: move |e| height.set(e.value()),
                    }
                }
            }
        }
    }
}
