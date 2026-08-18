//! The ballot-core durability WRITER (round-2 item 15, rewrite kickoff item 12):
//! loops the exact atomic-cast transaction the durable board runs (a
//! UNIQUE-token dedup insert plus an append-only body insert under BEGIN
//! IMMEDIATE) against a database file, until killed. It now writes the durable
//! board's REAL schema (`ballot_store::BOARD_DDL`), so the kill9 integration
//! test's post-crash atomicity assertions cover exactly the rows
//! `PersistentBoard::cast` writes. The token here is a synthetic monotonic value
//! (crash atomicity is orthogonal to signature verification, which the board's
//! own unit tests cover with real crypto).
//!
//! Usage: durability-harness <engine: sqlite|turso> <db-path>

use ballot_store::BOARD_DDL as SCHEMA;

fn main() {
    let mut args = std::env::args().skip(1);
    let engine = args.next().expect("engine arg (sqlite|turso)");
    let path = args.next().expect("db path arg");
    match engine.as_str() {
        "sqlite" => run_sqlite(&path),
        "turso" => run_turso(&path),
        other => panic!("unknown engine {other}"),
    }
}

fn run_sqlite(path: &str) {
    let conn = rusqlite::Connection::open(path).expect("open");
    // The decided hardened-durability settings (stack doc): WAL + synchronous
    // FULL, verified via readback.
    let mode: String = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
        .expect("wal");
    assert_eq!(mode.to_lowercase(), "wal");
    conn.pragma_update(None, "synchronous", "FULL")
        .expect("sync full");
    conn.execute_batch(SCHEMA).expect("schema");
    // Resume after the last committed position (the test may respawn the writer).
    let start: i64 = conn
        .query_row(
            "SELECT coalesce(max(position), -1) FROM board_nullifier",
            [],
            |r| r.get(0),
        )
        .expect("resume point");
    let mut position = start + 1;
    loop {
        conn.execute_batch("BEGIN IMMEDIATE")
            .expect("begin immediate");
        conn.execute(
            "INSERT INTO board_nullifier (token, position) VALUES (?1, ?2)",
            [position, position],
        )
        .expect("nullifier insert");
        conn.execute(
            "INSERT INTO board_body (position, body) VALUES (?1, '[0]')",
            [position],
        )
        .expect("body insert");
        conn.execute_batch("COMMIT").expect("commit");
        println!("{position}");
        position += 1;
    }
}

fn run_turso(path: &str) {
    // A kill-9 stress harness: each run_* builds its own runtime and dies with it.
    // ast-grep-ignore: rust-runtime-built-in-fn
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async {
        let db = turso::Builder::new_local(path).build().await.expect("open");
        let conn = db.connect().expect("connect");
        conn.execute_batch(SCHEMA).await.expect("schema");
        let mut rows = conn
            .query(
                "SELECT coalesce(max(position), -1) FROM board_nullifier",
                (),
            )
            .await
            .expect("resume q");
        let row = rows.next().await.expect("next").expect("row");
        let start: i64 = row.get(0).expect("max");
        let mut position = start + 1;
        loop {
            conn.execute("BEGIN IMMEDIATE", ()).await.expect("begin");
            conn.execute(
                "INSERT INTO board_nullifier (token, position) VALUES (?1, ?2)",
                (position, position),
            )
            .await
            .expect("nullifier insert");
            conn.execute(
                "INSERT INTO board_body (position, body) VALUES (?1, '[0]')",
                (position,),
            )
            .await
            .expect("body insert");
            conn.execute("COMMIT", ()).await.expect("commit");
            println!("{position}");
            position += 1;
        }
    });
}
