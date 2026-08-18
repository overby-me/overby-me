//! Workflow execution status tracking queries.
//!
//! Manages the `workflow_status` table, which tracks the lifecycle of each
//! workflow execution: `pending` → `running` → `success`/`failed`/`timeout`/`cancelled`.
//!
//! Each workflow execution gets one row, identified by its normalized
//! [`WorkflowId`](spindle_models::WorkflowId) string.

use rusqlite::{Connection, OptionalExtension, params};

/// A workflow status record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStatusRow {
    /// Database row ID.
    pub id: i64,
    /// Normalized workflow ID string (e.g. `"example.com-abc123-test"`).
    pub workflow_id: String,
    /// Knot server hostname from the pipeline ID.
    pub pipeline_knot: String,
    /// Pipeline record key.
    pub pipeline_rkey: String,
    /// Repository DID that owns this pipeline.
    pub repo_did: String,
    /// Workflow name (from the YAML filename).
    pub workflow_name: String,
    /// Current status (e.g. `"pending"`, `"running"`, `"success"`).
    pub status: String,
    /// When the workflow started running (ISO 8601), or `None` if still pending.
    pub started_at: Option<String>,
    /// When the workflow reached a terminal state (ISO 8601), or `None` if not finished.
    pub finished_at: Option<String>,
    /// When the record was created (ISO 8601).
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Insert / update operations
// ---------------------------------------------------------------------------

/// Create a new workflow status record in `pending` state.
///
/// If a record with the same `workflow_id` already exists, this is a no-op
/// (`INSERT OR IGNORE`).
pub fn status_pending(
    conn: &Connection,
    workflow_id: &str,
    pipeline_knot: &str,
    pipeline_rkey: &str,
    repo_did: &str,
    workflow_name: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO workflow_status \
         (workflow_id, pipeline_knot, pipeline_rkey, repo_did, workflow_name, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
        params![
            workflow_id,
            pipeline_knot,
            pipeline_rkey,
            repo_did,
            workflow_name
        ],
    )?;
    Ok(())
}

/// Transition a workflow to `running` state.
///
/// Sets `started_at` to the current UTC timestamp.
pub fn status_running(conn: &Connection, workflow_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_status \
         SET status = 'running', started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workflow_id = ?1",
        params![workflow_id],
    )?;
    Ok(())
}

/// Transition a workflow to `success` state.
///
/// Sets `finished_at` to the current UTC timestamp.
pub fn status_success(conn: &Connection, workflow_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_status \
         SET status = 'success', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workflow_id = ?1",
        params![workflow_id],
    )?;
    Ok(())
}

/// Transition a workflow to `failed` state.
///
/// Sets `finished_at` to the current UTC timestamp.
pub fn status_failed(conn: &Connection, workflow_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_status \
         SET status = 'failed', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workflow_id = ?1",
        params![workflow_id],
    )?;
    Ok(())
}

/// Transition a workflow to `timeout` state.
///
/// Sets `finished_at` to the current UTC timestamp.
pub fn status_timeout(conn: &Connection, workflow_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_status \
         SET status = 'timeout', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workflow_id = ?1",
        params![workflow_id],
    )?;
    Ok(())
}

/// Transition a workflow to `cancelled` state.
///
/// Sets `finished_at` to the current UTC timestamp.
pub fn status_cancelled(conn: &Connection, workflow_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE workflow_status \
         SET status = 'cancelled', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workflow_id = ?1",
        params![workflow_id],
    )?;
    Ok(())
}

/// Cancel all orphaned workflows that are still in `pending` or `running` state.
///
/// These are leftovers from a previous process that crashed or was restarted
/// before the workflows could reach a terminal state. Returns the number of
/// rows updated.
pub fn cancel_orphaned(conn: &Connection) -> rusqlite::Result<usize> {
    let updated = conn.execute(
        "UPDATE workflow_status \
         SET status = 'cancelled', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE status IN ('pending', 'running')",
        [],
    )?;
    Ok(updated)
}

// ---------------------------------------------------------------------------
// Query operations
// ---------------------------------------------------------------------------

/// Get the status record for a workflow by its normalized ID.
pub fn get_status(
    conn: &Connection,
    workflow_id: &str,
) -> rusqlite::Result<Option<WorkflowStatusRow>> {
    conn.query_row(
        "SELECT id, workflow_id, pipeline_knot, pipeline_rkey, repo_did, workflow_name, \
         status, started_at, finished_at, created_at \
         FROM workflow_status WHERE workflow_id = ?1",
        params![workflow_id],
        |row| {
            Ok(WorkflowStatusRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                pipeline_knot: row.get(2)?,
                pipeline_rkey: row.get(3)?,
                repo_did: row.get(4)?,
                workflow_name: row.get(5)?,
                status: row.get(6)?,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        },
    )
    .optional()
}

/// Get all workflow status records for a specific pipeline (by knot + rkey).
///
/// Results are ordered by creation time ascending.
pub fn get_statuses_for_pipeline(
    conn: &Connection,
    pipeline_knot: &str,
    pipeline_rkey: &str,
) -> rusqlite::Result<Vec<WorkflowStatusRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, workflow_id, pipeline_knot, pipeline_rkey, repo_did, workflow_name, \
         status, started_at, finished_at, created_at \
         FROM workflow_status \
         WHERE pipeline_knot = ?1 AND pipeline_rkey = ?2 \
         ORDER BY id ASC",
    )?;

    let rows = stmt
        .query_map(params![pipeline_knot, pipeline_rkey], |row| {
            Ok(WorkflowStatusRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                pipeline_knot: row.get(2)?,
                pipeline_rkey: row.get(3)?,
                repo_did: row.get(4)?,
                workflow_name: row.get(5)?,
                status: row.get(6)?,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Get all workflow status records for a specific repo DID.
///
/// Results are ordered by creation time ascending.
pub fn get_statuses_for_repo(
    conn: &Connection,
    repo_did: &str,
) -> rusqlite::Result<Vec<WorkflowStatusRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, workflow_id, pipeline_knot, pipeline_rkey, repo_did, workflow_name, \
         status, started_at, finished_at, created_at \
         FROM workflow_status \
         WHERE repo_did = ?1 \
         ORDER BY id ASC",
    )?;

    let rows = stmt
        .query_map(params![repo_did], |row| {
            Ok(WorkflowStatusRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                pipeline_knot: row.get(2)?,
                pipeline_rkey: row.get(3)?,
                repo_did: row.get(4)?,
                workflow_name: row.get(5)?,
                status: row.get(6)?,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Get all workflow status records with a specific status value.
///
/// Results are ordered by creation time ascending.
pub fn get_statuses_by_status(
    conn: &Connection,
    status: &str,
) -> rusqlite::Result<Vec<WorkflowStatusRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, workflow_id, pipeline_knot, pipeline_rkey, repo_did, workflow_name, \
         status, started_at, finished_at, created_at \
         FROM workflow_status \
         WHERE status = ?1 \
         ORDER BY id ASC",
    )?;

    let rows = stmt
        .query_map(params![status], |row| {
            Ok(WorkflowStatusRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                pipeline_knot: row.get(2)?,
                pipeline_rkey: row.get(3)?,
                repo_did: row.get(4)?,
                workflow_name: row.get(5)?,
                status: row.get(6)?,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Get all workflow status records.
///
/// Results are ordered by ID ascending.
pub fn get_all_statuses(conn: &Connection) -> rusqlite::Result<Vec<WorkflowStatusRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, workflow_id, pipeline_knot, pipeline_rkey, repo_did, workflow_name, \
         status, started_at, finished_at, created_at \
         FROM workflow_status \
         ORDER BY id ASC",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(WorkflowStatusRow {
                id: row.get(0)?,
                workflow_id: row.get(1)?,
                pipeline_knot: row.get(2)?,
                pipeline_rkey: row.get(3)?,
                repo_did: row.get(4)?,
                workflow_name: row.get(5)?,
                status: row.get(6)?,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Count workflows in a specific status.
pub fn count_by_status(conn: &Connection, status: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM workflow_status WHERE status = ?1",
        params![status],
        |row| row.get(0),
    )
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

    #[test]
    fn status_pending_creates_record() {
        let conn = setup_db();

        status_pending(
            &conn,
            "example.com-abc123-test",
            "example.com",
            "abc123",
            "did:plc:test",
            "test",
        )
        .unwrap();

        let row = get_status(&conn, "example.com-abc123-test")
            .unwrap()
            .expect("status should exist");
        assert_eq!(row.workflow_id, "example.com-abc123-test");
        assert_eq!(row.pipeline_knot, "example.com");
        assert_eq!(row.pipeline_rkey, "abc123");
        assert_eq!(row.workflow_name, "test");
        assert_eq!(row.status, "pending");
        assert!(row.started_at.is_none());
        assert!(row.finished_at.is_none());
    }

    #[test]
    fn status_pending_is_idempotent() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();

        let all = get_all_statuses(&conn).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn transition_pending_to_running() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_running(&conn, "wid-1").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert_eq!(row.status, "running");
        assert!(row.started_at.is_some());
        assert!(row.finished_at.is_none());
    }

    #[test]
    fn transition_running_to_success() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_running(&conn, "wid-1").unwrap();
        status_success(&conn, "wid-1").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert_eq!(row.status, "success");
        assert!(row.started_at.is_some());
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn transition_running_to_failed() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_running(&conn, "wid-1").unwrap();
        status_failed(&conn, "wid-1").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert_eq!(row.status, "failed");
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn transition_running_to_timeout() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_running(&conn, "wid-1").unwrap();
        status_timeout(&conn, "wid-1").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert_eq!(row.status, "timeout");
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn transition_running_to_cancelled() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_running(&conn, "wid-1").unwrap();
        status_cancelled(&conn, "wid-1").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert_eq!(row.status, "cancelled");
        assert!(row.finished_at.is_some());
    }

    #[test]
    fn get_status_not_found() {
        let conn = setup_db();
        let row = get_status(&conn, "nonexistent").unwrap();
        assert!(row.is_none());
    }

    #[test]
    fn get_statuses_for_pipeline_groups_correctly() {
        let conn = setup_db();

        status_pending(
            &conn,
            "knot-rkey1-test",
            "knot",
            "rkey1",
            "did:plc:test",
            "test",
        )
        .unwrap();
        status_pending(
            &conn,
            "knot-rkey1-lint",
            "knot",
            "rkey1",
            "did:plc:test",
            "lint",
        )
        .unwrap();
        status_pending(
            &conn,
            "knot-rkey2-test",
            "knot",
            "rkey2",
            "did:plc:test",
            "test",
        )
        .unwrap();

        let pipeline1 = get_statuses_for_pipeline(&conn, "knot", "rkey1").unwrap();
        assert_eq!(pipeline1.len(), 2);
        assert_eq!(pipeline1[0].workflow_name, "test");
        assert_eq!(pipeline1[1].workflow_name, "lint");

        let pipeline2 = get_statuses_for_pipeline(&conn, "knot", "rkey2").unwrap();
        assert_eq!(pipeline2.len(), 1);
        assert_eq!(pipeline2[0].workflow_name, "test");
    }

    #[test]
    fn get_statuses_for_pipeline_empty() {
        let conn = setup_db();
        let rows = get_statuses_for_pipeline(&conn, "nonexistent", "rkey").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn get_statuses_by_status_filters() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey1", "did:plc:test", "test").unwrap();
        status_pending(&conn, "wid-2", "knot", "rkey2", "did:plc:test", "lint").unwrap();
        status_pending(&conn, "wid-3", "knot", "rkey3", "did:plc:test", "build").unwrap();

        status_running(&conn, "wid-1").unwrap();
        status_running(&conn, "wid-2").unwrap();
        status_success(&conn, "wid-1").unwrap();

        let pending = get_statuses_by_status(&conn, "pending").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].workflow_id, "wid-3");

        let running = get_statuses_by_status(&conn, "running").unwrap();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].workflow_id, "wid-2");

        let success = get_statuses_by_status(&conn, "success").unwrap();
        assert_eq!(success.len(), 1);
        assert_eq!(success[0].workflow_id, "wid-1");
    }

    #[test]
    fn get_all_statuses_empty() {
        let conn = setup_db();
        let rows = get_all_statuses(&conn).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn get_all_statuses_returns_all() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey1", "did:plc:test", "test").unwrap();
        status_pending(&conn, "wid-2", "knot", "rkey2", "did:plc:test", "lint").unwrap();

        let rows = get_all_statuses(&conn).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn count_by_status_values() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey1", "did:plc:test", "a").unwrap();
        status_pending(&conn, "wid-2", "knot", "rkey2", "did:plc:test", "b").unwrap();
        status_pending(&conn, "wid-3", "knot", "rkey3", "did:plc:test", "c").unwrap();

        assert_eq!(count_by_status(&conn, "pending").unwrap(), 3);
        assert_eq!(count_by_status(&conn, "running").unwrap(), 0);

        status_running(&conn, "wid-1").unwrap();
        status_running(&conn, "wid-2").unwrap();

        assert_eq!(count_by_status(&conn, "pending").unwrap(), 1);
        assert_eq!(count_by_status(&conn, "running").unwrap(), 2);

        status_success(&conn, "wid-1").unwrap();

        assert_eq!(count_by_status(&conn, "running").unwrap(), 1);
        assert_eq!(count_by_status(&conn, "success").unwrap(), 1);
    }

    #[test]
    fn started_at_not_set_for_pending() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert!(
            row.started_at.is_none(),
            "pending should have no started_at"
        );
    }

    #[test]
    fn finished_at_not_set_for_running() {
        let conn = setup_db();

        status_pending(&conn, "wid-1", "knot", "rkey", "did:plc:test", "test").unwrap();
        status_running(&conn, "wid-1").unwrap();

        let row = get_status(&conn, "wid-1").unwrap().expect("should exist");
        assert!(row.started_at.is_some(), "running should have started_at");
        assert!(
            row.finished_at.is_none(),
            "running should have no finished_at"
        );
    }

    #[test]
    fn all_terminal_states_set_finished_at() {
        let conn = setup_db();

        // Test each terminal state
        for (wid, terminal_fn) in [
            (
                "wid-success",
                status_success as fn(&Connection, &str) -> rusqlite::Result<()>,
            ),
            (
                "wid-failed",
                status_failed as fn(&Connection, &str) -> rusqlite::Result<()>,
            ),
            (
                "wid-timeout",
                status_timeout as fn(&Connection, &str) -> rusqlite::Result<()>,
            ),
            (
                "wid-cancelled",
                status_cancelled as fn(&Connection, &str) -> rusqlite::Result<()>,
            ),
        ] {
            status_pending(&conn, wid, "knot", "rkey", "did:plc:test", "test").unwrap();
            status_running(&conn, wid).unwrap();
            terminal_fn(&conn, wid).unwrap();

            let row = get_status(&conn, wid).unwrap().expect("should exist");
            assert!(
                row.finished_at.is_some(),
                "{} should have finished_at set",
                wid
            );
        }
    }
}
