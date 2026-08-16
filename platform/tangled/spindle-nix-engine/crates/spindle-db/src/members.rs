//! Spindle member management and DID tracking queries.
//!
//! Manages the `spindle_members` and `dids` tables:
//!
//! - **`spindle_members`** — DIDs allowed to trigger pipelines on this spindle,
//!   populated from `sh.tangled.spindle.member` records via Jetstream ingestion.
//! - **`dids`** — DIDs to watch on the Jetstream. The Jetstream client subscribes
//!   to events for these DIDs (includes the spindle owner + all members).

use rusqlite::{Connection, OptionalExtension, params};

/// A spindle member record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// Database row ID.
    pub id: i64,
    /// DID of the member (e.g. `"did:plc:abc123"`).
    pub did: String,
    /// Role: `"owner"` or `"member"`.
    pub role: String,
    /// When the member was added.
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// spindle_members table
// ---------------------------------------------------------------------------

/// Add a spindle member.
///
/// If the DID already exists, this is a no-op (using `INSERT OR IGNORE`).
pub fn add_member(conn: &Connection, did: &str, role: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO spindle_members (did, role) VALUES (?1, ?2)",
        params![did, role],
    )?;
    Ok(())
}

/// Add a spindle member with the `"member"` role.
pub fn add_spindle_member(conn: &Connection, did: &str) -> rusqlite::Result<()> {
    add_member(conn, did, "member")
}

/// Add a spindle owner.
pub fn add_spindle_owner(conn: &Connection, did: &str) -> rusqlite::Result<()> {
    add_member(conn, did, "owner")
}

/// Remove a spindle member by DID.
///
/// Returns `true` if a row was deleted, `false` if the member wasn't found.
pub fn remove_member(conn: &Connection, did: &str) -> rusqlite::Result<bool> {
    let deleted = conn.execute("DELETE FROM spindle_members WHERE did = ?1", params![did])?;
    Ok(deleted > 0)
}

/// Get a spindle member by DID.
pub fn get_member(conn: &Connection, did: &str) -> rusqlite::Result<Option<Member>> {
    conn.query_row(
        "SELECT id, did, role, created_at FROM spindle_members WHERE did = ?1",
        params![did],
        |row| {
            Ok(Member {
                id: row.get(0)?,
                did: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
    .optional()
}

/// Get all spindle members.
pub fn get_all_members(conn: &Connection) -> rusqlite::Result<Vec<Member>> {
    let mut stmt =
        conn.prepare("SELECT id, did, role, created_at FROM spindle_members ORDER BY id")?;

    let members = stmt
        .query_map([], |row| {
            Ok(Member {
                id: row.get(0)?,
                did: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(members)
}

/// Get all spindle members with a specific role.
pub fn get_members_by_role(conn: &Connection, role: &str) -> rusqlite::Result<Vec<Member>> {
    let mut stmt = conn.prepare(
        "SELECT id, did, role, created_at FROM spindle_members WHERE role = ?1 ORDER BY id",
    )?;

    let members = stmt
        .query_map(params![role], |row| {
            Ok(Member {
                id: row.get(0)?,
                did: row.get(1)?,
                role: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(members)
}

/// Check whether a DID is a spindle member (any role).
pub fn is_member(conn: &Connection, did: &str) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM spindle_members WHERE did = ?1",
        params![did],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// dids table (Jetstream watch list)
// ---------------------------------------------------------------------------

/// Add a DID to the Jetstream watch list.
///
/// If the DID already exists, this is a no-op (the table has a primary key on `did`).
pub fn add_did(conn: &Connection, did: &str) -> rusqlite::Result<()> {
    conn.execute("INSERT OR IGNORE INTO dids (did) VALUES (?1)", params![did])?;
    Ok(())
}

/// Remove a DID from the Jetstream watch list.
///
/// Returns `true` if a row was deleted, `false` if the DID wasn't found.
pub fn remove_did(conn: &Connection, did: &str) -> rusqlite::Result<bool> {
    let deleted = conn.execute("DELETE FROM dids WHERE did = ?1", params![did])?;
    Ok(deleted > 0)
}

/// Get all DIDs on the Jetstream watch list.
pub fn get_all_dids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT did FROM dids ORDER BY did")?;

    let dids = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(dids)
}

/// Check whether a DID is on the Jetstream watch list.
pub fn has_did(conn: &Connection, did: &str) -> rusqlite::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM dids WHERE did = ?1",
        params![did],
        |row| row.get(0),
    )?;
    Ok(count > 0)
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

    // -----------------------------------------------------------------------
    // spindle_members tests
    // -----------------------------------------------------------------------

    #[test]
    fn add_and_get_member() {
        let conn = setup_db();

        add_spindle_member(&conn, "did:plc:alice").unwrap();
        let member = get_member(&conn, "did:plc:alice")
            .unwrap()
            .expect("member should exist");
        assert_eq!(member.did, "did:plc:alice");
        assert_eq!(member.role, "member");
    }

    #[test]
    fn add_owner() {
        let conn = setup_db();

        add_spindle_owner(&conn, "did:plc:owner").unwrap();
        let member = get_member(&conn, "did:plc:owner")
            .unwrap()
            .expect("owner should exist");
        assert_eq!(member.role, "owner");
    }

    #[test]
    fn add_member_is_idempotent() {
        let conn = setup_db();

        add_spindle_member(&conn, "did:plc:alice").unwrap();
        add_spindle_member(&conn, "did:plc:alice").unwrap();

        let members = get_all_members(&conn).unwrap();
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn remove_member_existing() {
        let conn = setup_db();

        add_spindle_member(&conn, "did:plc:alice").unwrap();
        assert!(remove_member(&conn, "did:plc:alice").unwrap());

        let member = get_member(&conn, "did:plc:alice").unwrap();
        assert!(member.is_none());
    }

    #[test]
    fn remove_member_not_found() {
        let conn = setup_db();
        assert!(!remove_member(&conn, "did:plc:nobody").unwrap());
    }

    #[test]
    fn get_all_members_empty() {
        let conn = setup_db();
        let members = get_all_members(&conn).unwrap();
        assert!(members.is_empty());
    }

    #[test]
    fn get_all_members_returns_all() {
        let conn = setup_db();

        add_spindle_owner(&conn, "did:plc:owner").unwrap();
        add_spindle_member(&conn, "did:plc:alice").unwrap();
        add_spindle_member(&conn, "did:plc:bob").unwrap();

        let members = get_all_members(&conn).unwrap();
        assert_eq!(members.len(), 3);
    }

    #[test]
    fn get_members_by_role_filters() {
        let conn = setup_db();

        add_spindle_owner(&conn, "did:plc:owner").unwrap();
        add_spindle_member(&conn, "did:plc:alice").unwrap();
        add_spindle_member(&conn, "did:plc:bob").unwrap();

        let owners = get_members_by_role(&conn, "owner").unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].did, "did:plc:owner");

        let members = get_members_by_role(&conn, "member").unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn is_member_true() {
        let conn = setup_db();

        add_spindle_member(&conn, "did:plc:alice").unwrap();
        assert!(is_member(&conn, "did:plc:alice").unwrap());
    }

    #[test]
    fn is_member_false() {
        let conn = setup_db();
        assert!(!is_member(&conn, "did:plc:nobody").unwrap());
    }

    #[test]
    fn is_member_includes_owner() {
        let conn = setup_db();

        add_spindle_owner(&conn, "did:plc:owner").unwrap();
        assert!(is_member(&conn, "did:plc:owner").unwrap());
    }

    // -----------------------------------------------------------------------
    // dids table tests
    // -----------------------------------------------------------------------

    #[test]
    fn add_and_get_dids() {
        let conn = setup_db();

        add_did(&conn, "did:plc:alice").unwrap();
        add_did(&conn, "did:plc:bob").unwrap();

        let dids = get_all_dids(&conn).unwrap();
        assert_eq!(dids, vec!["did:plc:alice", "did:plc:bob"]);
    }

    #[test]
    fn add_did_is_idempotent() {
        let conn = setup_db();

        add_did(&conn, "did:plc:alice").unwrap();
        add_did(&conn, "did:plc:alice").unwrap();

        let dids = get_all_dids(&conn).unwrap();
        assert_eq!(dids.len(), 1);
    }

    #[test]
    fn remove_did_existing() {
        let conn = setup_db();

        add_did(&conn, "did:plc:alice").unwrap();
        assert!(remove_did(&conn, "did:plc:alice").unwrap());

        let dids = get_all_dids(&conn).unwrap();
        assert!(dids.is_empty());
    }

    #[test]
    fn remove_did_not_found() {
        let conn = setup_db();
        assert!(!remove_did(&conn, "did:plc:nobody").unwrap());
    }

    #[test]
    fn has_did_true() {
        let conn = setup_db();

        add_did(&conn, "did:plc:alice").unwrap();
        assert!(has_did(&conn, "did:plc:alice").unwrap());
    }

    #[test]
    fn has_did_false() {
        let conn = setup_db();
        assert!(!has_did(&conn, "did:plc:nobody").unwrap());
    }

    #[test]
    fn get_all_dids_empty() {
        let conn = setup_db();
        let dids = get_all_dids(&conn).unwrap();
        assert!(dids.is_empty());
    }

    #[test]
    fn get_all_dids_sorted() {
        let conn = setup_db();

        add_did(&conn, "did:plc:charlie").unwrap();
        add_did(&conn, "did:plc:alice").unwrap();
        add_did(&conn, "did:plc:bob").unwrap();

        let dids = get_all_dids(&conn).unwrap();
        assert_eq!(
            dids,
            vec!["did:plc:alice", "did:plc:bob", "did:plc:charlie"]
        );
    }
}
