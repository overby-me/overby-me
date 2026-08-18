use goblin::elf::Elf;
use goblin::elf::dynamic::DT_NULL;
use std::process;

/// Add a DT_DEBUG entry by repurposing the first DT_NULL slot in .dynamic.
/// DT_NULL terminates the dynamic array; the loader stops at the *first*
/// DT_NULL it sees, so we must insert *before* the terminator. The
/// upstream fixtures all have several trailing DT_NULL slots, so we
/// overwrite the one immediately before the last (i.e. the first DT_NULL
/// in the run of trailing nulls).
pub fn add_debug_tag(elf: &Elf, data: &mut [u8]) {
    const DT_DEBUG: u64 = 21;
    let is_64 = elf.is_64;
    let is_le = elf.little_endian;

    let Some(dynamic) = &elf.dynamic else {
        eprintln!("patchelf: --add-debug-tag: no PT_DYNAMIC segment found");
        process::exit(1);
    };

    // Find .dynamic file offset from program headers.
    let dyn_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
        .unwrap_or_else(|| {
            eprintln!("patchelf: --add-debug-tag: no PT_DYNAMIC segment found");
            process::exit(1);
        });
    let dyn_offset = dyn_phdr.p_offset as usize;
    let entry_size = if is_64 { 16usize } else { 8usize };

    // Check for existing DT_DEBUG.
    for entry in &dynamic.dyns {
        if entry.d_tag == DT_DEBUG as i64 as u64 {
            return;
        }
    }

    // Find the first DT_NULL slot; write DT_DEBUG with d_val=0 there.
    for (idx, entry) in dynamic.dyns.iter().enumerate() {
        if entry.d_tag == DT_NULL {
            let entry_offset = dyn_offset + idx * entry_size;
            if is_64 {
                let tag = if is_le {
                    (DT_DEBUG).to_le_bytes()
                } else {
                    (DT_DEBUG).to_be_bytes()
                };
                data[entry_offset..entry_offset + 8].copy_from_slice(&tag);
                // d_val remains zero.
            } else {
                let tag = if is_le {
                    (DT_DEBUG as u32).to_le_bytes()
                } else {
                    (DT_DEBUG as u32).to_be_bytes()
                };
                data[entry_offset..entry_offset + 4].copy_from_slice(&tag);
            }
            return;
        }
    }

    eprintln!("patchelf: --add-debug-tag: no DT_NULL slot available in .dynamic");
    process::exit(1);
}
