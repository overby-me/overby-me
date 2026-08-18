//! Repository tracking queries.
//!
//! Manages the `repos` table, which tracks repositories that this spindle
//! instance watches for pipeline events. When a `sh.tangled.repo` record
//! points at this spindle's hostname, the repo is added here.

use rusqlite::{Connection, OptionalExtension, params};

/// A tracked repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Database row ID.
    pub id: i64,
    /// DID of the repo owner (e.g. `"did:plc:abc123"`).
    pub did: String,
    /// Repository name.
    pub name: String,
    /// Knot server hostname.
    pub knot: String,
    /// When the repo was added.
    pub created_at: String,
}

/// Add a repository to the watch list.
///
/// If a repo with the same `(did, name)` already exists, this is a no-op
/// (using `INSERT OR IGNORE`).
///
/// Returns the row ID of the inserted (or existing) repo.
pub fn add_repo(conn: &Connection, did: &str, name: &str, knot: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO repos (did, name, knot) VALUES (?1, ?2, ?3)",
        params![did, name, knot],
    )?;

    // Return the row ID (works for both new inserts and existing rows)
    let id: i64 = conn.query_row(
        "SELECT id FROM repos WHERE did = ?1 AND name = ?2",
        params![did, name],
        |row| row.get(0),
    )?;

    Ok(id)
}

/// Get a repository by owner DID and name.
pub fn get_repo(conn: &Connection, did: &str, name: &str) -> rusqlite::Result<Option<Repo>> {
    conn.query_row(
        "SELECT id, did, name, knot, created_at FROM repos WHERE did = ?1 AND name = ?2",
        params![did, name],
        |row| {
            Ok(Repo {
                id: row.get(0)?,
                did: row.get(1)?,
                name: row.get(2)?,
                knot: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )
    .optional()
}

/// Get a repository by its database row ID.
pub fn get_repo_by_id(conn: &Connection, id: i64) -> rusqlite::Result<Option<Repo>> {
    conn.query_row(
        "SELECT id, did, name, knot, created_at FROM repos WHERE id = ?1",
        params![id],
        |row| {
            Ok(Repo {
                id: row.get(0)?,
                did: row.get(1)?,
                name: row.get(2)?,
                knot: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )
    .optional()
}

/// Get all repositories tracked on a specific knot server.
pub fn get_repos_by_knot(conn: &Connection, knot: &str) -> rusqlite::Result<Vec<Repo>> {
    let mut stmt = conn
        .prepare("SELECT id, did, name, knot, created_at FROM repos WHERE knot = ?1 ORDER BY id")?;

    let repos = stmt
        .query_map(params![knot], |row| {
            Ok(Repo {
                id: row.get(0)?,
                did: row.get(1)?,
                name: row.get(2)?,
                knot: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(repos)
}

/// Get all repositories owned by a specific DID.
pub fn get_repos_by_did(conn: &Connection, did: &str) -> rusqlite::Result<Vec<Repo>> {
    let mut stmt = conn
        .prepare("SELECT id, did, name, knot, created_at FROM repos WHERE did = ?1 ORDER BY id")?;

    let repos = stmt
        .query_map(params![did], |row| {
            Ok(Repo {
                id: row.get(0)?,
                did: row.get(1)?,
                name: row.get(2)?,
                knot: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(repos)
}

/// Get all tracked repositories.
pub fn get_all_repos(conn: &Connection) -> rusqlite::Result<Vec<Repo>> {
    let mut stmt = conn.prepare("SELECT id, did, name, knot, created_at FROM repos ORDER BY id")?;

    let repos = stmt
        .query_map([], |row| {
            Ok(Repo {
                id: row.get(0)?,
                did: row.get(1)?,
                name: row.get(2)?,
                knot: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(repos)
}

/// Remove a repository from the watch list.
///
/// Returns `true` if a row was deleted, `false` if the repo wasn't found.
pub fn remove_repo(conn: &Connection, did: &str, name: &str) -> rusqlite::Result<bool> {
    let deleted = conn.execute(
        "DELETE FROM repos WHERE did = ?1 AND name = ?2",
        params![did, name],
    )?;
    Ok(deleted > 0)
}

/// Get all distinct knot hostnames from tracked repos.
///
/// Used by the knot event consumer to know which knots to subscribe to.
pub fn get_all_knots(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT knot FROM repos ORDER BY knot")?;

    let knots = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(knots)
}

#[cfg(test)]
mod tests {
    use crate::migrations;
    use crate::repos::*;

    fn setup_db() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        migrations::migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn add_and_get_repo() {
        let conn = setup_db();

        let id = add_repo(&conn, "did:plc:alice", "my-repo", "knot1.example.com").unwrap();
        assert!(id > 0);

        let repo = get_repo(&conn, "did:plc:alice", "my-repo")
            .unwrap()
            .expect("repo should exist");
        assert_eq!(repo.id, id);
        assert_eq!(repo.did, "did:plc:alice");
        assert_eq!(repo.name, "my-repo");
        assert_eq!(repo.knot, "knot1.example.com");
    }

    #[test]
    fn add_repo_is_idempotent() {
        let conn = setup_db();

        let id1 = add_repo(&conn, "did:plc:alice", "my-repo", "knot1.example.com").unwrap();
        let id2 = add_repo(&conn, "did:plc:alice", "my-repo", "knot1.example.com").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn get_repo_not_found() {
        let conn = setup_db();

        let repo = get_repo(&conn, "did:plc:nobody", "nope").unwrap();
        assert!(repo.is_none());
    }

    #[test]
    fn get_repo_by_id_found() {
        let conn = setup_db();

        let id = add_repo(&conn, "did:plc:alice", "repo1", "knot.example.com").unwrap();
        let repo = get_repo_by_id(&conn, id).unwrap().expect("should exist");
        assert_eq!(repo.did, "did:plc:alice");
        assert_eq!(repo.name, "repo1");
    }

    #[test]
    fn get_repo_by_id_not_found() {
        let conn = setup_db();

        let repo = get_repo_by_id(&conn, 9999).unwrap();
        assert!(repo.is_none());
    }

    #[test]
    fn test_get_repos_by_knot() {
        let conn = setup_db();

        add_repo(&conn, "did:plc:alice", "repo1", "knot-a.example.com").unwrap();
        add_repo(&conn, "did:plc:bob", "repo2", "knot-a.example.com").unwrap();
        add_repo(&conn, "did:plc:alice", "repo3", "knot-b.example.com").unwrap();

        let repos_a = get_repos_by_knot(&conn, "knot-a.example.com").unwrap();
        assert_eq!(repos_a.len(), 2);
        assert_eq!(repos_a[0].name, "repo1");
        assert_eq!(repos_a[1].name, "repo2");

        let repos_b = get_repos_by_knot(&conn, "knot-b.example.com").unwrap();
        assert_eq!(repos_b.len(), 1);
        assert_eq!(repos_b[0].name, "repo3");

        let repos_c = get_repos_by_knot(&conn, "knot-c.example.com").unwrap();
        assert!(repos_c.is_empty());
    }

    #[test]
    fn test_get_repos_by_did() {
        let conn = setup_db();

        add_repo(&conn, "did:plc:alice", "repo1", "knot.example.com").unwrap();
        add_repo(&conn, "did:plc:alice", "repo2", "knot.example.com").unwrap();
        add_repo(&conn, "did:plc:bob", "repo3", "knot.example.com").unwrap();

        let alice_repos = get_repos_by_did(&conn, "did:plc:alice").unwrap();
        assert_eq!(alice_repos.len(), 2);

        let bob_repos = get_repos_by_did(&conn, "did:plc:bob").unwrap();
        assert_eq!(bob_repos.len(), 1);
    }

    #[test]
    fn get_all_repos_empty() {
        let conn = setup_db();
        let repos = get_all_repos(&conn).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn get_all_repos_returns_all() {
        let conn = setup_db();

        add_repo(&conn, "did:plc:alice", "repo1", "knot.example.com").unwrap();
        add_repo(&conn, "did:plc:bob", "repo2", "knot2.example.com").unwrap();

        let repos = get_all_repos(&conn).unwrap();
        assert_eq!(repos.len(), 2);
    }

    #[test]
    fn remove_repo_existing() {
        let conn = setup_db();

        add_repo(&conn, "did:plc:alice", "repo1", "knot.example.com").unwrap();
        assert!(remove_repo(&conn, "did:plc:alice", "repo1").unwrap());

        let repo = get_repo(&conn, "did:plc:alice", "repo1").unwrap();
        assert!(repo.is_none());
    }

    #[test]
    fn remove_repo_not_found() {
        let conn = setup_db();
        assert!(!remove_repo(&conn, "did:plc:nobody", "nope").unwrap());
    }

    #[test]
    fn get_all_knots_distinct() {
        let conn = setup_db();

        add_repo(&conn, "did:plc:alice", "repo1", "knot-a.example.com").unwrap();
        add_repo(&conn, "did:plc:bob", "repo2", "knot-a.example.com").unwrap();
        add_repo(&conn, "did:plc:alice", "repo3", "knot-b.example.com").unwrap();

        let knots = get_all_knots(&conn).unwrap();
        assert_eq!(knots, vec!["knot-a.example.com", "knot-b.example.com"]);
    }

    #[test]
    fn get_all_knots_empty() {
        let conn = setup_db();
        let knots = get_all_knots(&conn).unwrap();
        assert!(knots.is_empty());
    }
}
