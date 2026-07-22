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

use crate::cap::{Cap, CapFlags};
use crate::table;

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

/// Raw-pointer dereference check with **provenance** (Task B3, invariant
/// I10). The MIR pass passes both the faulting pointer and the pointer at
/// its *derivation root*; the capability is resolved from the **root**, and
/// the faulting address is compared against it.
///
/// Resolving from the root, never the faulting address, is what makes an
/// overflow into an adjacent *live* allocation fail: looking up the fault
/// would find the neighbour's (valid) capability and wrongly pass — the
/// exact F10 defect in `docs/traces/rustsec-2021-0003.md`.
///
/// # Safety
///
/// Neither pointer is dereferenced here; both are only inspected.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_check_deref_rooted(fault: *const u8, root: *const u8) {
    if !REPORTER_REGISTERED.swap(true, Ordering::Relaxed) {
        #[cfg(not(miri))]
        // SAFETY: `report` is a valid `extern "C" fn()`.
        unsafe {
            atexit(report)
        };
    }
    DEREF_CHECKS.fetch_add(1, Ordering::Relaxed);

    if fault.is_null() {
        report_null_and_abort();
    }

    // I10: resolve the owning allocation from the DERIVATION ROOT. If the
    // root has no known provenance (foreign static, or an uninstrumented
    // allocator), fall through — v0's tolerant policy for unknown
    // heap/static provenance.
    if let Some(cap) = table::lookup(root as usize) {
        let f = fault as usize;
        // Spatial (I10): the access must be inside the owning allocation.
        if f < cap.base || f >= cap.base + cap.len {
            report_oob_and_abort(f, cap.base, cap.len, cap.id.raw());
        }
        // Temporal (I8): the owning allocation must still be live. A dead
        // STACK region is a use-after-scope-exit (the rusqlite-0128 class);
        // a dead heap region is a use-after-free. Pass the recorded mint line so
        // a heap use-after-free names where the dangling reference was born.
        if !table::is_live(cap.id) {
            report_uaf_at(f, cap.base, cap.id.raw(), cap.flags, cap.site, 0, cap.mint);
        }
    }
}

/// Case-mode dealloc-reachable re-check (instrumentation point 4, I6; trace
/// `rustsec-2021-0130` §3.4). The MIR pass injects this in `case` mode before a
/// safe-pointer dereference that follows a possibly-freeing call, so a
/// reference minted while its allocation was live but invalidated by an
/// intervening **heap free** is still caught. Unlike the full deref check it
/// aborts *only* on a dead **heap** allocation: `case` elides stack-scope
/// temporal safety (that is `through`'s job), so a resolved dead **stack**
/// region passes here, preserving the mode distinction. Spatial safety was
/// established once at the raw->safe cast, so this re-check is temporal only.
///
/// # Safety
///
/// Neither pointer is dereferenced; both are only inspected.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_check_dealloc_reachable(
    fault: *const u8,
    root: *const u8,
    read_line: usize,
) {
    if !REPORTER_REGISTERED.swap(true, Ordering::Relaxed) {
        #[cfg(not(miri))]
        // SAFETY: `report` is a valid `extern "C" fn()`.
        unsafe {
            atexit(report)
        };
    }
    DEREF_CHECKS.fetch_add(1, Ordering::Relaxed);

    // Temporal only, heap only: a dead heap allocation is a use-after-free
    // `case` must catch; a dead stack scope is elided (through's job); live or
    // unknown provenance passes. `read_line` is the source line of the dangling
    // dereference (the re-check is injected right at it), so the report can name
    // where the freed node was read (`read_at`).
    if let Some(cap) = table::lookup(root as usize)
        && !table::is_live(cap.id)
        && !cap.flags.contains(CapFlags::STACK)
    {
        report_uaf_at(
            fault as usize,
            cap.base,
            cap.id.raw(),
            cap.flags,
            cap.site,
            read_line as u32,
            cap.mint,
        );
    }
}

/// Raw→safe cast check (instrumentation point 1, trace §3.1 `ensure`). The MIR
/// pass injects this where a raw pointer becomes a safe reference (`&*p`,
/// `p as &T`), validating that the resulting reference's `[fault, fault+size)`
/// extent lies within a **live** allocation resolved from the derivation root
/// (I10). This is what makes `case`-mode elision sound: `case` elides the
/// reference's later dereferences, so the reference must be proven in-bounds
/// and live *once*, at the cast. `size` is the referent's size in bytes.
///
/// # Safety
///
/// Neither pointer is dereferenced; both are only inspected.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_ensure(fault: *const u8, root: *const u8, size: usize, mint_line: usize) {
    if !REPORTER_REGISTERED.swap(true, Ordering::Relaxed) {
        #[cfg(not(miri))]
        // SAFETY: `report` is a valid `extern "C" fn()`.
        unsafe {
            atexit(report)
        };
    }
    DEREF_CHECKS.fetch_add(1, Ordering::Relaxed);

    if let Some(cap) = extent_verify(fault, root, size) {
        // Valid mint: record where this reference was born on the owning
        // allocation, so a later use-after-free names it (`minted_at`). Heap
        // only — a stack scope's `site` already carries its escape line.
        if mint_line != 0 && !cap.flags.contains(CapFlags::STACK) {
            table::note_mint(cap.rec, mint_line as u32);
        }
    }
}

/// Projected-dereference extent check (instrumentation point 0, spatial+temporal
/// on the *accessed* extent). The MIR pass injects this for a raw or `through`
/// dereference of a simple projected place (`(*p).f`, `(*p)[i]`), where the
/// access covers `[fault, fault + size)` with `fault = p + offset` and `size`
/// the projected place's layout size. Unlike the single-address deref check it
/// verifies the whole extent, so a subobject that starts in bounds but extends
/// past the end of a mis-cast allocation is caught. Resolves from the derivation
/// root (I10), never the faulting address.
///
/// # Safety
///
/// Neither pointer is dereferenced; both are only inspected.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_check_extent(fault: *const u8, root: *const u8, size: usize) {
    if !REPORTER_REGISTERED.swap(true, Ordering::Relaxed) {
        #[cfg(not(miri))]
        // SAFETY: `report` is a valid `extern "C" fn()`.
        unsafe {
            atexit(report)
        };
    }
    DEREF_CHECKS.fetch_add(1, Ordering::Relaxed);
    extent_verify(fault, root, size);
}

/// Shared extent check for [`__fec_ensure`] and [`__fec_check_extent`]: resolve
/// the owning allocation from the derivation `root` (I10) and verify
/// `[fault, fault + size)` lies inside a live allocation. Aborts on an
/// out-of-bounds or dead resolve; returns the resolved cap, or `None` for
/// unknown provenance (v0's tolerant policy for foreign statics / uninstrumented
/// allocators).
#[inline]
fn extent_verify(fault: *const u8, root: *const u8, size: usize) -> Option<Cap> {
    if fault.is_null() {
        report_null_and_abort();
    }
    let cap = table::lookup(root as usize)?;
    let f = fault as usize;
    let end = f.saturating_add(size);
    // Spatial: the whole referent must lie inside the allocation.
    if f < cap.base || end > cap.base + cap.len {
        report_oob_and_abort(f, cap.base, cap.len, cap.id.raw());
    }
    // Temporal: the allocation must be live.
    if !table::is_live(cap.id) {
        report_uaf_at(f, cap.base, cap.id.raw(), cap.flags, cap.site, 0, cap.mint);
    }
    Some(cap)
}

/// Stack scope entry (I8). The MIR pass emits this at scope entry for a
/// local whose address escapes, registering `[base, base+len)` as a live
/// stack region so a later access through an escaped pointer can be checked
/// against it. `site` is the escape site (a `SiteId`, 0 = unknown) — where
/// the address is laundered out of the frame — recorded so a later
/// use-after-scope report can name it (I9 / trace F7).
///
/// # Safety
///
/// `base` is only recorded, never dereferenced.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_scope_enter(base: *const u8, len: usize, site: usize) {
    if base.is_null() || len == 0 {
        return;
    }
    let addr = base as usize;
    // A prior frame may have left a dead stack region at this address;
    // replace it so the fresh scope registers without overlap.
    if let Some(cap) = table::lookup(addr)
        && cap.base == addr
        && !table::is_live(cap.id)
        && let Some(freed) = table::deregister(addr)
    {
        table::release_record(freed);
    }
    table::register(addr, len, CapFlags::STACK, site as u32);
}

/// Stack scope exit (I8). The MIR pass emits this at frame teardown: it
/// poisons the region (clears liveness, keeps it findable) so a later
/// access through a pointer that escaped the scope resolves as a fatal
/// use-after-scope-exit.
///
/// # Safety
///
/// `base` is only recorded, never dereferenced.
#[unsafe(no_mangle)]
pub extern "C" fn __fec_scope_exit(base: *const u8) {
    if !base.is_null() {
        table::poison(base as usize);
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

#[cold]
#[inline(never)]
fn report_uaf_at(
    fault: usize,
    base: usize,
    id: u64,
    flags: CapFlags,
    site: u32,
    read_line: u32,
    mint: u32,
) -> ! {
    let stack = flags.contains(CapFlags::STACK);
    let kind = if stack {
        "UseAfterScopeExit"
    } else {
        "UseAfterFree"
    };
    let region = if stack {
        "stack scope"
    } else {
        "heap allocation"
    };
    // The escape site (I9 / trace F7): where the address was laundered out of
    // the frame — the registration site the developer needs, not just the
    // callback the abort fires in. 0 = unrecorded.
    let escaped = if site != 0 {
        format!(" escaped_at={site}")
    } else {
        String::new()
    };
    // The mint site (trace -0130 debuggability): the source line the dangling
    // reference was born at (its raw->safe cast), recorded on the heap
    // allocation by `note_mint` at `ensure` time. Stack scopes use `escaped_at`
    // instead. 0 = unrecorded.
    let minted = if !stack && mint != 0 {
        format!(" minted_at={mint}")
    } else {
        String::new()
    };
    // The dangling-read site (trace -0130 debuggability): the source line the
    // freed region was read at. The case-mode dealloc-reachable re-check knows
    // it because it is injected right at the dereference. 0 = unrecorded.
    let read = if read_line != 0 {
        format!(" read_at={read_line}")
    } else {
        String::new()
    };
    let msg = format!(
        "fe-c-violation kind={kind} fault={fault:#x} alloc_base={base:#x} alloc_id={id}{escaped}{minted}{read}\n\
fe-c: dereference into a dead {region} (id={id}, base={base:#x}); the owning \
region was torn down / freed before this access{}{}{}\n",
        if site != 0 {
            format!(" (escaped at source line {site})")
        } else {
            String::new()
        },
        if !stack && mint != 0 {
            format!(" (reference minted at source line {mint})")
        } else {
            String::new()
        },
        if read_line != 0 {
            format!(" (read at source line {read_line})")
        } else {
            String::new()
        }
    );
    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
    std::process::abort();
}

#[cold]
#[inline(never)]
fn report_oob_and_abort(fault: usize, base: usize, len: usize, id: u64) -> ! {
    let end = base + len;
    let past = fault.saturating_sub(end);
    // Machine-parseable first line (tests assert on it), then a human note.
    // The named allocation is the one the pointer was DERIVED from, not
    // whatever the faulting address happens to land in.
    let msg = format!(
        "fe-c-violation kind=OutOfBounds fault={fault:#x} alloc_base={base:#x} \
alloc_len={len} alloc_id={id}\n\
fe-c: out-of-bounds raw dereference {past} byte(s) past the end of the owning \
allocation [{base:#x}, {end:#x}); resolved at the derivation root per I10, not \
the faulting address\n"
    );
    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
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
