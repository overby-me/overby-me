//! ELF64 binary emission helpers.
//!
//! Common functions for writing ELF64 section headers, program headers,
//! and performing alignment/padding. Used by x86, RISC-V, and ARM linkers.

/// Write a 64-byte ELF64 section header to the buffer.
pub fn write_elf64_shdr(
    elf: &mut Vec<u8>,
    name: u32,
    sh_type: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
) {
    elf.extend_from_slice(&name.to_le_bytes());
    elf.extend_from_slice(&sh_type.to_le_bytes());
    elf.extend_from_slice(&flags.to_le_bytes());
    elf.extend_from_slice(&addr.to_le_bytes());
    elf.extend_from_slice(&offset.to_le_bytes());
    elf.extend_from_slice(&size.to_le_bytes());
    elf.extend_from_slice(&link.to_le_bytes());
    elf.extend_from_slice(&info.to_le_bytes());
    elf.extend_from_slice(&align.to_le_bytes());
    elf.extend_from_slice(&entsize.to_le_bytes());
}

/// Write a 56-byte ELF64 program header by appending to the buffer.
pub fn write_elf64_phdr(
    elf: &mut Vec<u8>,
    p_type: u32,
    p_flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    p_align: u64,
) {
    elf.extend_from_slice(&p_type.to_le_bytes());
    elf.extend_from_slice(&p_flags.to_le_bytes());
    elf.extend_from_slice(&offset.to_le_bytes());
    elf.extend_from_slice(&vaddr.to_le_bytes());
    elf.extend_from_slice(&paddr.to_le_bytes());
    elf.extend_from_slice(&filesz.to_le_bytes());
    elf.extend_from_slice(&memsz.to_le_bytes());
    elf.extend_from_slice(&p_align.to_le_bytes());
}

/// Write a 56-byte ELF64 program header at a specific offset (for backpatching).
pub fn write_elf64_phdr_at(
    elf: &mut [u8],
    off: usize,
    p_type: u32,
    p_flags: u32,
    offset: u64,
    vaddr: u64,
    paddr: u64,
    filesz: u64,
    memsz: u64,
    p_align: u64,
) {
    elf[off..off + 4].copy_from_slice(&p_type.to_le_bytes());
    elf[off + 4..off + 8].copy_from_slice(&p_flags.to_le_bytes());
    elf[off + 8..off + 16].copy_from_slice(&offset.to_le_bytes());
    elf[off + 16..off + 24].copy_from_slice(&vaddr.to_le_bytes());
    elf[off + 24..off + 32].copy_from_slice(&paddr.to_le_bytes());
    elf[off + 32..off + 40].copy_from_slice(&filesz.to_le_bytes());
    elf[off + 40..off + 48].copy_from_slice(&memsz.to_le_bytes());
    elf[off + 48..off + 56].copy_from_slice(&p_align.to_le_bytes());
}

/// Align `val` up to the next multiple of `align` (power-of-two alignment).
pub fn align_up_64(val: u64, align: u64) -> u64 {
    if align <= 1 {
        val
    } else {
        (val + align - 1) & !(align - 1)
    }
}

/// Extend buffer with zero bytes to reach `target` length.
pub fn pad_to(buf: &mut Vec<u8>, target: usize) {
    if buf.len() < target {
        buf.resize(target, 0);
    }
}
