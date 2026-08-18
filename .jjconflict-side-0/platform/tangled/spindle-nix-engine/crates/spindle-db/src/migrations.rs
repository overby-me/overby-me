//! Schema migration management for the spindle SQLite database.
//!
//! Migrations are embedded at compile time via `include_str!` and applied
//! automatically when the database is opened. A `schema_version` pragma
//! tracks which migrations have been applied.

use rusqlite::Connection;

/// Embedded SQL migration scripts, in order.
///
/// Each entry is `(version, name, sql)`. Versions are 1-indexed and must
/// be applied sequentially.
const MIGRATIONS: &[(u32, &str, &str)] = &[
    (1, "initial", include_str!("migrations/001_initial.sql")),
    (
        2,
        "add_repo_did",
        include_str!("migrations/002_add_repo_did.sql"),
    ),
    (
        3,
        "events_appview_compat",
        include_str!("migrations/003_events_appview_compat.sql"),
    ),
];

/// Errors that can occur during migration.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// A SQLite error occurred.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// A migration script failed.
    #[error("migration {version} ({name}) failed: {source}")]
    MigrationFailed {
        version: u32,
        name: String,
        source: rusqlite::Error,
    },
}

/// Return the current schema version from the database.
///
/// Uses SQLite's `user_version` pragma, which defaults to 0 for new databases.
pub fn current_version(conn: &Connection) -> Result<u32, MigrationError> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version)
}

/// Apply all pending migrations to the database.
///
/// Migrations are applied inside a transaction so that a failure rolls back
/// cleanly. After all pending migrations succeed, the `user_version` pragma
/// is updated to the latest version.
///
/// Returns the number of migrations that were applied.
pub fn migrate(conn: &mut Connection) -> Result<usize, MigrationError> {
    let current = current_version(conn)?;
    let mut applied = 0;

    for &(version, name, sql) in MIGRATIONS {
        if version <= current {
            continue;
        }

        tracing::info!(version, name, "applying migration");

        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .map_err(|e| MigrationError::MigrationFailed {
                version,
                name: name.to_owned(),
                source: e,
            })?;
        tx.pragma_update(None, "user_version", version)?;
        tx.commit()?;

        applied += 1;
    }

    if applied > 0 {
        tracing::info!(
            applied,
            version = MIGRATIONS.last().map(|m| m.0).unwrap_or(0),
            "migrations complete"
        );
    } else {
        tracing::debug!(version = current, "database schema is up to date");
    }

    Ok(applied)
}

/// Return the latest migration version available.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map(|m| m.0).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn
    }

    #[test]
    fn fresh_database_has_version_zero() {
        let conn = memory_db();
        assert_eq!(current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn migrate_applies_all_migrations() {
        let mut conn = memory_db();
        let applied = migrate(&mut conn).unwrap();
        assert_eq!(applied, MIGRATIONS.len());
        assert_eq!(current_version(&conn).unwrap(), latest_version());
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = memory_db();

        let first = migrate(&mut conn).unwrap();
        assert!(first > 0);

        let second = migrate(&mut conn).unwrap();
        assert_eq!(second, 0);

        assert_eq!(current_version(&conn).unwrap(), latest_version());
    }

    #[test]
    fn tables_exist_after_migration() {
        let mut conn = memory_db();
        migrate(&mut conn).unwrap();

        // Verify all expected tables exist by querying sqlite_master
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"repos".to_owned()), "missing repos table");
        assert!(
            tables.contains(&"spindle_members".to_owned()),
            "missing spindle_members table"
        );
        assert!(tables.contains(&"dids".to_owned()), "missing dids table");
        assert!(
            tables.contains(&"events".to_owned()),
            "missing events table"
        );
        assert!(
            tables.contains(&"workflow_status".to_owned()),
            "missing workflow_status table"
        );
        assert!(
            tables.contains(&"last_time_us".to_owned()),
            "missing last_time_us table"
        );
        assert!(tables.contains(&"knots".to_owned()), "missing knots table");
    }

    #[test]
    fn last_time_us_singleton_exists() {
        let mut conn = memory_db();
        migrate(&mut conn).unwrap();

        let time_us: i64 = conn
            .query_row("SELECT time_us FROM last_time_us WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(time_us, 0);
    }

    #[test]
    fn latest_version_matches_migration_count() {
        assert_eq!(latest_version(), MIGRATIONS.len() as u32);
    }
}
