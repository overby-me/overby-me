//! Kill -9 crash-recovery assertions for the atomic-cast transaction, on both
//! engines, plus the SQLite file-format bridge claim the migration story
//! rests on. Honest limits (recorded in the stack doc's gate): SIGKILL
//! exercises process-crash atomicity, not power loss; Antithesis coverage of
//! Turso is recorded upstream, not claimed locally testable.

use std::process::{Command, Stdio};
use std::time::Duration;

/// Spawn the writer against `path`, let it commit for `run_ms`, SIGKILL it.
/// Repeats `rounds` times (each respawn resumes after the last committed txn),
/// multiplying the chances of killing mid-transaction.
fn crash_loop(engine: &str, path: &str, rounds: u32, run_ms: u64) {
    let bin = env!("CARGO_BIN_EXE_durability-harness");
    for _ in 0..rounds {
        let mut child = Command::new(bin)
            .args([engine, path])
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn writer");
        std::thread::sleep(Duration::from_millis(run_ms));
        child.kill().expect("SIGKILL");
        child.wait().expect("reap");
    }
}

/// The atomicity assertion: every committed board position has BOTH its
/// nullifier (dedup) row and its body row; no position has only one of them.
/// Returns how many entries survived (must be > 0 for the test to have exercised
/// writes). These are exactly the rows `PersistentBoard::cast` writes.
fn assert_atomic(conn: &rusqlite::Connection) -> i64 {
    let orphan_nullifiers: i64 = conn
        .query_row(
            "SELECT count(*) FROM board_nullifier n WHERE NOT EXISTS
               (SELECT 1 FROM board_body b WHERE b.position = n.position)",
            [],
            |r| r.get(0),
        )
        .expect("orphan nullifiers");
    let orphan_bodies: i64 = conn
        .query_row(
            "SELECT count(*) FROM board_body b WHERE NOT EXISTS
               (SELECT 1 FROM board_nullifier n WHERE n.position = b.position)",
            [],
            |r| r.get(0),
        )
        .expect("orphan bodies");
    assert_eq!(orphan_nullifiers, 0, "a nullifier without its body");
    assert_eq!(orphan_bodies, 0, "a body without its nullifier");
    conn.query_row("SELECT count(*) FROM board_nullifier", [], |r| r.get(0))
        .expect("count")
}

fn assert_integrity(conn: &rusqlite::Connection) {
    let verdict: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check");
    assert_eq!(verdict, "ok");
}

#[test]
fn sqlite_survives_kill9_atomically() {
    let dir = std::env::temp_dir().join(format!("durability-sqlite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let path = dir.join("core.db");
    let path_s = path.to_str().unwrap();

    crash_loop("sqlite", path_s, 5, 250);

    let conn = rusqlite::Connection::open(path_s).expect("reopen after crashes");
    assert_integrity(&conn);
    let survived = assert_atomic(&conn);
    assert!(
        survived > 0,
        "the writer committed at least one transaction"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn turso_survives_kill9_and_bridges_to_stock_sqlite() {
    let dir = std::env::temp_dir().join(format!("durability-turso-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let path = dir.join("core.db");
    let path_s = path.to_str().unwrap();

    crash_loop("turso", path_s, 5, 250);

    // THE BRIDGE CLAIM: a Turso-written database file opens UNMODIFIED in
    // stock SQLite (rusqlite's bundled build is upstream SQLite code), and
    // the atomicity + integrity assertions hold there. This is the lossless
    // file-format bridge the migration story rests on.
    let conn = rusqlite::Connection::open(path_s).expect("turso file opens in stock SQLite");
    assert_integrity(&conn);
    let survived = assert_atomic(&conn);
    assert!(
        survived > 0,
        "the writer committed at least one transaction"
    );

    // And the same file re-opens in turso itself after the crashes.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async {
        let db = turso::Builder::new_local(path_s)
            .build()
            .await
            .expect("turso reopen");
        let tconn = db.connect().expect("connect");
        let mut rows = tconn
            .query("SELECT count(*) FROM board_nullifier", ())
            .await
            .expect("query");
        let row = rows.next().await.expect("next").expect("row");
        let n: i64 = row.get(0).expect("count");
        assert!(n > 0, "turso reads its own post-crash file");
    });
    let _ = std::fs::remove_dir_all(&dir);
}
