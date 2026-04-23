//! Minimal dynstr / PT_INTERP growth engine.
//!
//! Strings in .dynstr are referenced by *offset into strtab* (not by VA),
//! and PT_INTERP is read directly from the file by the kernel. So we can
//! relocate either by appending bytes somewhere in the file and updating
//! a few pointers (DT_STRTAB, DT_STRSZ, .dynstr section header, PT_INTERP
//! file/memory layout, LOAD #1 filesz/memsz).
//!
//! Two strategies, tried in order:
//!
//! 1. **LOAD #1 slack.** gcc/ld leaves 2-3 KB of zero padding between the
//!    end of LOAD #1 filesz and the next page boundary. Park the new
//!    payload there, grow LOAD #1 filesz/memsz to cover it. No phdr
//!    changes; no segments shift.
//!
//! 2. **GNU_STACK -> PT_LOAD.** For tiny binaries with no slack we append
//!    the payload at file EOF and rewrite the otherwise-useless
//!    PT_GNU_STACK phdr into a new R-only PT_LOAD covering it. PT_GNU_STACK
//!    is just a stack-permissions marker; converting it to a small extra
//!    LOAD is harmless.
//!
//! Supports 32-bit and 64-bit ELF, both endiannesses.

use goblin::elf::Elf;
use goblin::elf::dynamic::{DT_NULL, DT_STRSZ, DT_STRTAB};
use std::process;

const PT_LOAD: u32 = 1;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PF_R: u32 = 0x4;

/// Width / endian helper. Encapsulates all the ugly per-field byte
/// arithmetic so the actual algorithms read like pseudocode.
#[derive(Clone, Copy)]
pub struct ElfBits {
    pub is_64: bool,
    pub is_le: bool,
}

impl ElfBits {
    pub fn from(elf: &Elf) -> Self {
        Self {
            is_64: elf.is_64,
            is_le: elf.little_endian,
        }
    }

    /// Size of an Elf_Dyn entry.
    pub fn dyn_size(&self) -> usize {
        if self.is_64 { 16 } else { 8 }
    }

    /// Size of an Elf_Phdr entry.
    #[allow(dead_code)]
    pub fn phdr_size(&self) -> usize {
        if self.is_64 { 56 } else { 32 }
    }

    /// Size of an Elf_Shdr entry.
    #[allow(dead_code)]
    pub fn shdr_size(&self) -> usize {
        if self.is_64 { 64 } else { 40 }
    }

    /// Read a width-correct unsigned word.
    pub fn read_word(&self, data: &[u8], off: usize) -> u64 {
        if self.is_64 {
            self.read_u64(data, off)
        } else {
            self.read_u32(data, off) as u64
        }
    }

    pub fn write_word(&self, data: &mut [u8], off: usize, v: u64) {
        if self.is_64 {
            self.write_u64(data, off, v);
        } else {
            self.write_u32(data, off, v as u32);
        }
    }

    pub fn read_u64(&self, data: &[u8], off: usize) -> u64 {
        let b: [u8; 8] = data[off..off + 8].try_into().unwrap();
        if self.is_le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) }
    }
    pub fn write_u64(&self, data: &mut [u8], off: usize, v: u64) {
        let b = if self.is_le { v.to_le_bytes() } else { v.to_be_bytes() };
        data[off..off + 8].copy_from_slice(&b);
    }
    pub fn read_u32(&self, data: &[u8], off: usize) -> u32 {
        let b: [u8; 4] = data[off..off + 4].try_into().unwrap();
        if self.is_le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }
    }
    pub fn write_u32(&self, data: &mut [u8], off: usize, v: u32) {
        let b = if self.is_le { v.to_le_bytes() } else { v.to_be_bytes() };
        data[off..off + 4].copy_from_slice(&b);
    }

    /// Field offsets within an Elf_Dyn entry.
    /// Returns (d_tag_off, d_val_off, word_size).
    pub fn dyn_fields(&self, base: usize) -> (usize, usize, usize) {
        if self.is_64 { (base, base + 8, 8) } else { (base, base + 4, 4) }
    }

    /// Field offsets within an Elf_Phdr entry.
    /// Returns a struct of relevant offsets.
    pub fn phdr_fields(&self, base: usize) -> PhdrFields {
        if self.is_64 {
            PhdrFields {
                p_type: base,
                p_flags: base + 4,
                p_offset: base + 8,
                p_vaddr: base + 16,
                p_paddr: base + 24,
                p_filesz: base + 32,
                p_memsz: base + 40,
                p_align: base + 48,
            }
        } else {
            // 32-bit phdr: p_type, p_offset, p_vaddr, p_paddr, p_filesz,
            // p_memsz, p_flags, p_align.
            PhdrFields {
                p_type: base,
                p_offset: base + 4,
                p_vaddr: base + 8,
                p_paddr: base + 12,
                p_filesz: base + 16,
                p_memsz: base + 20,
                p_flags: base + 24,
                p_align: base + 28,
            }
        }
    }

    /// Field offsets within an Elf_Shdr entry.
    pub fn shdr_fields(&self, base: usize) -> ShdrFields {
        if self.is_64 {
            ShdrFields {
                sh_addr: base + 16,
                sh_offset: base + 24,
                sh_size: base + 32,
            }
        } else {
            // 32-bit shdr: sh_name(4) sh_type(4) sh_flags(4) sh_addr(4)
            // sh_offset(4) sh_size(4) sh_link(4) sh_info(4) sh_addralign(4)
            // sh_entsize(4).
            ShdrFields {
                sh_addr: base + 12,
                sh_offset: base + 16,
                sh_size: base + 20,
            }
        }
    }
}

pub struct PhdrFields {
    pub p_type: usize,
    pub p_flags: usize,
    pub p_offset: usize,
    pub p_vaddr: usize,
    pub p_paddr: usize,
    pub p_filesz: usize,
    pub p_memsz: usize,
    pub p_align: usize,
}

pub struct ShdrFields {
    pub sh_addr: usize,
    pub sh_offset: usize,
    pub sh_size: usize,
}

/// Grow .dynstr by appending extra bytes.
/// Returns (new_dynstr_vaddr, offset_of_first_appended_byte_within_strtab).
pub fn append_to_dynstr(elf: &Elf, data: &mut Vec<u8>, extra: &[u8]) -> (u64, u32) {
    let bits = ElfBits::from(elf);

    let dyn_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
        .unwrap_or_else(|| {
            eprintln!("patchelf: grow: no PT_DYNAMIC segment found");
            process::exit(1);
        });
    let dyn_off = dyn_phdr.p_offset as usize;
    let dyn_entsize = bits.dyn_size();
    let dynamic = elf.dynamic.as_ref().unwrap_or_else(|| {
        eprintln!("patchelf: grow: no .dynamic");
        process::exit(1);
    });

    let mut dt_strtab_idx = None;
    let mut dt_strsz_idx = None;
    let mut old_strtab_vaddr = 0u64;
    let mut old_strsz = 0u64;
    for (i, e) in dynamic.dyns.iter().enumerate() {
        match e.d_tag {
            DT_STRTAB => { dt_strtab_idx = Some(i); old_strtab_vaddr = e.d_val; }
            DT_STRSZ => { dt_strsz_idx = Some(i); old_strsz = e.d_val; }
            _ => {}
        }
    }
    let dt_strtab_idx = dt_strtab_idx.unwrap_or_else(|| {
        eprintln!("patchelf: grow: no DT_STRTAB");
        process::exit(1);
    });
    let dt_strsz_idx = dt_strsz_idx.unwrap_or_else(|| {
        eprintln!("patchelf: grow: no DT_STRSZ");
        process::exit(1);
    });

    let mut load_phdr_off = None;
    let mut load_offset = 0u64;
    let mut load_filesz = 0u64;
    let mut load_vaddr = 0u64;
    for (i, ph) in elf.program_headers.iter().enumerate() {
        if ph.p_type == PT_LOAD
            && old_strtab_vaddr >= ph.p_vaddr
            && old_strtab_vaddr < ph.p_vaddr + ph.p_memsz
        {
            load_phdr_off = Some(elf.header.e_phoff as usize + i * elf.header.e_phentsize as usize);
            load_offset = ph.p_offset;
            load_filesz = ph.p_filesz;
            load_vaddr = ph.p_vaddr;
            break;
        }
    }
    let load_phdr_off = load_phdr_off.unwrap_or_else(|| {
        eprintln!("patchelf: grow: DT_STRTAB not in any PT_LOAD");
        process::exit(1);
    });

    let load_end_off = load_offset + load_filesz;
    let mut next_load_off = u64::MAX;
    for ph in &elf.program_headers {
        if ph.p_type == PT_LOAD && ph.p_offset > load_offset && ph.p_offset < next_load_off {
            next_load_off = ph.p_offset;
        }
    }
    if next_load_off == u64::MAX {
        eprintln!("patchelf: grow: no second LOAD; cannot place enlarged dynstr");
        process::exit(1);
    }

    let new_dynstr_size = old_strsz + extra.len() as u64;
    let old_strtab_off = vaddr_to_offset(elf, old_strtab_vaddr).unwrap_or_else(|| {
        eprintln!("patchelf: grow: cannot map old DT_STRTAB to file offset");
        process::exit(1);
    });
    let strtab_bytes: Vec<u8> = data[old_strtab_off..old_strtab_off + old_strsz as usize].to_vec();

    let strtab_dyn = bits.dyn_fields(dyn_off + dt_strtab_idx * dyn_entsize);
    let strsz_dyn = bits.dyn_fields(dyn_off + dt_strsz_idx * dyn_entsize);
    let load_pf = bits.phdr_fields(load_phdr_off);

    let new_dynstr_off_aligned = (load_end_off + 7) & !7u64;
    if new_dynstr_off_aligned + new_dynstr_size <= next_load_off {
        // Fast path: park the enlarged dynstr in the LOAD #1 slack.
        let new_dynstr_off = new_dynstr_off_aligned;
        data[new_dynstr_off as usize..new_dynstr_off as usize + old_strsz as usize]
            .copy_from_slice(&strtab_bytes);
        data[new_dynstr_off as usize + old_strsz as usize
            ..new_dynstr_off as usize + new_dynstr_size as usize]
            .copy_from_slice(extra);

        let new_dynstr_vaddr = load_vaddr + (new_dynstr_off - load_offset);
        bits.write_word(data, strtab_dyn.1, new_dynstr_vaddr);
        bits.write_word(data, strsz_dyn.1, new_dynstr_size);

        let new_load_filesz = (new_dynstr_off + new_dynstr_size) - load_offset;
        bits.write_word(data, load_pf.p_filesz, new_load_filesz);
        let cur_memsz = bits.read_word(data, load_pf.p_memsz);
        if new_load_filesz > cur_memsz {
            bits.write_word(data, load_pf.p_memsz, new_load_filesz);
        }
        update_dynstr_section_header(
            data, &elf.header, &bits, old_strtab_off, new_dynstr_off, new_dynstr_size,
        );
        return (new_dynstr_vaddr, old_strsz as u32);
    }

    // Fallback: repurpose a benign phdr slot (PT_GNU_STACK or PT_NULL)
    // into a fresh PT_LOAD covering the new dynstr at end-of-file.
    const PT_NULL: u32 = 0;
    let mut spare_phdr_off = None;
    for (i, ph) in elf.program_headers.iter().enumerate() {
        if ph.p_type == PT_GNU_STACK || ph.p_type == PT_NULL {
            spare_phdr_off = Some(
                elf.header.e_phoff as usize + i * elf.header.e_phentsize as usize,
            );
            break;
        }
    }
    let gnu_stack_phdr_off = spare_phdr_off.unwrap_or_else(|| {
        let avail = next_load_off.saturating_sub(new_dynstr_off_aligned);
        eprintln!(
            "patchelf: grow: insufficient slack at end of first LOAD ({} bytes, need {}) and no PT_GNU_STACK / PT_NULL to repurpose",
            avail, new_dynstr_size
        );
        process::exit(1);
    });

    let page = 0x1000u64;
    let mut top_vaddr = 0u64;
    for ph in &elf.program_headers {
        if ph.p_type == PT_LOAD {
            top_vaddr = top_vaddr.max(ph.p_vaddr + ph.p_memsz);
        }
    }
    top_vaddr = (top_vaddr + page - 1) & !(page - 1);

    let cur_len = data.len() as u64;
    let new_off = (cur_len + page - 1) & !(page - 1);
    while (data.len() as u64) < new_off { data.push(0); }
    let pad_to_align = (page - (data.len() as u64 % page)) % page;
    for _ in 0..pad_to_align { data.push(0); }
    let new_dynstr_off = data.len() as u64;
    data.extend_from_slice(&strtab_bytes);
    data.extend_from_slice(extra);
    let memsz_aligned = (new_dynstr_size + page - 1) & !(page - 1);
    while ((data.len() as u64) - new_dynstr_off) < memsz_aligned { data.push(0); }

    let gs = bits.phdr_fields(gnu_stack_phdr_off);
    bits.write_u32(data, gs.p_type, PT_LOAD);
    bits.write_u32(data, gs.p_flags, PF_R);
    bits.write_word(data, gs.p_offset, new_dynstr_off);
    bits.write_word(data, gs.p_vaddr, top_vaddr);
    bits.write_word(data, gs.p_paddr, top_vaddr);
    bits.write_word(data, gs.p_filesz, new_dynstr_size);
    bits.write_word(data, gs.p_memsz, memsz_aligned);
    bits.write_word(data, gs.p_align, page);

    bits.write_word(data, strtab_dyn.1, top_vaddr);
    bits.write_word(data, strsz_dyn.1, new_dynstr_size);
    update_dynstr_section_header(
        data, &elf.header, &bits, old_strtab_off, new_dynstr_off, new_dynstr_size,
    );

    (top_vaddr, old_strsz as u32)
}

fn vaddr_to_offset(elf: &Elf, vaddr: u64) -> Option<usize> {
    for ph in &elf.program_headers {
        if ph.p_type == PT_LOAD && vaddr >= ph.p_vaddr && vaddr < ph.p_vaddr + ph.p_memsz {
            return Some((vaddr - ph.p_vaddr + ph.p_offset) as usize);
        }
    }
    None
}

fn update_dynstr_section_header(
    data: &mut [u8],
    header: &goblin::elf::Header,
    bits: &ElfBits,
    old_off: usize,
    new_off: u64,
    new_size: u64,
) {
    let shoff = header.e_shoff as usize;
    let entsize = header.e_shentsize as usize;
    let nshdr = header.e_shnum as usize;
    for i in 0..nshdr {
        let sh = shoff + i * entsize;
        let sf = bits.shdr_fields(sh);
        let sh_offset = bits.read_word(data, sf.sh_offset);
        if sh_offset as usize == old_off {
            bits.write_word(data, sf.sh_addr, new_off);
            bits.write_word(data, sf.sh_offset, new_off);
            bits.write_word(data, sf.sh_size, new_size);
            return;
        }
    }
}

/// Find the first DT_NULL slot in .dynamic and overwrite it with
/// (d_tag, d_val). Returns true on success.
pub fn add_dynamic_entry(elf: &Elf, data: &mut [u8], d_tag: u64, d_val: u64) -> bool {
    let bits = ElfBits::from(elf);
    let dyn_phdr = match elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
    {
        Some(p) => p,
        None => return false,
    };
    let dyn_off = dyn_phdr.p_offset as usize;
    let dyn_entsize = bits.dyn_size();
    let dynamic = match &elf.dynamic {
        Some(d) => d,
        None => return false,
    };
    for (idx, entry) in dynamic.dyns.iter().enumerate() {
        if entry.d_tag == DT_NULL {
            let entry_off = dyn_off + idx * dyn_entsize;
            let (tag_off, val_off, _) = bits.dyn_fields(entry_off);
            bits.write_word(data, tag_off, d_tag);
            bits.write_word(data, val_off, d_val);
            return true;
        }
    }
    false
}
