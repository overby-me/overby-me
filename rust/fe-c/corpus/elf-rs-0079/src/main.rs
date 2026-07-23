//! RUSTSEC-2022-0079: elf_rs 0.2.0 out-of-bounds via an unvalidated section
//! header count. `ElfGen::section_header_raw` does
//!
//! ```ignore
//! let sh_off = self.elf_header().section_header_offset() as usize;
//! let sh_num = self.elf_header().section_header_entry_num() as usize;
//! let sh_ptr = self.content().as_ptr().add(sh_off);
//! from_raw_parts(sh_ptr as *const ET::SectionHeader, sh_num)
//! ```
//!
//! with `sh_off`/`sh_num` read straight from the ELF header the caller supplies.
//! A crafted header with a huge `sh_num` builds a slice reaching far past the
//! input buffer; iterating the section headers reads out of bounds (the
//! advisory reports trivial SIGABRT/SEGV under fuzzing).
//!
//! The slice-constructor extent check vets `[sh_ptr, sh_ptr + sh_num *
//! size_of::<SectionHeader64>())` against the derivation root (the heap ELF
//! buffer) at the `from_raw_parts` mint, so it aborts `OutOfBounds` in **both**
//! modes — before the out-of-bounds element is ever dereferenced, naming the
//! owning buffer (I10). The ELF bytes live in a `Vec<u8>` so FecAlloc tracks
//! them.

extern crate cementite;

use std::hint::black_box;

use elf_rs::{Elf, ElfFile};

#[global_allocator]
static ALLOC: cementite::FecAlloc = cementite::FecAlloc;

/// Builds a minimal 64-byte ELF64 header (exactly `size_of::<ElfHeader64>()`)
/// that parses but lies: `section_header_offset = 0` and
/// `section_header_entry_num = 1000` (each entry is 64 bytes, so the section
/// table claims 64000 bytes over a 64-byte buffer).
fn crafted_elf() -> Vec<u8> {
    let mut buf = vec![0u8; 64];
    buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']); // e_ident magic
    buf[4] = 2; // EI_CLASS = ELFCLASS64
    buf[5] = 1; // EI_DATA  = little-endian
    buf[6] = 1; // EI_VERSION
    // e_shoff @ 0x28: section header table offset = 0 (table starts at the base)
    buf[40..48].copy_from_slice(&0u64.to_le_bytes());
    // e_ehsize @ 0x34: header size = 64 (so from_bytes's length check passes)
    buf[52..54].copy_from_slice(&64u16.to_le_bytes());
    // e_shentsize @ 0x3a: section header entry size = 64
    buf[58..60].copy_from_slice(&64u16.to_le_bytes());
    // e_shnum @ 0x3c: section header count = 1000 — the lie.
    buf[60..62].copy_from_slice(&1000u16.to_le_bytes());
    buf
}

fn main() {
    let buf = crafted_elf();
    let base = buf.as_ptr();
    eprintln!("BASE={base:p} len={}", buf.len());

    let elf = Elf::from_bytes(&buf).expect("crafted header parses");
    // Iterating the section headers calls section_header_raw() ->
    // from_raw_parts(base, 1000): a 64000-byte slice over the 64-byte buffer.
    // The slice-ctor extent check aborts at construction.
    let count = elf.section_header_iter().count();
    println!("NO_ABORT section_headers={count}");
    black_box(count);
}
