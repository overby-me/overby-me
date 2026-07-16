//! The ballot-core durability WRITER (round-2 item 15): loops the exact
//! atomic-cast transaction shape from the domain model (a unique-constrained
//! dedup insert plus an append-only ballot insert under BEGIN IMMEDIATE)
//! against a database file, until killed. The kill9 integration test spawns
//! this binary, SIGKILLs it mid-write, and asserts post-crash atomicity.
//!
//! The tested shape is invariant under the coming dedup-key change (interim
//! per-user marker becomes the token nullifier), so this harness transfers
//! unchanged as the ballot core's durability suite.
//!
//! Usage: durability-harness <engine: sqlite|turso> <db-path>

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS dedup (
  txn_id INTEGER PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS ballot (
  txn_id  INTEGER NOT NULL,
  choices TEXT NOT NULL
);
";

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
    // Resume after the last committed txn (the test may respawn the writer).
    let start: i64 = conn
        .query_row("SELECT coalesce(max(txn_id), 0) FROM dedup", [], |r| {
            r.get(0)
        })
        .expect("resume point");
    let mut txn_id = start + 1;
    loop {
        conn.execute_batch("BEGIN IMMEDIATE")
            .expect("begin immediate");
        conn.execute("INSERT INTO dedup (txn_id) VALUES (?1)", [txn_id])
            .expect("dedup insert");
        conn.execute(
            "INSERT INTO ballot (txn_id, choices) VALUES (?1, '[0]')",
            [txn_id],
        )
        .expect("ballot insert");
        conn.execute_batch("COMMIT").expect("commit");
        println!("{txn_id}");
        txn_id += 1;
    }
}

fn run_turso(path: &str) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async {
        let db = turso::Builder::new_local(path).build().await.expect("open");
        let conn = db.connect().expect("connect");
        conn.execute_batch(SCHEMA).await.expect("schema");
        let mut rows = conn
            .query("SELECT coalesce(max(txn_id), 0) FROM dedup", ())
            .await
            .expect("resume q");
        let row = rows.next().await.expect("next").expect("row");
        let start: i64 = row.get(0).expect("max");
        let mut txn_id = start + 1;
        loop {
            conn.execute("BEGIN IMMEDIATE", ()).await.expect("begin");
            conn.execute("INSERT INTO dedup (txn_id) VALUES (?1)", (txn_id,))
                .await
                .expect("dedup insert");
            conn.execute(
                "INSERT INTO ballot (txn_id, choices) VALUES (?1, '[0]')",
                (txn_id,),
            )
            .await
            .expect("ballot insert");
            conn.execute("COMMIT", ()).await.expect("commit");
            println!("{txn_id}");
            txn_id += 1;
        }
    });
}
