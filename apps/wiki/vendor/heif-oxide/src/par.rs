//! Scoped parallelism, or the lack of it on a target without threads.
//!
//! THE VENDOR PATCH. Upstream calls `std::thread::scope` unconditionally in
//! three places. On `wasm32-unknown-unknown` a browser cannot spawn a thread,
//! so `Scope::spawn` panics with
//!
//! ```text
//! failed to spawn thread: Error { kind: Unsupported, ... }
//! ```
//!
//! and under `panic = "abort"` that ends the whole app, not just the decode.
//!
//! Note the work is already split for one worker there: `available_parallelism`
//! returns `Err` on wasm, so upstream's `unwrap_or(1)` yields a single chunk. It
//! still spawns a thread to run that one chunk, which is the only reason it
//! dies. Running the chunk on the calling thread instead changes nothing about
//! what is computed or in what order.
//!
//! Everywhere else this is `std::thread::scope` itself, so native builds keep
//! the parallel decode exactly as upstream wrote it.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::thread::scope;

/// A scope that runs each "spawned" closure immediately, on this thread.
///
/// `spawn` returns nothing, where the real one returns a `ScopedJoinHandle`.
/// None of the three call sites keep the handle, so the difference does not
/// show. `Send` is likewise not required: nothing crosses a thread.
#[cfg(target_arch = "wasm32")]
pub(crate) struct Scope;

#[cfg(target_arch = "wasm32")]
impl Scope {
    pub(crate) fn spawn<F: FnOnce()>(&self, f: F) {
        f();
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn scope<R>(f: impl FnOnce(&Scope) -> R) -> R {
    f(&Scope)
}
