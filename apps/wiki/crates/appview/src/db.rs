//! The Turso datastore handle. `acquire()` returns a connection with
//! foreign-key enforcement on, and verified on.
//!
//! The FK default is a build-time choice, not a constant of the engine (turso
//! and stock SQLite both ship it off, `crates/schema/tests/roundtrip.rs`), so
//! the pragma is set and read back per connection rather than assumed.

use std::sync::Arc;
use std::time::Duration;
use turso::{Builder, Connection, Database};

/// How long a write waits its turn. A write takes milliseconds, so this is only
/// reached by a burst far past anything a meeting produces.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// A handle to the Turso database. Cloneable and cheap; each `acquire()` opens
/// a fresh connection.
#[derive(Clone)]
pub struct Db {
    inner: Database,
    writers: Arc<tokio::sync::Mutex<()>>,
}

impl Db {
    /// Open (or create) the Turso database at `path` (`:memory:` for tests).
    pub async fn open(path: &str) -> Result<Self, DbError> {
        let inner = Builder::new_local(path).build().await?;
        Ok(Self {
            inner,
            writers: Arc::default(),
        })
    }

    /// A place in the queue of writers, for a write a whole room makes at once
    /// (a ballot). The busy timeout alone gets every such write through, but by
    /// polling, in no order: this waits asleep, first come first served.
    pub async fn write_turn(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.writers.clone().lock_owned().await
    }

    /// A connection with foreign keys enforced. Fails rather than hand out one
    /// that would accept a dangling reference.
    pub async fn acquire(&self) -> Result<Connection, DbError> {
        let conn = self.inner.connect()?;
        // The engine's default is to fail a write at once while another
        // connection holds the write lock, which a room tapping together does
        // to almost every write.
        conn.busy_timeout(BUSY_TIMEOUT)?;
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
        if table_exists(&conn, "context").await {
            let found = schema_version(&conn).await?;
            if found != SCHEMA_VERSION {
                return Err(DbError::SchemaVersion {
                    found,
                    expected: SCHEMA_VERSION,
                });
            }
        } else {
            conn.execute_batch(wiki_schema::ENTITY_SCHEMA).await?;
            conn.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION}"), ())
                .await?;
        }
        conn.execute_batch(crate::schema::RUNTIME_DDL).await?;
        conn.execute_batch(crate::speak::SPEAK_DDL).await?;
        conn.execute_batch(crate::projector::PROJECTOR_DDL).await?;
        conn.execute_batch(crate::blob::BLOB_DDL).await?;
        conn.execute_batch(crate::push::PUSH_DDL).await?;
        conn.execute_batch(crate::feedback::FEEDBACK_DDL).await?;
        // The ballot service's durable tables (public board + private roster),
        // both IF NOT EXISTS, so they live in the same datastore as the entities.
        crate::ballot::init_ballot_schema(self).await?;
        Ok(())
    }
}

/// The version of the schema this binary reads and writes. Bump it with any
/// change to a table that an existing file would already hold in its old shape:
/// `wiki_domain_types::DDL`, and the `IF NOT EXISTS` tables too, since that
/// clause keeps an old table as it is.
///
/// There are no migrations yet, and the entity tables are plain `CREATE TABLE`,
/// so a file made by an older binary keeps its old columns. Without this the
/// process would start and then fail one query at a time; with it, it refuses
/// to start and says why.
pub const SCHEMA_VERSION: i64 = 5;

async fn schema_version(conn: &Connection) -> Result<i64, DbError> {
    let mut rows = conn.query("PRAGMA user_version", ()).await?;
    match rows.next().await? {
        Some(row) => Ok(row.get::<i64>(0)?),
        None => Ok(0),
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
    /// The file was made for another version of the entity schema.
    SchemaVersion {
        found: i64,
        expected: i64,
    },
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Turso(e) => write!(f, "turso error: {e}"),
            DbError::ForeignKeysOff => write!(f, "foreign keys are not enforced"),
            DbError::SchemaVersion { found, expected } => write!(
                f,
                "the datastore holds schema version {found}, this binary needs {expected}; \
                 there are no migrations yet, so rebuild it from the migration pipeline"
            ),
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

    /// Without a busy timeout the engine fails a write the moment another
    /// connection holds the lock: measured here before the fix, 371 of 400.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn writers_on_many_connections_wait_their_turn() {
        let db = Db::open(":memory:").await.expect("open");
        let conn = db.acquire().await.expect("conn");
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", ())
            .await
            .expect("ddl");

        let writers = (0..16).map(|_| {
            let db = db.clone();
            tokio::spawn(async move {
                for _ in 0..25 {
                    let conn = db.acquire().await?;
                    conn.execute("BEGIN IMMEDIATE", ()).await?;
                    let mut rows = conn
                        .query("SELECT coalesce(max(id), 0) + 1 FROM t", ())
                        .await?;
                    let next: i64 = rows.next().await?.expect("row").get(0)?;
                    drop(rows);
                    // Give the others every chance to collide.
                    tokio::task::yield_now().await;
                    conn.execute("INSERT INTO t (id) VALUES (?1)", [next])
                        .await?;
                    conn.execute("COMMIT", ()).await?;
                }
                Ok::<_, DbError>(())
            })
        });
        for writer in writers.collect::<Vec<_>>() {
            writer.await.expect("join").expect("a write was refused");
        }
        let mut rows = conn.query("SELECT count(*) FROM t", ()).await.expect("q");
        let written: i64 = rows
            .next()
            .await
            .expect("next")
            .expect("row")
            .get(0)
            .expect("n");
        assert_eq!(written, 400);
    }

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

    #[tokio::test(flavor = "current_thread")]
    async fn a_file_from_another_schema_version_is_refused_not_served() {
        let dir = std::env::temp_dir().join(format!("appview-{}", crate::util::random_token(8)));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("old.db");
        let path = path.to_str().expect("utf-8 path");

        let db = Db::open(path).await.expect("open");
        db.init_schema().await.expect("a fresh file initializes");
        db.init_schema()
            .await
            .expect("and the same binary reopens it");

        let conn = db.acquire().await.expect("conn");
        conn.execute("PRAGMA user_version = 0", ())
            .await
            .expect("age the file");
        drop(conn);
        match db.init_schema().await {
            Err(DbError::SchemaVersion { found: 0, expected }) => {
                assert_eq!(expected, SCHEMA_VERSION);
            }
            other => panic!("an old file was accepted: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
