//! libc allocator interposition, tier 1 (Task A4).
//!
//! `#[no_mangle] extern "C"` definitions of `malloc`/`calloc`/`realloc`/
//! `free`/`posix_memalign` that override libc's, forward to the real ones
//! via `dlsym(RTLD_NEXT, …)`, and register foreign/libc-internal
//! allocations in the same table Rust allocations use. This is
//! *interposition, not instrumentation*: the project stays pure Rust and
//! the C code is untouched (PLAN section 3).
//!
//! Gated behind the off-by-default `interpose` feature: a build that also
//! installs [`crate::FecAlloc`] as the global allocator would otherwise
//! see every Rust allocation twice (once here, once there). When the
//! feature is on, `FecAlloc` brackets its `System` calls with
//! [`enter_reentrant`] so exactly one layer registers.
//!
//! ## Three hazards, one thread-local guard
//!
//! 1. **dlsym bootstrap.** Resolving `malloc` calls `dlsym`, which itself
//!    calls `calloc`. A reentrant allocation during resolution is served
//!    from a static bump buffer instead of recursing into `dlsym`.
//! 2. **register reentrancy.** Registration must not loop back through
//!    `malloc`. On the native target it uses `mmap`, but the guard makes
//!    that robust regardless.
//! 3. **FecAlloc coordination.** See above.
//!
//! Not exercised under Miri (dlsym/mmap are outside its model); the same
//! table/quarantine unsafe is Miri-checked through the Rust allocator path.

use core::cell::Cell;
use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use crate::cap::CapFlags;
use crate::table;

// ---- reentrancy guard -----------------------------------------------------

#[thread_local]
static REENTRANT: Cell<bool> = Cell::new(false);

/// RAII scope that marks the current thread as inside allocator machinery.
/// While held, interposed entry points forward to the real allocator
/// without touching the table, breaking the three recursion hazards above.
pub(crate) struct ReentGuard(bool);

impl Drop for ReentGuard {
    fn drop(&mut self) {
        REENTRANT.set(self.0);
    }
}

/// Enters a reentrant scope, restoring the previous state on drop. Used by
/// [`crate::FecAlloc`] to bracket its `System` calls so this layer does not
/// double-register Rust allocations.
pub(crate) fn enter_reentrant() -> ReentGuard {
    ReentGuard(REENTRANT.replace(true))
}

fn is_reentrant() -> bool {
    REENTRANT.get()
}

// ---- real libc functions, resolved lazily via dlsym(RTLD_NEXT) ------------

type MallocFn = unsafe extern "C" fn(usize) -> *mut c_void;
type CallocFn = unsafe extern "C" fn(usize, usize) -> *mut c_void;
type ReallocFn = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
type FreeFn = unsafe extern "C" fn(*mut c_void);
type PosixMemalignFn = unsafe extern "C" fn(*mut *mut c_void, usize, usize) -> c_int;

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

/// `RTLD_NEXT`: resolve the *next* definition after this object, i.e. the
/// real libc one. It is a magic handle, not a real address.
const RTLD_NEXT: *mut c_void = usize::MAX as *mut c_void;

static REAL_MALLOC: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static REAL_CALLOC: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static REAL_REALLOC: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static REAL_FREE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static REAL_POSIX_MEMALIGN: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// Resolves `name` via `dlsym(RTLD_NEXT, …)`, caching the result. Returns
/// null while resolution is already in progress on this thread, so the
/// caller falls back to the bootstrap buffer rather than recursing.
fn resolve(slot: &AtomicPtr<c_void>, name: &[u8]) -> *mut c_void {
    let cached = slot.load(Ordering::Acquire);
    if !cached.is_null() {
        return cached;
    }
    if is_reentrant() {
        return core::ptr::null_mut();
    }
    debug_assert_eq!(*name.last().unwrap(), 0, "name must be NUL-terminated");
    let _guard = enter_reentrant();
    // SAFETY: RTLD_NEXT is the documented magic handle; name is a
    // NUL-terminated C string.
    let sym = unsafe { dlsym(RTLD_NEXT, name.as_ptr().cast()) };
    if !sym.is_null() {
        slot.store(sym, Ordering::Release);
    }
    sym
}

// ---- bootstrap bump buffer ------------------------------------------------

const BOOT_BYTES: usize = 1 << 16;

/// A zero-initialized buffer serving the handful of allocations `dlsym`
/// makes before the real `malloc` is known. Never freed; a leak of at most
/// `BOOT_BYTES`.
#[repr(align(16))]
struct BootBuf(core::cell::UnsafeCell<[u8; BOOT_BYTES]>);
// SAFETY: access is bump-allocated via an atomic offset; bytes are handed
// out once and never aliased mutably.
unsafe impl Sync for BootBuf {}

static BOOT_BUF: BootBuf = BootBuf(core::cell::UnsafeCell::new([0; BOOT_BYTES]));
static BOOT_OFF: AtomicUsize = AtomicUsize::new(0);

fn boot_base() -> usize {
    BOOT_BUF.0.get() as usize
}

fn is_bootstrap(ptr: *mut c_void) -> bool {
    let a = ptr as usize;
    let base = boot_base();
    a >= base && a < base + BOOT_BYTES
}

/// Serves `size` bytes (min `align`) from the bump buffer. Aborts the
/// process if the buffer is exhausted: that would mean `dlsym` needs more
/// scratch than expected and continuing is not safe.
fn bootstrap_alloc(size: usize, align: usize) -> *mut c_void {
    let align = align.max(16);
    let mut off = BOOT_OFF.load(Ordering::Relaxed);
    loop {
        let base = (off + align - 1) & !(align - 1);
        let end = match base.checked_add(size.max(1)) {
            Some(e) if e <= BOOT_BYTES => e,
            _ => {
                // No allocator to fall back on; loudly abort.
                rtabort("cementite: bootstrap allocation buffer exhausted");
            }
        };
        match BOOT_OFF.compare_exchange_weak(off, end, Ordering::Relaxed, Ordering::Relaxed) {
            // SAFETY: [base, end) is a fresh, exclusively-owned slice of the
            // buffer; the pointer stays valid for the process lifetime.
            Ok(_) => return unsafe { (boot_base() as *mut u8).add(base) as *mut c_void },
            Err(cur) => off = cur,
        }
    }
}

fn rtabort(msg: &str) -> ! {
    // Avoid any allocation on the abort path.
    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
    let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\n");
    std::process::abort();
}

// ---- interposed entry points ----------------------------------------------

/// Flag on interposed (foreign/libc) allocations: visible to C, so exempt
/// from Rust-exclusivity elision and eligible for escape reporting (I9).
const FOREIGN: CapFlags = CapFlags::ESCAPED;

/// # Safety
/// Standard C `malloc` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    let real = resolve(&REAL_MALLOC, b"malloc\0");
    if is_reentrant() {
        return raw_malloc(real, size);
    }
    let _guard = enter_reentrant();
    let p = raw_malloc(real, size);
    if !p.is_null() && size > 0 {
        table::register(p as usize, size, FOREIGN, 0);
    }
    p
}

fn raw_malloc(real: *mut c_void, size: usize) -> *mut c_void {
    if real.is_null() {
        return bootstrap_alloc(size, 16);
    }
    // SAFETY: `real` is libc malloc resolved via dlsym.
    unsafe { core::mem::transmute::<*mut c_void, MallocFn>(real)(size) }
}

/// # Safety
/// Standard C `calloc` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(nmemb: usize, size: usize) -> *mut c_void {
    let total = match nmemb.checked_mul(size) {
        Some(t) => t,
        None => return core::ptr::null_mut(),
    };
    let real = resolve(&REAL_CALLOC, b"calloc\0");
    if is_reentrant() {
        return raw_calloc(real, nmemb, size, total);
    }
    let _guard = enter_reentrant();
    let p = raw_calloc(real, nmemb, size, total);
    if !p.is_null() && total > 0 {
        table::register(p as usize, total, FOREIGN, 0);
    }
    p
}

fn raw_calloc(real: *mut c_void, nmemb: usize, size: usize, total: usize) -> *mut c_void {
    if real.is_null() {
        // Bootstrap buffer is already zeroed.
        return bootstrap_alloc(total, 16);
    }
    // SAFETY: `real` is libc calloc resolved via dlsym.
    unsafe { core::mem::transmute::<*mut c_void, CallocFn>(real)(nmemb, size) }
}

/// # Safety
/// Standard C `realloc` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        // SAFETY: realloc(NULL, n) == malloc(n).
        return unsafe { malloc(size) };
    }
    if is_bootstrap(ptr) {
        // Migrate a bootstrap allocation into the real heap.
        // SAFETY: fresh allocation; copy stays within the bootstrap buffer.
        let new = unsafe { malloc(size) };
        if !new.is_null() {
            let copy = size.min(BOOT_BYTES - (ptr as usize - boot_base()));
            // SAFETY: src is within BOOT_BUF, dst is a fresh size-byte alloc.
            unsafe { core::ptr::copy_nonoverlapping(ptr as *const u8, new as *mut u8, copy) };
        }
        return new;
    }

    let real = resolve(&REAL_REALLOC, b"realloc\0");
    if real.is_null() {
        return core::ptr::null_mut();
    }
    if is_reentrant() {
        // SAFETY: real libc realloc.
        return unsafe { core::mem::transmute::<*mut c_void, ReallocFn>(real)(ptr, size) };
    }
    let _guard = enter_reentrant();
    // SAFETY: real libc realloc.
    let new = unsafe { core::mem::transmute::<*mut c_void, ReallocFn>(real)(ptr, size) };
    if !new.is_null() {
        // The old block was resized in place or freed-and-moved; either way
        // its old identity is gone. Retire it and register the result.
        table::deregister(ptr as usize);
        if size > 0 {
            table::register(new as usize, size, FOREIGN, 0);
        }
    }
    // On failure the original block is untouched and stays registered.
    new
}

/// # Safety
/// Standard C `free` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    if ptr.is_null() || is_bootstrap(ptr) {
        // Bootstrap memory is never returned to the OS.
        return;
    }
    let real = resolve(&REAL_FREE, b"free\0");
    if real.is_null() {
        return;
    }
    if is_reentrant() {
        // SAFETY: real libc free.
        unsafe { core::mem::transmute::<*mut c_void, FreeFn>(real)(ptr) };
        return;
    }
    let _guard = enter_reentrant();
    // I7: clear the liveness bit before the memory is released. (v0
    // interposed frees release immediately; routing them through the
    // shared quarantine needs the per-origin release dispatch noted in
    // STATUS, and is deferred.)
    table::deregister(ptr as usize);
    // SAFETY: real libc free; ptr came from a matching libc allocation.
    unsafe { core::mem::transmute::<*mut c_void, FreeFn>(real)(ptr) };
}

/// # Safety
/// Standard POSIX `posix_memalign` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_memalign(
    memptr: *mut *mut c_void,
    align: usize,
    size: usize,
) -> c_int {
    const EINVAL: c_int = 22;
    const ENOMEM: c_int = 12;

    // align must be a power of two and a multiple of size_of::<*mut _>().
    if !align.is_power_of_two() || !align.is_multiple_of(size_of::<*mut c_void>()) {
        return EINVAL;
    }
    let real = resolve(&REAL_POSIX_MEMALIGN, b"posix_memalign\0");

    if real.is_null() {
        let p = bootstrap_alloc(size, align);
        if p.is_null() {
            return ENOMEM;
        }
        // SAFETY: caller-provided out-pointer.
        unsafe { *memptr = p };
        return 0;
    }
    if is_reentrant() {
        // SAFETY: real libc posix_memalign.
        return unsafe {
            core::mem::transmute::<*mut c_void, PosixMemalignFn>(real)(memptr, align, size)
        };
    }
    let _guard = enter_reentrant();
    // SAFETY: real libc posix_memalign.
    let r =
        unsafe { core::mem::transmute::<*mut c_void, PosixMemalignFn>(real)(memptr, align, size) };
    if r == 0 && size > 0 {
        // SAFETY: on success memptr holds a valid allocation pointer.
        let p = unsafe { *memptr };
        if !p.is_null() {
            table::register(p as usize, size, FOREIGN, 0);
        }
    }
    r
}
