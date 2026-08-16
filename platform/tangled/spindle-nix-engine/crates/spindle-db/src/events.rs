//! Pipeline event log queries.
//!
//! Manages the `events` table, which stores pipeline status events as JSON
//! blobs for WebSocket `/events` backfill. Clients connect with a cursor
//! (event ID) and receive all events after that cursor.
//!
//! Also manages the `last_time_us` singleton table for Jetstream cursor
//! persistence, and the `knots` table for knot cursor tracking.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// A stored pipeline event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Auto-incrementing event ID.
    pub id: i64,
    /// Event kind (legacy, e.g. `"pipeline_status"`).
    pub kind: String,
    /// JSON-encoded event payload.
    pub payload: String,
    /// When the event was created (ISO 8601).
    pub created_at: String,
    /// Record key (TID-like unique identifier for the event).
    pub rkey: String,
    /// AT Protocol NSID (e.g. `"sh.tangled.pipeline.status"`).
    pub nsid: String,
    /// Unix nanosecond timestamp (used as cursor by the appview).
    pub created: i64,
}

/// A knot cursor record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnotCursor {
    /// Knot server hostname.
    pub knot: String,
    /// Last processed event cursor for this knot (may be `None`).
    pub cursor: Option<String>,
    /// When the knot was added.
    pub added_at: String,
}

// ---------------------------------------------------------------------------
// events table
// ---------------------------------------------------------------------------

/// Parameters for inserting a new pipeline event.
pub struct InsertEventParams<'a> {
    /// Record key (unique identifier for this event).
    pub rkey: &'a str,
    /// AT Protocol NSID (e.g. `"sh.tangled.pipeline.status"`).
    pub nsid: &'a str,
    /// JSON-encoded event payload.
    pub payload: &'a str,
    /// Unix nanosecond timestamp.
    pub created: i64,
}

/// Insert a new pipeline event.
///
/// Returns the auto-generated event ID.
pub fn insert_event(conn: &Connection, params: &InsertEventParams) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO events (kind, payload, rkey, nsid, created) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            params.nsid,
            params.payload,
            params.rkey,
            params.nsid,
            params.created
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Get all events with a `created` timestamp greater than the given cursor.
///
/// This is the primary query for WebSocket `/events` backfill: a client
/// provides its last-seen cursor (unix nanos) and receives all newer events.
///
/// Results are ordered by `created` ascending (oldest first), limited to 100.
pub fn get_events_after(conn: &Connection, cursor: i64) -> rusqlite::Result<Vec<Event>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, payload, created_at, rkey, nsid, created \
         FROM events WHERE created > ?1 ORDER BY created ASC LIMIT 100",
    )?;

    let events = stmt
        .query_map(params![cursor], |row| {
            Ok(Event {
                id: row.get(0)?,
                kind: row.get(1)?,
                payload: row.get(2)?,
                created_at: row.get(3)?,
                rkey: row.get(4)?,
                nsid: row.get(5)?,
                created: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(events)
}

/// Get all events (no cursor filter).
///
/// Results are ordered by ID ascending.
pub fn get_all_events(conn: &Connection) -> rusqlite::Result<Vec<Event>> {
    get_events_after(conn, 0)
}

/// Get the latest event ID (the current cursor head).
///
/// Returns `None` if no events exist.
pub fn get_latest_event_id(conn: &Connection) -> rusqlite::Result<Option<i64>> {
    conn.query_row("SELECT MAX(id) FROM events", [], |row| row.get(0))
}

/// Get a single event by ID.
pub fn get_event(conn: &Connection, id: i64) -> rusqlite::Result<Option<Event>> {
    conn.query_row(
        "SELECT id, kind, payload, created_at, rkey, nsid, created FROM events WHERE id = ?1",
        params![id],
        |row| {
            Ok(Event {
                id: row.get(0)?,
                kind: row.get(1)?,
                payload: row.get(2)?,
                created_at: row.get(3)?,
                rkey: row.get(4)?,
                nsid: row.get(5)?,
                created: row.get(6)?,
            })
        },
    )
    .optional()
}

/// Get the total number of stored events.
pub fn event_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
}

// ---------------------------------------------------------------------------
// last_time_us table (Jetstream cursor)
// ---------------------------------------------------------------------------

/// Save the Jetstream cursor (last processed event timestamp in microseconds).
///
/// The `last_time_us` table is a singleton — there's always exactly one row
/// with `id = 1`, initialized to `0` by the migration.
pub fn save_last_time_us(conn: &Connection, time_us: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE last_time_us SET time_us = ?1 WHERE id = 1",
        params![time_us],
    )?;
    Ok(())
}

/// Get the Jetstream cursor (last processed event timestamp in microseconds).
///
/// Returns `0` if no cursor has been saved yet (the default from the migration).
pub fn get_last_time_us(conn: &Connection) -> rusqlite::Result<i64> {
    let time_us: i64 =
        conn.query_row("SELECT time_us FROM last_time_us WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    Ok(time_us)
}

// ---------------------------------------------------------------------------
// knots table
// ---------------------------------------------------------------------------

/// Add a knot to the tracking table.
///
/// If the knot already exists, this is a no-op (`INSERT OR IGNORE`).
pub fn add_knot(conn: &Connection, knot: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO knots (knot) VALUES (?1)",
        params![knot],
    )?;
    Ok(())
}

/// Remove a knot from the tracking table.
///
/// Returns `true` if a row was deleted, `false` if the knot wasn't found.
pub fn remove_knot(conn: &Connection, knot: &str) -> rusqlite::Result<bool> {
    let deleted = conn.execute("DELETE FROM knots WHERE knot = ?1", params![knot])?;
    Ok(deleted > 0)
}

/// Update the cursor for a knot.
pub fn update_knot_cursor(conn: &Connection, knot: &str, cursor: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE knots SET cursor = ?1 WHERE knot = ?2",
        params![cursor, knot],
    )?;
    Ok(())
}

/// Get the cursor for a knot.
///
/// Returns `None` if the knot doesn't exist or has no cursor set.
pub fn get_knot_cursor(conn: &Connection, knot: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT cursor FROM knots WHERE knot = ?1",
        params![knot],
        |row| row.get(0),
    )
    .optional()
    // Flatten: if the row exists but cursor is NULL, we still want None
    .map(|opt| opt.flatten())
}

/// Get all tracked knots with their cursors.
pub fn get_all_knots(conn: &Connection) -> rusqlite::Result<Vec<KnotCursor>> {
    let mut stmt = conn.prepare("SELECT knot, cursor, added_at FROM knots ORDER BY knot")?;

    let knots = stmt
        .query_map([], |row| {
            Ok(KnotCursor {
                knot: row.get(0)?,
                cursor: row.get(1)?,
                added_at: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(knots)
}

/// Get all tracked knot hostnames (without cursor data).
pub fn get_knot_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT knot FROM knots ORDER BY knot")?;

    let names = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations;

    fn setup_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        migrations::migrate(&mut conn).unwrap();
        conn
    }

    /// Helper to create test event params with incrementing timestamps.
    fn test_params<'a>(nsid: &'a str, payload: &'a str, created: i64) -> InsertEventParams<'a> {
        InsertEventParams {
            rkey: "test-rkey",
            nsid,
            payload,
            created,
        }
    }

    // -----------------------------------------------------------------------
    // events table tests
    // -----------------------------------------------------------------------

    #[test]
    fn insert_and_get_event() {
        let conn = setup_db();

        let params = InsertEventParams {
            rkey: "rkey-1",
            nsid: "sh.tangled.pipeline.status",
            payload: r#"{"status":"running"}"#,
            created: 1000,
        };
        let id = insert_event(&conn, &params).unwrap();
        assert!(id > 0);

        let event = get_event(&conn, id).unwrap().expect("event should exist");
        assert_eq!(event.id, id);
        assert_eq!(event.nsid, "sh.tangled.pipeline.status");
        assert_eq!(event.rkey, "rkey-1");
        assert_eq!(event.payload, r#"{"status":"running"}"#);
        assert_eq!(event.created, 1000);
    }

    #[test]
    fn get_event_not_found() {
        let conn = setup_db();
        let event = get_event(&conn, 9999).unwrap();
        assert!(event.is_none());
    }

    #[test]
    fn get_events_after_cursor() {
        let conn = setup_db();

        insert_event(&conn, &test_params("a", "payload1", 100)).unwrap();
        insert_event(&conn, &test_params("b", "payload2", 200)).unwrap();
        insert_event(&conn, &test_params("c", "payload3", 300)).unwrap();

        // Get events after cursor=100 (first event's created timestamp)
        let events = get_events_after(&conn, 100).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].nsid, "b");
        assert_eq!(events[1].nsid, "c");
    }

    #[test]
    fn get_events_after_cursor_zero_returns_all() {
        let conn = setup_db();

        insert_event(&conn, &test_params("a", "p1", 100)).unwrap();
        insert_event(&conn, &test_params("b", "p2", 200)).unwrap();

        let events = get_events_after(&conn, 0).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn get_events_after_cursor_beyond_latest() {
        let conn = setup_db();

        insert_event(&conn, &test_params("a", "p1", 100)).unwrap();

        let events = get_events_after(&conn, 9999).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn get_all_events_empty() {
        let conn = setup_db();
        let events = get_all_events(&conn).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn get_latest_event_id_empty() {
        let conn = setup_db();
        let latest = get_latest_event_id(&conn).unwrap();
        assert!(latest.is_none());
    }

    #[test]
    fn get_latest_event_id_returns_max() {
        let conn = setup_db();

        insert_event(&conn, &test_params("a", "p1", 100)).unwrap();
        let id2 = insert_event(&conn, &test_params("b", "p2", 200)).unwrap();

        let latest = get_latest_event_id(&conn).unwrap();
        assert_eq!(latest, Some(id2));
    }

    #[test]
    fn event_count_empty() {
        let conn = setup_db();
        assert_eq!(event_count(&conn).unwrap(), 0);
    }

    #[test]
    fn event_count_after_inserts() {
        let conn = setup_db();

        insert_event(&conn, &test_params("a", "p1", 100)).unwrap();
        insert_event(&conn, &test_params("b", "p2", 200)).unwrap();
        insert_event(&conn, &test_params("c", "p3", 300)).unwrap();

        assert_eq!(event_count(&conn).unwrap(), 3);
    }

    #[test]
    fn events_ordered_by_created_ascending() {
        let conn = setup_db();

        // Insert in non-sequential order to verify ordering by created
        insert_event(&conn, &test_params("first", "p1", 100)).unwrap();
        insert_event(&conn, &test_params("second", "p2", 200)).unwrap();
        insert_event(&conn, &test_params("third", "p3", 300)).unwrap();

        let events = get_all_events(&conn).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].created, 100);
        assert_eq!(events[1].created, 200);
        assert_eq!(events[2].created, 300);
    }

    // -----------------------------------------------------------------------
    // last_time_us tests
    // -----------------------------------------------------------------------

    #[test]
    fn last_time_us_default_is_zero() {
        let conn = setup_db();
        assert_eq!(get_last_time_us(&conn).unwrap(), 0);
    }

    #[test]
    fn save_and_get_last_time_us() {
        let conn = setup_db();

        save_last_time_us(&conn, 1_700_000_000_000_000).unwrap();
        assert_eq!(get_last_time_us(&conn).unwrap(), 1_700_000_000_000_000);

        save_last_time_us(&conn, 1_800_000_000_000_000).unwrap();
        assert_eq!(get_last_time_us(&conn).unwrap(), 1_800_000_000_000_000);
    }

    // -----------------------------------------------------------------------
    // knots table tests
    // -----------------------------------------------------------------------

    #[test]
    fn add_and_get_knots() {
        let conn = setup_db();

        add_knot(&conn, "knot-a.example.com").unwrap();
        add_knot(&conn, "knot-b.example.com").unwrap();

        let names = get_knot_names(&conn).unwrap();
        assert_eq!(names, vec!["knot-a.example.com", "knot-b.example.com"]);
    }

    #[test]
    fn add_knot_is_idempotent() {
        let conn = setup_db();

        add_knot(&conn, "knot.example.com").unwrap();
        add_knot(&conn, "knot.example.com").unwrap();

        let names = get_knot_names(&conn).unwrap();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn remove_knot_existing() {
        let conn = setup_db();

        add_knot(&conn, "knot.example.com").unwrap();
        assert!(remove_knot(&conn, "knot.example.com").unwrap());

        let names = get_knot_names(&conn).unwrap();
        assert!(names.is_empty());
    }

    #[test]
    fn remove_knot_not_found() {
        let conn = setup_db();
        assert!(!remove_knot(&conn, "nonexistent.example.com").unwrap());
    }

    #[test]
    fn knot_cursor_initially_none() {
        let conn = setup_db();

        add_knot(&conn, "knot.example.com").unwrap();
        let cursor = get_knot_cursor(&conn, "knot.example.com").unwrap();
        assert!(cursor.is_none());
    }

    #[test]
    fn update_and_get_knot_cursor() {
        let conn = setup_db();

        add_knot(&conn, "knot.example.com").unwrap();
        update_knot_cursor(&conn, "knot.example.com", "cursor-abc-123").unwrap();

        let cursor = get_knot_cursor(&conn, "knot.example.com").unwrap();
        assert_eq!(cursor.as_deref(), Some("cursor-abc-123"));
    }

    #[test]
    fn update_knot_cursor_overwrites() {
        let conn = setup_db();

        add_knot(&conn, "knot.example.com").unwrap();
        update_knot_cursor(&conn, "knot.example.com", "cursor-1").unwrap();
        update_knot_cursor(&conn, "knot.example.com", "cursor-2").unwrap();

        let cursor = get_knot_cursor(&conn, "knot.example.com").unwrap();
        assert_eq!(cursor.as_deref(), Some("cursor-2"));
    }

    #[test]
    fn get_knot_cursor_nonexistent_knot() {
        let conn = setup_db();
        let cursor = get_knot_cursor(&conn, "nonexistent.example.com").unwrap();
        assert!(cursor.is_none());
    }

    #[test]
    fn get_all_knots_with_cursors() {
        let conn = setup_db();

        add_knot(&conn, "knot-a.example.com").unwrap();
        add_knot(&conn, "knot-b.example.com").unwrap();
        update_knot_cursor(&conn, "knot-a.example.com", "cursor-a").unwrap();

        let knots = get_all_knots(&conn).unwrap();
        assert_eq!(knots.len(), 2);
        assert_eq!(knots[0].knot, "knot-a.example.com");
        assert_eq!(knots[0].cursor.as_deref(), Some("cursor-a"));
        assert_eq!(knots[1].knot, "knot-b.example.com");
        assert!(knots[1].cursor.is_none());
    }

    #[test]
    fn get_all_knots_empty() {
        let conn = setup_db();
        let knots = get_all_knots(&conn).unwrap();
        assert!(knots.is_empty());
    }

    #[test]
    fn get_knot_names_sorted() {
        let conn = setup_db();

        add_knot(&conn, "charlie.example.com").unwrap();
        add_knot(&conn, "alpha.example.com").unwrap();
        add_knot(&conn, "bravo.example.com").unwrap();

        let names = get_knot_names(&conn).unwrap();
        assert_eq!(
            names,
            vec![
                "alpha.example.com",
                "bravo.example.com",
                "charlie.example.com"
            ]
        );
    }
}
