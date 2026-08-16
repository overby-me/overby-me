//! SQLite database layer for `tangled-spindle-nix`.
//!
//! This crate provides persistent storage for the spindle runner using SQLite
//! (via `rusqlite` with WAL mode). It manages:
//!
//! - **Repos** — Tracked repositories that this spindle watches.
//! - **Members** — Spindle membership records (DIDs allowed to use this spindle).
//! - **Events** — Pipeline event log (for WebSocket backfill on `/events`).
//! - **Status** — Workflow execution status tracking.
//! - **Cursor** — Jetstream cursor persistence for reconnection.
//! - **Knots** — Knot server tracking with per-knot cursors.
//!
//! # Schema
//!
//! Migrations are embedded via `include_str!` and applied automatically on
//! database open. The schema matches the upstream Go spindle's SQLite tables.
//!
//! # Usage
//!
//! ```no_run
//! use spindle_db::Database;
//!
//! let db = Database::open("spindle.db").expect("failed to open database");
//! // The database is now ready to use — migrations have been applied.
//! ```
//!
//! # Thread Safety
//!
//! The [`Database`] wrapper holds a `Mutex<Connection>` so it can be shared
//! across async tasks via `Arc<Database>`. Each operation acquires the lock
//! for the duration of the call.

pub mod events;
pub mod members;
pub mod migrations;
pub mod repos;
pub mod status;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

/// Errors that can occur in database operations.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// A SQLite error occurred.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A migration error occurred.
    #[error("migration error: {0}")]
    Migration(#[from] migrations::MigrationError),

    /// The database lock was poisoned (a thread panicked while holding it).
    #[error("database lock poisoned")]
    LockPoisoned,
}

/// Thread-safe database handle.
///
/// Wraps a `rusqlite::Connection` in a `Mutex` so it can be shared across
/// async tasks via `Arc<Database>`. All public methods acquire the lock,
/// perform the operation, and release it.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use spindle_db::Database;
///
/// let db = Arc::new(Database::open("spindle.db").unwrap());
///
/// // Use from multiple tasks:
/// let db2 = Arc::clone(&db);
/// // tokio::spawn(async move { db2.add_repo(...) });
/// ```
pub struct Database {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// Open (or create) a SQLite database at the given path and apply migrations.
    ///
    /// The database is configured with:
    /// - WAL journal mode for concurrent readers.
    /// - Foreign keys enabled.
    /// - Busy timeout of 5 seconds.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let mut conn = Connection::open(path)?;
        Self::configure(&conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for testing).
    ///
    /// Applies all migrations automatically.
    pub fn open_in_memory() -> Result<Self, DbError> {
        let mut conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Configure SQLite pragmas for optimal performance.
    fn configure(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;",
        )
    }

    /// Acquire the connection lock.
    ///
    /// Returns a `MutexGuard<Connection>` that auto-releases on drop.
    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, DbError> {
        self.conn.lock().map_err(|_| DbError::LockPoisoned)
    }

    // -----------------------------------------------------------------------
    // Repos
    // -----------------------------------------------------------------------

    /// Add a repository to the watch list.
    ///
    /// See [`repos::add_repo`] for details.
    pub fn add_repo(&self, did: &str, name: &str, knot: &str) -> Result<i64, DbError> {
        Ok(repos::add_repo(&*self.conn()?, did, name, knot)?)
    }

    /// Get a repository by owner DID and name.
    pub fn get_repo(&self, did: &str, name: &str) -> Result<Option<repos::Repo>, DbError> {
        Ok(repos::get_repo(&*self.conn()?, did, name)?)
    }

    /// Get a repository by its database row ID.
    pub fn get_repo_by_id(&self, id: i64) -> Result<Option<repos::Repo>, DbError> {
        Ok(repos::get_repo_by_id(&*self.conn()?, id)?)
    }

    /// Get all repositories tracked on a specific knot server.
    pub fn get_repos_by_knot(&self, knot: &str) -> Result<Vec<repos::Repo>, DbError> {
        Ok(repos::get_repos_by_knot(&*self.conn()?, knot)?)
    }

    /// Get all repositories owned by a specific DID.
    pub fn get_repos_by_did(&self, did: &str) -> Result<Vec<repos::Repo>, DbError> {
        Ok(repos::get_repos_by_did(&*self.conn()?, did)?)
    }

    /// Get all tracked repositories.
    pub fn get_all_repos(&self) -> Result<Vec<repos::Repo>, DbError> {
        Ok(repos::get_all_repos(&*self.conn()?)?)
    }

    /// Remove a repository from the watch list.
    pub fn remove_repo(&self, did: &str, name: &str) -> Result<bool, DbError> {
        Ok(repos::remove_repo(&*self.conn()?, did, name)?)
    }

    /// Get all distinct knot hostnames from tracked repos.
    pub fn get_repo_knots(&self) -> Result<Vec<String>, DbError> {
        Ok(repos::get_all_knots(&*self.conn()?)?)
    }

    // -----------------------------------------------------------------------
    // Members
    // -----------------------------------------------------------------------

    /// Add a spindle member with the `"member"` role.
    pub fn add_spindle_member(&self, did: &str) -> Result<(), DbError> {
        Ok(members::add_spindle_member(&*self.conn()?, did)?)
    }

    /// Add a spindle owner.
    pub fn add_spindle_owner(&self, did: &str) -> Result<(), DbError> {
        Ok(members::add_spindle_owner(&*self.conn()?, did)?)
    }

    /// Remove a spindle member by DID.
    pub fn remove_member(&self, did: &str) -> Result<bool, DbError> {
        Ok(members::remove_member(&*self.conn()?, did)?)
    }

    /// Get a spindle member by DID.
    pub fn get_member(&self, did: &str) -> Result<Option<members::Member>, DbError> {
        Ok(members::get_member(&*self.conn()?, did)?)
    }

    /// Get all spindle members.
    pub fn get_all_members(&self) -> Result<Vec<members::Member>, DbError> {
        Ok(members::get_all_members(&*self.conn()?)?)
    }

    /// Get all spindle members with a specific role.
    pub fn get_members_by_role(&self, role: &str) -> Result<Vec<members::Member>, DbError> {
        Ok(members::get_members_by_role(&*self.conn()?, role)?)
    }

    /// Check whether a DID is a spindle member (any role).
    pub fn is_member(&self, did: &str) -> Result<bool, DbError> {
        Ok(members::is_member(&*self.conn()?, did)?)
    }

    // -----------------------------------------------------------------------
    // DIDs (Jetstream watch list)
    // -----------------------------------------------------------------------

    /// Add a DID to the Jetstream watch list.
    pub fn add_did(&self, did: &str) -> Result<(), DbError> {
        Ok(members::add_did(&*self.conn()?, did)?)
    }

    /// Remove a DID from the Jetstream watch list.
    pub fn remove_did(&self, did: &str) -> Result<bool, DbError> {
        Ok(members::remove_did(&*self.conn()?, did)?)
    }

    /// Get all DIDs on the Jetstream watch list.
    pub fn get_all_dids(&self) -> Result<Vec<String>, DbError> {
        Ok(members::get_all_dids(&*self.conn()?)?)
    }

    /// Check whether a DID is on the Jetstream watch list.
    pub fn has_did(&self, did: &str) -> Result<bool, DbError> {
        Ok(members::has_did(&*self.conn()?, did)?)
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    /// Insert a new pipeline event.
    pub fn insert_event(&self, params: &events::InsertEventParams) -> Result<i64, DbError> {
        Ok(events::insert_event(&*self.conn()?, params)?)
    }

    /// Get all events after the given cursor (unix nanos).
    pub fn get_events_after(&self, cursor: i64) -> Result<Vec<events::Event>, DbError> {
        Ok(events::get_events_after(&*self.conn()?, cursor)?)
    }

    /// Get all events.
    pub fn get_all_events(&self) -> Result<Vec<events::Event>, DbError> {
        Ok(events::get_all_events(&*self.conn()?)?)
    }

    /// Get the latest event ID.
    pub fn get_latest_event_id(&self) -> Result<Option<i64>, DbError> {
        Ok(events::get_latest_event_id(&*self.conn()?)?)
    }

    /// Get a single event by ID.
    pub fn get_event(&self, id: i64) -> Result<Option<events::Event>, DbError> {
        Ok(events::get_event(&*self.conn()?, id)?)
    }

    /// Get the total number of stored events.
    pub fn event_count(&self) -> Result<i64, DbError> {
        Ok(events::event_count(&*self.conn()?)?)
    }

    // -----------------------------------------------------------------------
    // Jetstream cursor
    // -----------------------------------------------------------------------

    /// Save the Jetstream cursor (last processed event timestamp in microseconds).
    pub fn save_last_time_us(&self, time_us: i64) -> Result<(), DbError> {
        Ok(events::save_last_time_us(&*self.conn()?, time_us)?)
    }

    /// Get the Jetstream cursor.
    pub fn get_last_time_us(&self) -> Result<i64, DbError> {
        Ok(events::get_last_time_us(&*self.conn()?)?)
    }

    // -----------------------------------------------------------------------
    // Knots
    // -----------------------------------------------------------------------

    /// Add a knot to the tracking table.
    pub fn add_knot(&self, knot: &str) -> Result<(), DbError> {
        Ok(events::add_knot(&*self.conn()?, knot)?)
    }

    /// Remove a knot from the tracking table.
    pub fn remove_knot(&self, knot: &str) -> Result<bool, DbError> {
        Ok(events::remove_knot(&*self.conn()?, knot)?)
    }

    /// Update the cursor for a knot.
    pub fn update_knot_cursor(&self, knot: &str, cursor: &str) -> Result<(), DbError> {
        Ok(events::update_knot_cursor(&*self.conn()?, knot, cursor)?)
    }

    /// Get the cursor for a knot.
    pub fn get_knot_cursor(&self, knot: &str) -> Result<Option<String>, DbError> {
        Ok(events::get_knot_cursor(&*self.conn()?, knot)?)
    }

    /// Get all tracked knots with their cursors.
    pub fn get_all_knots(&self) -> Result<Vec<events::KnotCursor>, DbError> {
        Ok(events::get_all_knots(&*self.conn()?)?)
    }

    /// Get all tracked knot hostnames.
    pub fn get_knot_names(&self) -> Result<Vec<String>, DbError> {
        Ok(events::get_knot_names(&*self.conn()?)?)
    }

    // -----------------------------------------------------------------------
    // Workflow status
    // -----------------------------------------------------------------------

    /// Create a new workflow status record in `pending` state.
    pub fn status_pending(
        &self,
        workflow_id: &str,
        pipeline_knot: &str,
        pipeline_rkey: &str,
        repo_did: &str,
        workflow_name: &str,
    ) -> Result<(), DbError> {
        Ok(status::status_pending(
            &*self.conn()?,
            workflow_id,
            pipeline_knot,
            pipeline_rkey,
            repo_did,
            workflow_name,
        )?)
    }

    /// Transition a workflow to `running` state.
    pub fn status_running(&self, workflow_id: &str) -> Result<(), DbError> {
        Ok(status::status_running(&*self.conn()?, workflow_id)?)
    }

    /// Transition a workflow to `success` state.
    pub fn status_success(&self, workflow_id: &str) -> Result<(), DbError> {
        Ok(status::status_success(&*self.conn()?, workflow_id)?)
    }

    /// Transition a workflow to `failed` state.
    pub fn status_failed(&self, workflow_id: &str) -> Result<(), DbError> {
        Ok(status::status_failed(&*self.conn()?, workflow_id)?)
    }

    /// Transition a workflow to `timeout` state.
    pub fn status_timeout(&self, workflow_id: &str) -> Result<(), DbError> {
        Ok(status::status_timeout(&*self.conn()?, workflow_id)?)
    }

    /// Transition a workflow to `cancelled` state.
    pub fn status_cancelled(&self, workflow_id: &str) -> Result<(), DbError> {
        Ok(status::status_cancelled(&*self.conn()?, workflow_id)?)
    }

    /// Cancel all orphaned workflows still in `pending` or `running` state.
    ///
    /// Should be called on startup to clean up leftovers from a previous crash.
    pub fn cancel_orphaned_workflows(&self) -> Result<usize, DbError> {
        Ok(status::cancel_orphaned(&*self.conn()?)?)
    }

    /// Get the status record for a workflow.
    pub fn get_status(
        &self,
        workflow_id: &str,
    ) -> Result<Option<status::WorkflowStatusRow>, DbError> {
        Ok(status::get_status(&*self.conn()?, workflow_id)?)
    }

    /// Get all workflow status records for a specific pipeline.
    pub fn get_statuses_for_pipeline(
        &self,
        pipeline_knot: &str,
        pipeline_rkey: &str,
    ) -> Result<Vec<status::WorkflowStatusRow>, DbError> {
        Ok(status::get_statuses_for_pipeline(
            &*self.conn()?,
            pipeline_knot,
            pipeline_rkey,
        )?)
    }

    /// Get all workflow status records for a specific repo DID.
    pub fn get_statuses_for_repo(
        &self,
        repo_did: &str,
    ) -> Result<Vec<status::WorkflowStatusRow>, DbError> {
        Ok(status::get_statuses_for_repo(&*self.conn()?, repo_did)?)
    }

    /// Get all workflow status records with a specific status value.
    pub fn get_statuses_by_status(
        &self,
        status_value: &str,
    ) -> Result<Vec<status::WorkflowStatusRow>, DbError> {
        Ok(status::get_statuses_by_status(
            &*self.conn()?,
            status_value,
        )?)
    }

    /// Get all workflow status records.
    pub fn get_all_statuses(&self) -> Result<Vec<status::WorkflowStatusRow>, DbError> {
        Ok(status::get_all_statuses(&*self.conn()?)?)
    }

    /// Count workflows in a specific status.
    pub fn count_by_status(&self, status_value: &str) -> Result<i64, DbError> {
        Ok(status::count_by_status(&*self.conn()?, status_value)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_succeeds() {
        let db = Database::open_in_memory().unwrap();
        // Verify the database is usable
        assert!(db.get_all_repos().unwrap().is_empty());
        assert!(db.get_all_members().unwrap().is_empty());
        assert!(db.get_all_events().unwrap().is_empty());
        assert!(db.get_all_statuses().unwrap().is_empty());
        assert_eq!(db.get_last_time_us().unwrap(), 0);
    }

    #[test]
    fn database_wrapper_round_trip() {
        let db = Database::open_in_memory().unwrap();

        // Repos
        let repo_id = db
            .add_repo("did:plc:alice", "my-repo", "knot.example.com")
            .unwrap();
        assert!(repo_id > 0);
        let repo = db.get_repo("did:plc:alice", "my-repo").unwrap().unwrap();
        assert_eq!(repo.knot, "knot.example.com");

        // Members
        db.add_spindle_owner("did:plc:owner").unwrap();
        db.add_spindle_member("did:plc:alice").unwrap();
        assert!(db.is_member("did:plc:owner").unwrap());
        assert!(db.is_member("did:plc:alice").unwrap());
        assert!(!db.is_member("did:plc:nobody").unwrap());

        // DIDs
        db.add_did("did:plc:owner").unwrap();
        db.add_did("did:plc:alice").unwrap();
        assert!(db.has_did("did:plc:owner").unwrap());
        let dids = db.get_all_dids().unwrap();
        assert_eq!(dids.len(), 2);

        // Events
        let eid = db
            .insert_event(&events::InsertEventParams {
                rkey: "test-rkey",
                nsid: "sh.tangled.pipeline.status",
                payload: r#"{"test":true}"#,
                created: 1000,
            })
            .unwrap();
        let event = db.get_event(eid).unwrap().unwrap();
        assert_eq!(event.nsid, "sh.tangled.pipeline.status");
        assert_eq!(db.event_count().unwrap(), 1);

        // Jetstream cursor
        db.save_last_time_us(123_456_789).unwrap();
        assert_eq!(db.get_last_time_us().unwrap(), 123_456_789);

        // Knots
        db.add_knot("knot.example.com").unwrap();
        db.update_knot_cursor("knot.example.com", "cursor-1")
            .unwrap();
        assert_eq!(
            db.get_knot_cursor("knot.example.com").unwrap().as_deref(),
            Some("cursor-1")
        );

        // Status
        db.status_pending("wid-1", "knot", "rkey", "did:plc:test", "test")
            .unwrap();
        db.status_running("wid-1").unwrap();
        db.status_success("wid-1").unwrap();
        let st = db.get_status("wid-1").unwrap().unwrap();
        assert_eq!(st.status, "success");
        assert!(st.finished_at.is_some());
    }

    #[test]
    fn debug_impl() {
        let db = Database::open_in_memory().unwrap();
        let debug = format!("{:?}", db);
        assert!(debug.contains("Database"));
    }
}
