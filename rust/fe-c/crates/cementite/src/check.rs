//! Check entry points the MIR pass rewrites accesses into (Task B2+).
//!
//! These are the "ordinary calls into cementite" the design mandates: the
//! `fe-c-driver` MIR pass injects a call to one of these before each
//! instrumented access. In B2 the raw-deref entry point counts executed
//! checks (proving the rewriting fires at runtime) and rejects null; the
//! bounds/liveness comparison against a propagated capability is Task B3.
//!
//! Kept in a plain, always-compiled module (no feature gate) so the symbol
//! is linkable into any instrumented crate, and resolvable by path
//! (`cementite::__fec_check_deref`) from the driver.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Number of raw-dereference checks executed this process.
static DEREF_CHECKS: AtomicU64 = AtomicU64::new(0);
/// Whether the at-exit reporter has been registered yet.
static REPORTER_REGISTERED: AtomicBool = AtomicBool::new(false);

#[cfg(not(miri))]
unsafe extern "C" {
    /// libc `atexit`, used to print the check tally once at process end.
    fn atexit(cb: extern "C" fn()) -> i32;
}

/// Prints the executed-check tally at process exit. The format is stable so
/// tests can assert on it. Not compiled under Miri, which cannot register
/// `atexit`.
#[cfg(not(miri))]
extern "C" fn report() {
    let n = DEREF_CHECKS.load(Ordering::Relaxed);
    let msg = format!("fe-c: {n} deref checks executed\n");
    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
}

/// Returns the number of raw-dereference checks executed so far.
pub fn deref_check_count() -> u64 {
    DEREF_CHECKS.load(Ordering::Relaxed)
}

/// Raw-pointer dereference check. The MIR pass injects a call to this
/// before every raw dereference (instrumentation point 0). B2 counts the
/// call and rejects null; B3 upgrades it to a bounds/liveness comparison
/// against the propagated capability (I10), which is why the pass threads
/// the pointer through as a distinct value now.
///
/// `extern "C"` + `#[no_mangle]` so the symbol is stable, but the driver
/// resolves it by Rust path, so it also stays `pub` at the crate root.
///
/// # Safety
///
/// `ptr` is only inspected (null-checked), never dereferenced here.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_check_deref(ptr: *const u8) {
    // Register the exit reporter exactly once, on the first check. Skipped
    // under Miri, which does not shim `atexit`.
    if !REPORTER_REGISTERED.swap(true, Ordering::Relaxed) {
        #[cfg(not(miri))]
        // SAFETY: `report` is a valid `extern "C" fn()`.
        unsafe {
            atexit(report)
        };
    }

    DEREF_CHECKS.fetch_add(1, Ordering::Relaxed);

    // A genuine minimal check: dereferencing null is always a bug. Real
    // bounds/liveness checking against the propagated capability is B3.
    if ptr.is_null() {
        report_null_and_abort();
    }
}

#[cold]
#[inline(never)]
fn report_null_and_abort() -> ! {
    let _ = std::io::Write::write_all(
        &mut std::io::stderr(),
        b"fe-c: null raw-pointer dereference\n",
    );
    std::process::abort();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_allows_valid_pointers() {
        let before = deref_check_count();
        let x = 0x1234u32;
        __fec_check_deref(&x as *const u32 as *const u8);
        __fec_check_deref(&x as *const u32 as *const u8);
        assert_eq!(deref_check_count(), before + 2);
    }
}
