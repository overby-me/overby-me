//! The Turso datastore handle. `acquire()` returns a connection and attempts to
//! enable foreign-key enforcement.
//!
//! FINDING (turso 0.2.2, verified): turso does NOT recognize
//! `PRAGMA foreign_keys` ("Not a valid pragma name") and does NOT enforce
//! foreign keys (a dangling reference inserts cleanly). Plain SQLite (the
//! lossless bridge, `crates/schema/tests/roundtrip.rs`) DOES enforce it once the
//! pragma is set and read back. So on turso today, referential integrity must be
//! upheld at the application layer (the loader inserts in FK order; reads join
//! defensively); the ballot core's off-node replication is the real integrity
//! control regardless. `acquire()` therefore sets the pragma best-effort and
//! `foreign_keys_enforced()` reports the truth so callers can branch. When turso
//! gains FK support, `foreign_keys_enforced()` flips to true and nothing else
//! changes.

use turso::{Builder, Connection, Database};

/// A handle to the Turso database. Cloneable and cheap; each `acquire()` opens
/// a fresh connection.
#[derive(Clone)]
pub struct Db {
    inner: Database,
}

impl Db {
    /// Open (or create) the Turso database at `path` (`:memory:` for tests).
    pub async fn open(path: &str) -> Result<Self, DbError> {
        let inner = Builder::new_local(path).build().await?;
        Ok(Self { inner })
    }

    /// A connection with foreign-key enforcement requested. On SQLite this takes
    /// effect; on turso 0.2.2 the pragma is unsupported and silently a no-op
    /// (tolerated, see the module finding). Never fails on the pragma itself.
    pub async fn acquire(&self) -> Result<Connection, DbError> {
        let conn = self.inner.connect()?;
        // Best-effort: errors here mean the engine does not support the pragma
        // (turso 0.2.2), which is a recorded gap, not a connection failure.
        let _ = conn.execute("PRAGMA foreign_keys=ON", ()).await;
        Ok(conn)
    }

    /// Create the schema on a fresh database: the migrated entity subset
    /// (`wiki_schema::ENTITY_SCHEMA`), this crate's runtime infra tables
    /// (`crate::schema::RUNTIME_DDL`), and the ballot service's board + roster
    /// tables (`crate::ballot::init_ballot_schema`). Guarded so an
    /// already-initialized persistent file (where the plain `CREATE TABLE` entity
    /// DDL would error on re-run) is left untouched; the runtime and ballot DDL
    /// are `IF NOT EXISTS` regardless.
    pub async fn init_schema(&self) -> Result<(), DbError> {
        let conn = self.acquire().await?;
        if !table_exists(&conn, "context").await {
            conn.execute_batch(wiki_schema::ENTITY_SCHEMA).await?;
        }
        conn.execute_batch(crate::schema::RUNTIME_DDL).await?;
        // The ballot service's durable tables (public board + private roster),
        // both IF NOT EXISTS, so they live in the same datastore as the entities.
        crate::ballot::init_ballot_schema(self).await?;
        Ok(())
    }
}

/// Whether a table named `name` already exists (so schema init is idempotent on
/// a persistent file).
async fn table_exists(conn: &Connection, name: &str) -> bool {
    match conn
        .query(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
        )
        .await
    {
        Ok(mut rows) => matches!(rows.next().await, Ok(Some(_))),
        Err(_) => false,
    }
}

/// Whether foreign keys are ACTUALLY enforced on `conn`: true on SQLite once the
/// pragma is set, false on turso 0.2.2 (pragma unsupported). Callers needing a
/// referential-integrity guarantee branch on this instead of assuming it.
pub async fn foreign_keys_enforced(conn: &Connection) -> bool {
    match conn.query("PRAGMA foreign_keys", ()).await {
        Ok(mut rows) => matches!(
            rows.next().await,
            Ok(Some(row)) if row.get::<i64>(0).unwrap_or(0) == 1
        ),
        Err(_) => false,
    }
}

#[derive(Debug)]
pub enum DbError {
    Turso(turso::Error),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Turso(e) => write!(f, "turso error: {e}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<turso::Error> for DbError {
    fn from(e: turso::Error) -> Self {
        DbError::Turso(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn acquire_succeeds_and_reports_fk_status() {
        let db = Db::open(":memory:").await.expect("open");
        // acquire() never fails on the (unsupported-on-turso) pragma.
        let conn = db.acquire().await.expect("acquire");
        // FINDING pinned: turso 0.2.2 does not enforce foreign keys, so the
        // status reports false. This assertion flips when turso adds FK support,
        // which is the signal to drop the app-layer FK-ordering workaround.
        assert!(
            !foreign_keys_enforced(&conn).await,
            "turso 0.2.2 is expected NOT to enforce foreign keys; if this now \
             enforces, turso gained FK support and the loader's app-layer \
             ordering can be revisited"
        );
    }
}
