//! Private memory backend for the runtime's own metadata.
//!
//! Table nodes, record arenas and bitmap chunks must never come from the
//! global allocator: once `FecAlloc` is installed (Task A3) that would
//! recurse. Everything here is grow-only anonymous mappings; metadata is
//! never returned to the OS (records and nodes are pooled via freelists at
//! the caller's level instead).
//!
//! The anonymous mapping is obtained with a **raw `mmap` syscall** (inline
//! asm), so `cementite` has *no dependencies*: a runtime linked into every
//! instrumented binary must not drag in `rustix`/`libc`/`bitflags`, whose
//! versions cannot be reconciled across separate cargo build graphs.
//!
//! Under Miri the syscall path is replaced by `std::alloc` (Miri cannot
//! interpret inline-asm syscalls). The unsafe pointer discipline on top of
//! the backend is identical in both configurations, which is what the Miri
//! tier is there to check.

/// Allocates `bytes` of zeroed, page-aligned memory that lives for the rest
/// of the process. Aborts on exhaustion: the runtime cannot degrade.
pub(crate) fn alloc_zeroed_forever(bytes: usize) -> *mut u8 {
    assert!(bytes > 0);

    #[cfg(not(miri))]
    {
        // SAFETY: a `MAP_PRIVATE | MAP_ANONYMOUS` mapping with a null hint;
        // the kernel picks the placement and zero-fills it.
        let ptr = unsafe { raw_mmap_anonymous(bytes) };
        // mmap returns a small negative value (`-errno`) on failure; user
        // addresses are positive as `isize` on the supported targets.
        if (ptr as isize) < 0 || ptr.is_null() {
            // No unwinding out of the runtime; see cementite-api.md.
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                b"cementite: metadata mmap failed\n",
            );
            std::process::abort();
        }
        ptr
    }

    #[cfg(miri)]
    {
        let layout = std::alloc::Layout::from_size_align(bytes, 4096).expect("layout");
        // SAFETY: layout has non-zero size; the allocation is intentionally
        // leaked (metadata lives forever, matching the mmap path).
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "cementite: metadata alloc failed");
        ptr
    }
}

/// `mmap(NULL, len, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`
/// via a raw Linux syscall. Returns the mapping, or a small negative
/// `-errno` value on failure.
///
/// # Safety
///
/// Issues a raw syscall; `len` must be non-zero.
#[cfg(not(miri))]
unsafe fn raw_mmap_anonymous(len: usize) -> *mut u8 {
    const PROT_READ_WRITE: usize = 0x1 | 0x2;
    const MAP_PRIVATE_ANON: usize = 0x02 | 0x20;
    let ret: isize;

    #[cfg(target_arch = "x86_64")]
    // SAFETY: mmap syscall (nr 9) with the standard argument registers.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 9isize => ret,
            in("rdi") 0usize,
            in("rsi") len,
            in("rdx") PROT_READ_WRITE,
            in("r10") MAP_PRIVATE_ANON,
            in("r8") -1isize as usize,
            in("r9") 0usize,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack, preserves_flags),
        );
    }

    #[cfg(target_arch = "aarch64")]
    // SAFETY: mmap syscall (nr 222) with the AArch64 argument registers.
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") 222usize,
            inlateout("x0") 0usize => ret,
            in("x1") len,
            in("x2") PROT_READ_WRITE,
            in("x3") MAP_PRIVATE_ANON,
            in("x4") -1isize as usize,
            in("x5") 0usize,
            options(nostack, preserves_flags),
        );
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    compile_error!("cementite's raw mmap supports x86_64 and aarch64 (v0 targets)");

    ret as *mut u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroed_and_aligned() {
        let bytes = 4096 * 2;
        let ptr = alloc_zeroed_forever(bytes);
        assert_eq!(ptr as usize % 4096, 0);
        // SAFETY: freshly mapped region of `bytes` bytes.
        let all_zero = unsafe { (0..bytes).all(|i| *ptr.add(i) == 0) };
        assert!(all_zero);
        // SAFETY: within the mapping; checks the memory is writable.
        unsafe {
            ptr.write(0xa5);
            assert_eq!(ptr.read(), 0xa5);
        }
    }
}
