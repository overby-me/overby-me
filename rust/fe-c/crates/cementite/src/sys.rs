//! Private memory backend for the runtime's own metadata.
//!
//! Table nodes, record arenas and bitmap chunks must never come from the
//! global allocator: once `FecAlloc` is installed (Task A3) that would
//! recurse. Everything here is grow-only anonymous mappings; metadata is
//! never returned to the OS (records and nodes are pooled via freelists at
//! the caller's level instead).
//!
//! Under Miri the raw `mmap` syscall path is replaced by `std::alloc`
//! (rustix issues syscalls via inline asm, which Miri cannot interpret).
//! The unsafe pointer discipline on top of the backend is identical in both
//! configurations, which is what the Miri tier is there to check.

/// Allocates `bytes` of zeroed, page-aligned memory that lives for the rest
/// of the process. Aborts on exhaustion: the runtime cannot degrade.
pub(crate) fn alloc_zeroed_forever(bytes: usize) -> *mut u8 {
    assert!(bytes > 0);

    #[cfg(not(miri))]
    {
        use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous};
        // SAFETY: anonymous private mapping with a null hint; the kernel
        // picks the placement and the mapping is zero-filled.
        let ptr = unsafe {
            mmap_anonymous(
                core::ptr::null_mut(),
                bytes,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::PRIVATE,
            )
        };
        match ptr {
            Ok(p) => p.cast(),
            Err(err) => {
                // No unwinding out of the runtime; see cementite-api.md.
                eprintln!("cementite: metadata mmap of {bytes} bytes failed: {err}");
                std::process::abort();
            }
        }
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
