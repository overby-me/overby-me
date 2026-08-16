//! RUSTSEC-2021-0128 reproducer against real `rusqlite` 0.25.3 (Task B5).
//!
//! `create_scalar_function`'s lifetime bound was too relaxed, so a closure
//! capturing a **stack borrow** could be registered with SQLite, outlive the
//! registering frame, and be invoked later by SQLite (C) — reading a dropped
//! stack local through a safe reference. See `docs/traces/rustsec-2021-0128.md`.
//!
//! Under the Fe-C instrument driver with `FEC_MODE=through`, the escape
//! analysis registers the local's stack scope when the borrow is captured into
//! the closure; the frame's return poisons it; and the closure's read of the
//! dead local — a *safe* dereference, which only through mode checks — resolves
//! the dead scope and aborts. The SQLite C code itself is not instrumented;
//! only the Rust boundary is checked.

extern crate cementite;

use rusqlite::functions::FunctionFlags;
use rusqlite::Connection;

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

#[inline(never)]
fn register(db: &Connection) -> rusqlite::Result<()> {
    let local: u64 = std::hint::black_box(0xDEAD_BEEF_CAFE);
    eprintln!("STACK_LOCAL={:p}", &local);
    // The bug: `move` closure capturing a borrow of `local`, which does not
    // outlive `db` — accepted by 0.25.3's too-relaxed bound. The closure reads
    // `local` back through `*r`, a safe dereference in its own body (so it is
    // instrumented, unlike a std method call), which only through mode checks.
    let r: &u64 = &local;
    db.create_scalar_function("f", 0, FunctionFlags::SQLITE_UTF8, move |_ctx| {
        Ok(*r as i64) // reads the (soon-dead) `local` through `r`
    })?;
    Ok(())
} // `local` dropped here; the registered closure now dangles

fn main() -> rusqlite::Result<()> {
    let db = Connection::open_in_memory()?;
    register(&db)?;
    // SQLite invokes the registered closure, reading the dead stack local.
    let v: i64 = db.query_row("SELECT f()", [], |row| row.get(0))?;
    println!("NO_ABORT v={v}");
    Ok(())
}
