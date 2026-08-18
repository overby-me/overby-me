use goblin::elf::Elf;
use goblin::elf::dynamic::{DT_STRSZ, DT_STRTAB};
use std::process;

/// Get the file offset and size of the dynamic string table (.dynstr).
pub fn dynstr_info(elf: &Elf, _data: &[u8]) -> (usize, usize) {
    let mut strtab_addr: Option<u64> = None;
    let mut strsz: Option<u64> = None;

    if let Some(dynamic) = &elf.dynamic {
        for dyn_entry in &dynamic.dyns {
            match dyn_entry.d_tag {
                DT_STRTAB => strtab_addr = Some(dyn_entry.d_val),
                DT_STRSZ => strsz = Some(dyn_entry.d_val),
                _ => {}
            }
        }
    }

    let strtab_vaddr = strtab_addr.unwrap_or_else(|| {
        eprintln!("patchelf: no DT_STRTAB found");
        process::exit(1);
    });
    let strtab_size = strsz.unwrap_or_else(|| {
        eprintln!("patchelf: no DT_STRSZ found");
        process::exit(1);
    });

    let strtab_offset = vaddr_to_offset(elf, strtab_vaddr).unwrap_or_else(|| {
        eprintln!("patchelf: cannot map DT_STRTAB vaddr to file offset");
        process::exit(1);
    });

    (strtab_offset, strtab_size as usize)
}

/// Convert a virtual address to a file offset using program headers.
pub fn vaddr_to_offset(elf: &Elf, vaddr: u64) -> Option<usize> {
    for phdr in &elf.program_headers {
        if phdr.p_type == goblin::elf::program_header::PT_LOAD
            && vaddr >= phdr.p_vaddr
            && vaddr < phdr.p_vaddr + phdr.p_memsz
        {
            return Some((vaddr - phdr.p_vaddr + phdr.p_offset) as usize);
        }
    }
    None
}

/// Read a null-terminated string from a buffer at the given offset.
pub fn read_str_at(data: &[u8], offset: usize) -> &str {
    let end = data[offset..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| offset + p)
        .unwrap_or(data.len());
    std::str::from_utf8(&data[offset..end]).unwrap_or("")
}

/// Find the file offset of a string within the dynamic string table, given its value.
#[allow(dead_code)]
pub fn find_dynstr_offset(
    data: &[u8],
    strtab_offset: usize,
    strtab_size: usize,
    needle: &str,
) -> Option<usize> {
    let strtab = &data[strtab_offset..strtab_offset + strtab_size];
    let needle_bytes = needle.as_bytes();
    let mut i = 0;
    while i < strtab.len() {
        let end = strtab[i..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| i + p)
            .unwrap_or(strtab.len());
        if &strtab[i..end] == needle_bytes {
            return Some(strtab_offset + i);
        }
        i = end + 1;
    }
    None
}

/// Write a string in-place in the buffer at the given offset, padding with null bytes
/// up to max_len (including the null terminator area).
pub fn write_str_inplace(data: &mut [u8], offset: usize, max_len: usize, new_val: &str) {
    let new_bytes = new_val.as_bytes();
    if new_bytes.len() + 1 > max_len {
        eprintln!(
            "patchelf: new value '{}' ({} bytes) does not fit in the available space ({} bytes)",
            new_val,
            new_bytes.len(),
            max_len - 1
        );
        process::exit(1);
    }
    data[offset..offset + new_bytes.len()].copy_from_slice(new_bytes);
    for b in &mut data[offset + new_bytes.len()..offset + max_len] {
        *b = 0;
    }
}

/// Get the writable space at offset (string length + following null padding).
pub fn available_space_at(data: &[u8], offset: usize, strtab_end: usize) -> usize {
    let str_end = data[offset..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| offset + p)
        .unwrap_or(strtab_end);
    let mut pad_end = str_end + 1;
    while pad_end < strtab_end && data[pad_end] == 0 {
        pad_end += 1;
    }
    pad_end - offset
}
