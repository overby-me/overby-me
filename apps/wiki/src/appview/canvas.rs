//! The canvas: a board a room paints together, one cell per person per cooldown.
//!
//! To the components a canvas is a node whose `data` holds its size and
//! cooldown and whose `mutable` says it is open, with a cell as a hidden child
//! keyed `p_<x>_<y>`. The AppView keeps a canvas and its cells in tables of
//! their own, so they are read there and dressed as those.

use super::seen::saw_node;
use super::{ask, ask_quiet, client, reported};
use crate::model::{InsertedNode, Uuid};
use appview_client::{create_canvas, get_canvas, paint_cell, set_canvas_open};

/// How many cells a canvas may be across or down. The AppView's own cap.
pub const MAX_CANVAS_SIDE: u32 = 128;

/// One painted cell: where it is, what colour, and who put it there.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub at: (u32, u32),
    pub colour: u8,
    pub owner: Option<String>,
    /// When it was last painted, as an ISO timestamp.
    pub when: Option<String>,
}

/// One `{ key, data, ownerId, updatedAt }` row as a [`Cell`]: the shape a live
/// update of a cell arrives in.
pub fn parse_cell_full(row: &serde_json::Value) -> Option<Cell> {
    let key = row.get("key")?.as_str()?;
    let (x, y) = key.strip_prefix("p_")?.split_once('_')?;
    let text = |key: &str| {
        row.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(Cell {
        at: (x.parse().ok()?, y.parse().ok()?),
        colour: u8::try_from(row.get("data")?.get("c")?.as_u64()?).ok()?,
        owner: text("ownerId"),
        when: text("updatedAt"),
    })
}

/// The AppView's `[x, y, colour, painter, painted_at]` as a [`Cell`], with
/// `painter` looked up among the board's painters.
fn cell(row: &serde_json::Value, painters: &[String]) -> Option<Cell> {
    let row = row.as_array()?;
    let number = |i: usize| row.get(i)?.as_u64();
    Some(Cell {
        at: (
            u32::try_from(number(0)?).ok()?,
            u32::try_from(number(1)?).ok()?,
        ),
        colour: u8::try_from(number(2)?).ok()?,
        owner: number(3)
            .and_then(|i| painters.get(usize::try_from(i).ok()?))
            .cloned(),
        when: row.get(4).and_then(|v| v.as_str()).map(str::to_string),
    })
}

/// A moment no cell was painted at or after, for a read that wants the board
/// and none of its cells: timestamps here compare as text.
pub(crate) const NO_CELLS: &str = "9999";

/// A canvas's `data` as the interim keeps it, which is what its component reads.
pub(crate) fn canvas_data(board: &get_canvas::Output) -> serde_json::Value {
    serde_json::json!({ "w": board.width, "h": board.height, "cooldown": board.cooldown })
}

pub(crate) async fn read_canvas(
    access_token: Option<&str>,
    canvas_id: &str,
    since: Option<&str>,
) -> Result<get_canvas::Output, appview_client::Error> {
    let client = client(access_token);
    let params = get_canvas::Params {
        id: canvas_id.to_string(),
        since: since.map(str::to_string),
    };
    ask_quiet(true, || client.get_canvas(&params)).await
}

/// Every painted cell of a canvas.
pub async fn load_canvas(access_token: Option<&str>, canvas_id: &str) -> Result<Vec<Cell>, String> {
    let board = read_canvas(access_token, canvas_id, None)
        .await
        .map_err(|e| reported("getCanvas", &e))?;
    Ok(board
        .cells
        .iter()
        .filter_map(|row| cell(row, &board.painters))
        .collect())
}

/// Paint one cell. The cooldown is the AppView's to enforce, and its refusal is
/// an answer the board expects: said to the caller, and to nobody else.
pub async fn paint_cell(
    access_token: Option<&str>,
    canvas_id: &str,
    _context_id: &str,
    _painter_id: &str,
    x: u32,
    y: u32,
    colour: u8,
) -> Result<(), String> {
    let client = client(access_token);
    let stroke = paint_cell::Input {
        canvas: canvas_id.to_string(),
        x: i64::from(x),
        y: i64::from(y),
        colour: i64::from(colour),
    };
    match ask_quiet(false, || client.paint_cell(&stroke)).await {
        Ok(_) => Ok(()),
        Err(e) if e.name() == Some("TooSoon") => Err(super::said(&e)),
        Err(e) => Err(reported("paintCell", &e)),
    }
}

/// `2026-05-01T18:30:00.123Z` from unix milliseconds.
fn iso(ms: i64) -> String {
    let (secs, millis) = (ms.div_euclid(1000), ms.rem_euclid(1000));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (Howard Hinnant's algorithm), as the AppView's own.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// When the caller last painted here, which the board counts its cooldown from.
/// The AppView says when they may paint next, so this is that less the cooldown.
pub async fn my_last_paint(
    access_token: Option<&str>,
    canvas_id: &str,
    _user_id: &str,
) -> Option<String> {
    let board = read_canvas(access_token, canvas_id, Some(NO_CELLS))
        .await
        .ok()?;
    let next = board.next_paint_at?;
    Some(iso(next - board.cooldown * 1000))
}

pub async fn create_canvas(
    access_token: Option<&str>,
    context_id: &str,
    name: &str,
    width: u32,
    height: u32,
    cooldown_seconds: u32,
) -> Result<InsertedNode, String> {
    let client = client(access_token);
    let board = create_canvas::Input {
        parent_id: context_id.to_string(),
        name: name.to_string(),
        width: Some(i64::from(width.clamp(1, MAX_CANVAS_SIDE))),
        height: Some(i64::from(height.clamp(1, MAX_CANVAS_SIDE))),
        cooldown: Some(i64::from(cooldown_seconds)),
    };
    let made = ask("createCanvas", false, || client.create_canvas(&board)).await?;
    saw_node(&made.id, "document", "canvas");
    Ok(InsertedNode {
        id: Uuid(made.id),
        key: made.path.rsplit('/').next().unwrap_or_default().to_string(),
    })
}

/// Open a canvas to the room, or lock it so it takes no more paint.
pub async fn set_canvas_open(
    access_token: Option<&str>,
    canvas_id: &str,
    open: bool,
) -> Result<(), String> {
    let client = client(access_token);
    let change = set_canvas_open::Input {
        id: canvas_id.to_string(),
        open,
    };
    ask("setCanvasOpen", false, || client.set_canvas_open(&change)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_is_written_as_the_appview_writes_it() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(iso(1_582_934_400_000), "2020-02-29T00:00:00.000Z");
    }

    #[test]
    fn a_cell_is_read_from_either_shape() {
        let painters = ["did:plc:bob".to_string()];
        let row = serde_json::json!([3, 4, 7, 0, "2026-05-01T18:30:00.000Z"]);
        let painted = cell(&row, &painters).expect("a cell");
        assert_eq!((painted.at, painted.colour), ((3, 4), 7));
        assert_eq!(painted.owner.as_deref(), Some("did:plc:bob"));
        let nobody = serde_json::json!([0, 0, 1, null, "2026-05-01T18:30:00.000Z"]);
        assert_eq!(cell(&nobody, &painters).expect("a cell").owner, None);

        let pushed =
            serde_json::json!({"key": "p_3_4", "data": {"c": 7}, "ownerId": "did:plc:bob"});
        assert_eq!(parse_cell_full(&pushed).expect("a cell").at, (3, 4));
        assert_eq!(parse_cell_full(&serde_json::json!({"key": "p_3"})), None);
    }
}
