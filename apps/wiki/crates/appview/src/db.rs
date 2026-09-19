//! The Turso datastore handle. `acquire()` returns a connection with
//! foreign-key enforcement on, and verified on.
//!
//! The FK default is a build-time choice, not a constant of the engine (turso
//! and stock SQLite both ship it off, `crates/schema/tests/roundtrip.rs`), so
//! the pragma is set and read back per connection rather than assumed.

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

    /// A connection with foreign keys enforced. Fails rather than hand out one
    /// that would accept a dangling reference.
    pub async fn acquire(&self) -> Result<Connection, DbError> {
        let conn = self.inner.connect()?;
        conn.execute("PRAGMA foreign_keys=ON", ()).await?;
        if !foreign_keys_enforced(&conn).await {
            return Err(DbError::ForeignKeysOff);
        }
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

/// Whether foreign keys are actually enforced on `conn` (the pragma read back).
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
    /// The engine would not turn foreign-key enforcement on.
    ForeignKeysOff,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Turso(e) => write!(f, "turso error: {e}"),
            DbError::ForeignKeysOff => write!(f, "foreign keys are not enforced"),
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
    async fn a_connection_refuses_a_dangling_reference() {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let conn = db.acquire().await.expect("acquire");
        assert!(
            foreign_keys_enforced(&conn).await,
            "acquire() handed out a connection without foreign keys"
        );
        let dangling = conn
            .execute(
                "INSERT INTO document (id, context_id, kind, title, slug, path) \
                 VALUES ('d', 'no-such-context', 'document', 'T', 't', 't')",
                (),
            )
            .await;
        assert!(
            dangling.is_err(),
            "a document in a context that does not exist was accepted"
        );
    }
}
