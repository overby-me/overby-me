//! The pixel canvas (`pixel/canvas` + `pixel/pixel`).
//!
//! A canvas is an ordinary node; each painted cell is a hidden child keyed
//! `p_<x>_<y>`, so the unique index on `(parent_id, key)` makes a cell's identity
//! the database's problem rather than ours, and repainting is an update of that
//! row. The mimes mirror pairs the app already has (`vote/poll` + `vote/vote`,
//! `speak/list` + `speak/speak`).
//!
//! Raw GraphQL rather than cynic fragments throughout: these are three small
//! operations over one lean row shape, and the cost of a wire type per operation
//! is not repaid here.

use super::*;

/// How many cells a canvas may be across or down.
///
/// A cap, not a recommendation. 128x128 is 16,384 rows, which inserts in about
/// three seconds and is far more than a room will ever fill; the point is that a
/// mistyped number cannot ask the database for a million nodes.
pub const MAX_CANVAS_SIDE: u32 = 128;

/// The colour of every painted cell of a canvas, as `((x, y), colour)`.
///
/// Only `key` and `data` are selected. The rest of a node row (name, path,
/// ancestors, timestamps, the owner relation) is dead weight multiplied by the
/// number of cells, which is the whole reason this is fast.
pub async fn load_canvas(
    access_token: Option<&str>,
    canvas_id: &str,
) -> Result<Vec<((u32, u32), u8)>, String> {
    let data = execute_raw_vars(
        access_token,
        "query($p: uuid!) { \
           nodes(where: {parentId: {_eq: $p}, mimeId: {_eq: \"pixel/pixel\"}}) { key data } \
         }",
        serde_json::json!({ "p": canvas_id }),
    )
    .await?;
    // `execute_raw_vars` returns the `data` OBJECT, so `nodes` is at the top.
    let rows = data
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(rows.iter().filter_map(parse_cell).collect())
}

/// One `{ key, data }` row as `((x, y), colour)`, or `None` if it is not a cell.
///
/// Pure, so the wire shape can be tested without a browser or a server.
pub fn parse_cell(row: &serde_json::Value) -> Option<((u32, u32), u8)> {
    let key = row.get("key")?.as_str()?;
    let colour = row.get("data")?.get("c")?.as_u64()? as u8;
    let (x, y) = parse_key(key)?;
    Some(((x, y), colour))
}

/// `p_<x>_<y>` back into coordinates.
pub fn parse_key(key: &str) -> Option<(u32, u32)> {
    let rest = key.strip_prefix("p_")?;
    let (x, y) = rest.split_once('_')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// The key a cell is stored under.
pub fn cell_key(x: u32, y: u32) -> String {
    format!("p_{x}_{y}")
}

/// Paint one cell, creating it or repainting it.
///
/// Update first, insert only if the cell was untouched. The obvious `on_conflict`
/// upsert is not available: `(parent_id, key)` is a PARTIAL unique index
/// (`where deleted_at is null`), and Postgres `on conflict on constraint` takes
/// only real constraints, so Hasura answers "constraint … does not exist".
/// Verified against production rather than inferred from the schema, which lists
/// the index in `nodes_constraint` as though it were usable.
///
/// A repaint is therefore one round trip and a fresh cell is two, both bounded by
/// the cooldown the database enforces (`migrations/0007`).
pub async fn paint_cell(
    access_token: Option<&str>,
    canvas_id: &str,
    context_id: &str,
    x: u32,
    y: u32,
    colour: u8,
) -> Result<(), String> {
    let key = cell_key(x, y);
    let updated = execute_raw_vars_quiet(
        access_token,
        "mutation($p: uuid!, $k: String!, $d: jsonb!) { \
           updateNodes(where: {parentId: {_eq: $p}, key: {_eq: $k}}, _set: {data: $d}) \
           { affected_rows } }",
        serde_json::json!({ "p": canvas_id, "k": key, "d": { "c": colour } }),
    )
    .await?;
    if affected_rows(&updated) > 0 {
        return Ok(());
    }
    execute_raw_vars_quiet(
        access_token,
        "mutation($o: nodes_insert_input!) { insertNode(object: $o) { id } }",
        serde_json::json!({ "o": {
            "parentId": canvas_id,
            "contextId": context_id,
            "key": key,
            "name": "px",
            "mimeId": "pixel/pixel",
            "mutable": false,
            "data": { "c": colour },
        }}),
    )
    .await
    .map(|_| ())
}

/// How many rows an `updateNodes` touched. Zero means the cell is new.
fn affected_rows(data: &serde_json::Value) -> u64 {
    data.get("updateNodes")
        .and_then(|u| u.get("affected_rows"))
        .and_then(|a| a.as_u64())
        .unwrap_or(0)
}

/// When this person last painted here, as an ISO timestamp.
///
/// The cooldown is enforced by the database, and the database's refusal does not
/// reliably reach the client: Hasura puts the trigger's `retry_after_ms` in
/// `extensions.internal`, which it omits outside dev mode. So the countdown is
/// derived from a fact the client can always fetch instead. It also survives a
/// reload, which parsing an error never could.
pub async fn my_last_paint(
    access_token: Option<&str>,
    canvas_id: &str,
    user_id: &str,
) -> Option<String> {
    let data = execute_raw_vars(
        access_token,
        "query($p: uuid!, $u: uuid!) { \
           nodesAggregate(where: {parentId: {_eq: $p}, mimeId: {_eq: \"pixel/pixel\"}, \
                                  ownerId: {_eq: $u}}) \
           { aggregate { max { updatedAt } } } }",
        serde_json::json!({ "p": canvas_id, "u": user_id }),
    )
    .await
    .ok()?;
    data.get("nodesAggregate")?
        .get("aggregate")?
        .get("max")?
        .get("updatedAt")?
        .as_str()
        .map(str::to_string)
}

/// Everything painted on this canvas since `since`, pushed as it happens.
///
/// A STREAMING subscription, not a live query: it delivers only rows newer than
/// the cursor, so a placement is one small frame rather than the whole canvas
/// re-sent to everybody. Verified against this deployment before the app was
/// built. Scoped by `parentId`, so watching one canvas never carries another's
/// traffic.
pub fn canvas_stream(canvas_id: &str, since: &str) -> String {
    format!(
        "subscription {{ \
           nodes_stream(batch_size: 200, \
                        cursor: {{initial_value: {{updatedAt: \"{since}\"}}, ordering: ASC}}, \
                        where: {{parentId: {{_eq: \"{canvas}\"}}, \
                                 mimeId: {{_eq: \"pixel/pixel\"}}}}) \
           {{ key data }} }}",
        since = gql_escape(since),
        canvas = gql_escape(canvas_id),
    )
}

/// Create a canvas under `context_id`, granting the two permissions it needs if
/// this context has never had one.
///
/// Mirrors `create_speaker_list`: an owner may seed permissions for their own
/// context, so a feature can arrive in an existing context without a migration
/// touching every row in the database.
///
/// `cooldown` is the rate limit written onto the member permission, and it is
/// what makes the canvas survivable in a hall: it bounds the whole feature's
/// write rate to one placement per person per interval, enforced by the trigger
/// rather than by the client asking nicely.
pub async fn create_canvas(
    access_token: Option<&str>,
    context_id: &str,
    name: &str,
    width: u32,
    height: u32,
    cooldown_seconds: u32,
) -> Result<model::InsertedNode, String> {
    let width = width.clamp(1, MAX_CANVAS_SIDE);
    let height = height.clamp(1, MAX_CANVAS_SIDE);

    let existing = node_insert_mimes(access_token, context_id).await;
    if !existing.iter().any(|m| m == "pixel/canvas") {
        execute_raw_vars(
            access_token,
            "mutation($objs: [permissions_insert_input!]!) { \
               insertPermissions(objects: $objs) { affected_rows } }",
            serde_json::json!({ "objs": [
                {
                    "contextId": context_id,
                    "nodeId": context_id,
                    "mimeId": "pixel/canvas",
                    "role": "owner",
                    "parents": ["wiki/event", "wiki/group", "wiki/folder"],
                    "active": true,
                    "insert": true, "select": true, "update": true, "delete": true,
                },
                {
                    "contextId": context_id,
                    "nodeId": context_id,
                    "mimeId": "pixel/pixel",
                    "role": "member",
                    "parents": ["pixel/canvas"],
                    "active": true,
                    // No delete: a cell is repainted, never removed, so the
                    // canvas cannot be quietly erased one cell at a time.
                    "insert": true, "select": true, "update": true, "delete": false,
                    "rate_limit": format!("{cooldown_seconds} seconds"),
                },
            ] }),
        )
        .await?;
    }

    insert_node_named(
        access_token,
        model::NodesInsertInput {
            name: Some(name.to_string()),
            key: None,
            mime_id: Some("pixel/canvas".to_string()),
            parent_id: Some(model::Uuid(context_id.to_string())),
            context_id: Some(model::Uuid(context_id.to_string())),
            data: Some(model::Jsonb(serde_json::json!({
                "w": width, "h": height, "cooldown": cooldown_seconds,
            }))),
            ..Default::default()
        },
        name,
    )
    .await?
    .ok_or_else(|| "canvas not created".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_key_round_trips() {
        assert_eq!(cell_key(0, 0), "p_0_0");
        assert_eq!(parse_key(&cell_key(12, 34)), Some((12, 34)));
        // Anything that is not a cell is ignored rather than guessed at.
        assert_eq!(parse_key("p_12"), None);
        assert_eq!(parse_key("12_34"), None);
        assert_eq!(parse_key("p_x_1"), None);
    }

    #[test]
    fn a_row_becomes_a_coloured_cell() {
        let row = serde_json::json!({"key": "p_3_4", "data": {"c": 7}});
        assert_eq!(parse_cell(&row), Some(((3, 4), 7)));
        // A row without a colour is not a cell; better to skip it than to paint
        // an arbitrary one.
        assert_eq!(parse_cell(&serde_json::json!({"key": "p_3_4"})), None);
        assert_eq!(parse_cell(&serde_json::json!({"data": {"c": 1}})), None);
    }

    /// A repaint is an update; only an untouched cell is inserted.
    #[test]
    fn a_painted_cell_is_updated_rather_than_inserted() {
        let updated = serde_json::json!({"updateNodes": {"affected_rows": 1}});
        assert_eq!(affected_rows(&updated), 1);
        let missing = serde_json::json!({"updateNodes": {"affected_rows": 0}});
        assert_eq!(affected_rows(&missing), 0, "zero means insert it");
        // A shape we do not recognise must not be read as "already painted", or
        // the cell would silently never appear.
        assert_eq!(affected_rows(&serde_json::json!({})), 0);
    }

    /// The stream must ask for a delta, scoped to one canvas.
    #[test]
    fn the_subscription_streams_one_canvas() {
        let q = canvas_stream("canvas-1", "2026-07-31T00:00:00Z");
        assert!(q.contains("nodes_stream"), "{q}");
        assert!(q.contains("cursor:"), "must be a delta, not a live query: {q}");
        assert!(q.contains(r#"parentId: {_eq: "canvas-1"}"#), "{q}");
        assert!(q.contains(r#"mimeId: {_eq: "pixel/pixel"}"#), "{q}");
    }

    /// A mistyped size cannot ask the database for a million rows.
    #[test]
    fn a_canvas_side_is_capped() {
        assert_eq!(1000u32.clamp(1, MAX_CANVAS_SIDE), MAX_CANVAS_SIDE);
        assert_eq!(0u32.clamp(1, MAX_CANVAS_SIDE), 1);
    }
}
