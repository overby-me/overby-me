//! RISC-V linker shared types and utilities.
//!
//! This module provides all the building blocks used by `link.rs` for both
//! executable and shared library linking:
//!
//! - **Relocation constants**: All `R_RISCV_*` values from the ELF psABI spec.
//! - **Instruction patching**: `patch_{u,i,s,b,j,cb,cj}_type` functions that
//!   encode resolved addresses into RISC-V instruction formats.
//! - **Symbol resolution**: `resolve_symbol_value`, `got_sym_key`,
//!   `find_hi20_value` for AUIPC/LO12 relocation pairing.
//! - **Shared types**: `GlobalSym`, `MergedSection`, `InputSecRef`.
//! - **ELF writing helpers** (re-exported from `linker_common`): `write_shdr`,
//!   `write_phdr`, `write_phdr_at`, `align_up`, `pad_to`.
//! - **Utility functions**: `build_gnu_hash`, `find_versioned_soname`,
//!   `resolve_archive_members`, `output_section_name`, `section_order`.

use super::elf_read::*;
use std::collections::HashMap;

// ── RISC-V relocation type constants ─────────────────────────────────────
// Values from the RISC-V ELF psABI specification.

pub const R_RISCV_32: u32 = 1;
pub const R_RISCV_64: u32 = 2;
pub const R_RISCV_BRANCH: u32 = 16;
pub const R_RISCV_JAL: u32 = 17;
pub const R_RISCV_CALL_PLT: u32 = 19;
pub const R_RISCV_GOT_HI20: u32 = 20;
pub const R_RISCV_TLS_GOT_HI20: u32 = 21;
pub const R_RISCV_TLS_GD_HI20: u32 = 22;
pub const R_RISCV_PCREL_HI20: u32 = 23;
pub const R_RISCV_PCREL_LO12_I: u32 = 24;
pub const R_RISCV_PCREL_LO12_S: u32 = 25;
pub const R_RISCV_HI20: u32 = 26;
pub const R_RISCV_LO12_I: u32 = 27;
pub const R_RISCV_LO12_S: u32 = 28;
pub const R_RISCV_TPREL_HI20: u32 = 29;
pub const R_RISCV_TPREL_LO12_I: u32 = 30;
pub const R_RISCV_TPREL_LO12_S: u32 = 31;
pub const R_RISCV_TPREL_ADD: u32 = 32;
pub const R_RISCV_ADD8: u32 = 33;
pub const R_RISCV_ADD16: u32 = 34;
pub const R_RISCV_ADD32: u32 = 35;
pub const R_RISCV_ADD64: u32 = 36;
pub const R_RISCV_SUB8: u32 = 37;
pub const R_RISCV_SUB16: u32 = 38;
pub const R_RISCV_SUB32: u32 = 39;
pub const R_RISCV_SUB64: u32 = 40;
pub const R_RISCV_ALIGN: u32 = 43;
pub const R_RISCV_RVC_BRANCH: u32 = 44;
pub const R_RISCV_RVC_JUMP: u32 = 45;
pub const R_RISCV_RELAX: u32 = 51;
pub const R_RISCV_SET6: u32 = 53;
pub const R_RISCV_SUB6: u32 = 52;
pub const R_RISCV_SET8: u32 = 54;
pub const R_RISCV_SET16: u32 = 55;
pub const R_RISCV_SET32: u32 = 56;
pub const R_RISCV_32_PCREL: u32 = 57;
pub const R_RISCV_SET_ULEB128: u32 = 60;
pub const R_RISCV_SUB_ULEB128: u32 = 61;

// ── Instruction patching ─────────────────────────────────────────────────

/// Patch a U-type instruction (LUI/AUIPC) with a 20-bit immediate.
pub fn patch_u_type(data: &mut [u8], off: usize, value: u32) {
    if off + 4 > data.len() {
        return;
    }
    let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    let hi = (value.wrapping_add(0x800)) & 0xFFFFF000;
    let insn = (insn & 0xFFF) | hi;
    data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
}

/// Patch an I-type instruction with a 12-bit immediate.
pub fn patch_i_type(data: &mut [u8], off: usize, value: u32) {
    if off + 4 > data.len() {
        return;
    }
    let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    let imm = value & 0xFFF;
    let insn = (insn & 0x000FFFFF) | (imm << 20);
    data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
}

/// Patch an S-type instruction with a 12-bit immediate.
pub fn patch_s_type(data: &mut [u8], off: usize, value: u32) {
    if off + 4 > data.len() {
        return;
    }
    let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    let imm = value & 0xFFF;
    let imm_hi = (imm >> 5) & 0x7F;
    let imm_lo = imm & 0x1F;
    let insn = (insn & 0x01FFF07F) | (imm_hi << 25) | (imm_lo << 7);
    data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
}

/// Patch a B-type instruction with a 13-bit PC-relative offset.
pub fn patch_b_type(data: &mut [u8], off: usize, value: u32) {
    if off + 4 > data.len() {
        return;
    }
    let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    let imm = value;
    let bit12 = (imm >> 12) & 1;
    let bits10_5 = (imm >> 5) & 0x3F;
    let bits4_1 = (imm >> 1) & 0xF;
    let bit11 = (imm >> 11) & 1;
    let insn =
        (insn & 0x01FFF07F) | (bit12 << 31) | (bits10_5 << 25) | (bits4_1 << 8) | (bit11 << 7);
    data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
}

/// Patch a J-type instruction with a 21-bit PC-relative offset.
pub fn patch_j_type(data: &mut [u8], off: usize, value: u32) {
    if off + 4 > data.len() {
        return;
    }
    let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    let imm = value;
    let bit20 = (imm >> 20) & 1;
    let bits10_1 = (imm >> 1) & 0x3FF;
    let bit11 = (imm >> 11) & 1;
    let bits19_12 = (imm >> 12) & 0xFF;
    let insn =
        (insn & 0xFFF) | (bit20 << 31) | (bits10_1 << 21) | (bit11 << 20) | (bits19_12 << 12);
    data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
}

/// Patch a CB-type (compressed branch) instruction with an 8-bit signed offset.
/// Used for c.beqz/c.bnez (R_RISCV_RVC_BRANCH).
pub fn patch_cb_type(data: &mut [u8], off: usize, value: u32) {
    if off + 2 > data.len() {
        return;
    }
    let insn = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
    let imm = value;
    let bit8 = (imm >> 8) & 1;
    let bits4_3 = (imm >> 3) & 0x3;
    let bits7_6 = (imm >> 6) & 0x3;
    let bits2_1 = (imm >> 1) & 0x3;
    let bit5 = (imm >> 5) & 1;
    let insn = (insn & 0xE383)
        | ((bit8 as u16) << 12)
        | ((bits4_3 as u16) << 10)
        | ((bits7_6 as u16) << 5)
        | ((bits2_1 as u16) << 3)
        | ((bit5 as u16) << 2);
    data[off..off + 2].copy_from_slice(&insn.to_le_bytes());
}

/// Patch a CJ-type (compressed jump) instruction with an 11-bit signed offset.
/// Used for c.j/c.jal (R_RISCV_RVC_JUMP).
pub fn patch_cj_type(data: &mut [u8], off: usize, value: u32) {
    if off + 2 > data.len() {
        return;
    }
    let insn = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
    let imm = value;
    let bit11 = (imm >> 11) & 1;
    let bit4 = (imm >> 4) & 1;
    let bits9_8 = (imm >> 8) & 0x3;
    let bit10 = (imm >> 10) & 1;
    let bit6 = (imm >> 6) & 1;
    let bit7 = (imm >> 7) & 1;
    let bits3_1 = (imm >> 1) & 0x7;
    let bit5 = (imm >> 5) & 1;
    let encoded = (bit11 << 10)
        | (bit4 << 9)
        | (bits9_8 << 7)
        | (bit10 << 6)
        | (bit6 << 5)
        | (bit7 << 4)
        | (bits3_1 << 1)
        | bit5;
    let insn = (insn & 0xE003) | ((encoded as u16) << 2);
    data[off..off + 2].copy_from_slice(&insn.to_le_bytes());
}

// ── Symbol resolution helpers ────────────────────────────────────────────

/// Resolve a symbol to its virtual address, handling section symbols, globals,
/// and locals.
pub fn resolve_symbol_value(
    sym: &Symbol,
    sym_idx: usize,
    obj: &ElfObject,
    obj_idx: usize,
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    local_sym_vaddrs: &[Vec<u64>],
    global_syms: &HashMap<String, GlobalSym>,
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        if (sym.shndx as usize) < obj.sections.len() {
            if let Some(&(mi, mo)) = sec_mapping.get(&(obj_idx, sym.shndx as usize)) {
                return section_vaddrs[mi] + mo;
            }
        }
        0
    } else if !sym.name.is_empty() && sym.binding() != STB_LOCAL {
        global_syms.get(&sym.name).map(|gs| gs.value).unwrap_or(0)
    } else {
        local_sym_vaddrs
            .get(obj_idx)
            .and_then(|v| v.get(sym_idx))
            .copied()
            .unwrap_or(0)
    }
}

/// Build a unique GOT key for a symbol referenced via GOT_HI20 or TLS_GOT_HI20.
///
/// Local and section symbols aren't in `global_syms`, so they need synthetic keys
/// that incorporate the object index to avoid collisions between objects. The addend
/// is included because different offsets within the same section need distinct GOT
/// entries pointing to different addresses.
///
/// Returns (key, is_local) where is_local indicates the symbol won't be in global_syms.
pub fn got_sym_key(obj_idx: usize, sym: &Symbol, addend: i64) -> (String, bool) {
    if sym.sym_type() == STT_SECTION {
        (
            format!("__local_sec_{}_{}_{}", obj_idx, sym.shndx, addend),
            true,
        )
    } else if sym.binding() == STB_LOCAL {
        (format!("__local_{}_{}_{}", obj_idx, sym.name, addend), true)
    } else {
        (sym.name.clone(), false)
    }
}

/// Find the hi20 value for a pcrel_lo12 relocation in executable linking.
///
/// RISC-V uses paired AUIPC+LO12 relocations. A pcrel_lo12 references the AUIPC
/// instruction by its address; this function scans the relocation table to find
/// the corresponding HI20 relocation and computes the low 12 bits of the resolved
/// address.
pub fn find_hi20_value(
    obj: &ElfObject,
    obj_idx: usize,
    sec_idx: usize,
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    local_sym_vaddrs: &[Vec<u64>],
    global_syms: &HashMap<String, GlobalSym>,
    auipc_vaddr: u64,
    sec_offset: u64,
    got_vaddr: u64,
    got_symbols: &[String],
    got_plt_vaddr: u64,
    gd_tls_relax_info: &HashMap<u64, (u64, i64)>,
    tls_vaddr: u64,
) -> i64 {
    // Check if this references a GD->LE relaxed auipc (now a lui)
    if let Some(&(sym_val, addend)) = gd_tls_relax_info.get(&auipc_vaddr) {
        let tprel = (sym_val as i64 + addend - tls_vaddr as i64) as u32;
        return (tprel & 0xFFF) as i64;
    }

    find_hi20_value_core(
        obj,
        obj_idx,
        sec_idx,
        sec_mapping,
        section_vaddrs,
        local_sym_vaddrs,
        global_syms,
        auipc_vaddr,
        sec_offset,
        got_vaddr,
        got_symbols,
        Some(got_plt_vaddr),
    )
}

/// Find the hi20 value for a pcrel_lo12 relocation in shared library linking.
///
/// Simplified variant without GD->LE relaxation (not valid in shared libs)
/// and without PLT GOT entries.
pub fn find_hi20_value_shared(
    obj: &ElfObject,
    obj_idx: usize,
    sec_idx: usize,
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    local_sym_vaddrs: &[Vec<u64>],
    global_syms: &HashMap<String, GlobalSym>,
    auipc_vaddr: u64,
    sec_offset: u64,
    got_vaddr: u64,
    got_symbols: &[String],
) -> i64 {
    find_hi20_value_core(
        obj,
        obj_idx,
        sec_idx,
        sec_mapping,
        section_vaddrs,
        local_sym_vaddrs,
        global_syms,
        auipc_vaddr,
        sec_offset,
        got_vaddr,
        got_symbols,
        None,
    )
}

/// Core implementation for finding the hi20 value paired with a pcrel_lo12.
fn find_hi20_value_core(
    obj: &ElfObject,
    obj_idx: usize,
    sec_idx: usize,
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    local_sym_vaddrs: &[Vec<u64>],
    global_syms: &HashMap<String, GlobalSym>,
    auipc_vaddr: u64,
    sec_offset: u64,
    got_vaddr: u64,
    got_symbols: &[String],
    got_plt_vaddr: Option<u64>,
) -> i64 {
    if sec_idx >= obj.relocations.len() {
        return 0;
    }
    let relocs = &obj.relocations[sec_idx];
    for reloc in relocs {
        let reloc_vaddr = sec_offset + reloc.offset;
        let (mi, _mo) = match sec_mapping.get(&(obj_idx, sec_idx)) {
            Some(&v) => v,
            None => continue,
        };
        let this_vaddr = section_vaddrs[mi] + reloc_vaddr;
        if this_vaddr != auipc_vaddr {
            continue;
        }

        match reloc.rela_type {
            R_RISCV_PCREL_HI20 => {
                let hi_sym_idx = reloc.sym_idx as usize;
                let sym = &obj.symbols[hi_sym_idx];
                let s = resolve_symbol_value(
                    sym,
                    hi_sym_idx,
                    obj,
                    obj_idx,
                    sec_mapping,
                    section_vaddrs,
                    local_sym_vaddrs,
                    global_syms,
                );
                let target = s as i64 + reloc.addend;
                return (target - auipc_vaddr as i64) & 0xFFF;
            }
            R_RISCV_GOT_HI20 | R_RISCV_TLS_GOT_HI20 => {
                let hi_sym_idx = reloc.sym_idx as usize;
                let sym = &obj.symbols[hi_sym_idx];
                let (sym_name, _) = got_sym_key(obj_idx, sym, reloc.addend);

                // Executable path: check global_syms first (for got_offset or PLT),
                // then fall back to got_symbols list.
                // Shared lib path: check got_symbols first, then global_syms.
                let got_entry_vaddr = if let Some(gpv) = got_plt_vaddr {
                    // Executable path
                    if let Some(gs) = global_syms.get(&sym.name) {
                        if let Some(got_off) = gs.got_offset {
                            got_vaddr + got_off
                        } else {
                            // PLT symbol: use GOT.PLT
                            gpv + (2 + gs.plt_idx) as u64 * 8
                        }
                    } else if let Some(idx) = got_symbols.iter().position(|n| n == &sym_name) {
                        got_vaddr + idx as u64 * 8
                    } else {
                        0
                    }
                } else {
                    // Shared lib path: got_symbols first, then global_syms
                    if let Some(idx) = got_symbols.iter().position(|n| n == &sym_name) {
                        got_vaddr + idx as u64 * 8
                    } else if let Some(gs) = global_syms.get(&sym.name) {
                        if let Some(got_off) = gs.got_offset {
                            got_vaddr + got_off
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                };

                return (got_entry_vaddr as i64 + reloc.addend - auipc_vaddr as i64) & 0xFFF;
            }
            _ => {}
        }
    }
    0
}

/// Represents a global symbol's definition, used by both executable and shared
/// library linking.
#[derive(Clone, Debug)]
pub struct GlobalSym {
    /// Virtual address of the symbol.
    pub value: u64,
    pub size: u64,
    pub binding: u8,
    pub sym_type: u8,
    pub visibility: u8,
    /// True if this symbol is defined in an input object file.
    pub defined: bool,
    /// True if this symbol needs a PLT/GOT entry (from a shared library).
    pub needs_plt: bool,
    /// Index into the PLT (if needs_plt).
    pub plt_idx: usize,
    /// GOT offset (relative to GOT base) if this symbol has a GOT entry.
    pub got_offset: Option<u64>,
    /// Section index in the merged section list, if defined locally.
    pub section_idx: Option<usize>,
}

/// A merged input section with its assigned virtual address.
pub struct MergedSection {
    pub name: String,
    pub sh_type: u32,
    pub sh_flags: u64,
    pub data: Vec<u8>,
    pub vaddr: u64,
    pub align: u64,
}

/// Tracks where each input section maps to in the merged output.
pub struct InputSecRef {
    pub obj_idx: usize,
    pub sec_idx: usize,
    pub merged_sec_idx: usize,
    pub offset_in_merged: u64,
}

// ── Section name mapping ─────────────────────────────────────────────────

/// Map an input section name to its output section name.
///
/// Returns None for sections that should be skipped (non-alloc, group sections).
/// Handles RISC-V-specific sections (.sdata, .sbss, .riscv.attributes) in
/// addition to standard ELF section name folding (.text.foo -> .text, etc).
pub fn output_section_name(name: &str, sh_type: u32, sh_flags: u64) -> Option<String> {
    // Skip non-alloc sections (except .riscv.attributes which we handle separately)
    if sh_flags & SHF_ALLOC == 0 && sh_type != SHT_RISCV_ATTRIBUTES {
        return None;
    }
    if sh_type == SHT_GROUP {
        return None;
    }

    // Standard section name folding: .text.foo -> .text, etc.
    let prefixes = [
        (".text.", ".text"),
        (".rodata.", ".rodata"),
        (".data.rel.ro", ".data"),
        (".data.", ".data"),
        (".bss.", ".bss"),
        (".tdata.", ".tdata"),
        (".tbss.", ".tbss"),
        // RISC-V specific: small data/bss sections
        (".sdata.", ".sdata"),
        (".sbss.", ".sbss"),
    ];

    for (prefix, output) in &prefixes {
        if name.starts_with(prefix) {
            return Some(output.to_string());
        }
    }

    // Array sections: fold numbered variants (.init_array.65535 -> .init_array)
    for arr in &[".init_array", ".fini_array", ".preinit_array"] {
        if name == *arr || name.starts_with(&format!("{}.", arr)) {
            return Some(arr.to_string());
        }
    }

    Some(name.to_string())
}

// ── ELF writing helpers ──────────────────────────────────────────────────
// Re-exported from linker_common for backward compatibility with link.rs imports.

pub use crate::backend::linker_common::align_up_64 as align_up;
pub use crate::backend::linker_common::pad_to;
pub use crate::backend::linker_common::write_elf64_phdr as write_phdr;
pub use crate::backend::linker_common::write_elf64_phdr_at as write_phdr_at;
pub use crate::backend::linker_common::write_elf64_shdr as write_shdr;

/// Build a proper `.gnu.hash` section for the given dynamic symbol names.
///
/// Returns `(hash_data, sorted_order)` where `sorted_order[i]` is the original
/// index in `sym_names` for the i-th symbol in the reordered dynsym.
/// The dynsym must be reordered according to `sorted_order` for the hash table
/// to work correctly.
///
/// `sym_names` should NOT include the null symbol at index 0.
pub fn build_gnu_hash(sym_names: &[String]) -> (Vec<u8>, Vec<usize>) {
    let nsyms = sym_names.len();
    if nsyms == 0 {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes()); // nbuckets
        data.extend_from_slice(&1u32.to_le_bytes()); // symoffset
        data.extend_from_slice(&1u32.to_le_bytes()); // bloom_size
        data.extend_from_slice(&6u32.to_le_bytes()); // bloom_shift
        data.extend_from_slice(&0u64.to_le_bytes()); // bloom[0]
        data.extend_from_slice(&0u32.to_le_bytes()); // bucket[0]
        return (data, Vec::new());
    }

    // Compute hashes for all symbols
    let hashes: Vec<u32> = sym_names
        .iter()
        .map(|n| crate::backend::linker_common::gnu_hash(n.as_bytes()))
        .collect();

    // Choose number of buckets: roughly nsyms, but at least 1
    let nbuckets = nsyms.max(1) as u32;

    // Sort symbols by bucket (hash % nbuckets), preserving relative order
    let mut indices: Vec<usize> = (0..nsyms).collect();
    indices.sort_by_key(|&i| hashes[i] % nbuckets);

    // symoffset = 1 (first hashed symbol is at dynsym index 1, after null entry)
    let symoffset = 1u32;

    // Build bloom filter.  ELF64 uses 64-bit bloom words.
    let bloom_shift = 6u32;
    let bloom_size = (nsyms / 2).max(1).next_power_of_two() as u32;
    let mut bloom: Vec<u64> = vec![0u64; bloom_size as usize];
    let c = 64u32; // bits per bloom word for ELF64
    for &idx in &indices {
        let h = hashes[idx];
        let word_idx = ((h / c) % bloom_size) as usize;
        let bit1 = (h % c) as u64;
        let bit2 = ((h >> bloom_shift) % c) as u64;
        bloom[word_idx] |= (1u64 << bit1) | (1u64 << bit2);
    }

    // Build buckets and hash chain.
    // bucket[b] = first dynsym index in this bucket, or 0 if empty.
    // chain[i] = hash value with low bit = 1 if last in chain.
    let mut buckets: Vec<u32> = vec![0u32; nbuckets as usize];
    let mut chain: Vec<u32> = vec![0u32; nsyms];

    for (pos, &orig_idx) in indices.iter().enumerate() {
        let h = hashes[orig_idx];
        let bucket = (h % nbuckets) as usize;
        let dynsym_idx = (pos as u32) + symoffset;
        if buckets[bucket] == 0 {
            buckets[bucket] = dynsym_idx;
        }
        // Store hash with low bit cleared (we set it for chain terminators below)
        chain[pos] = h & !1;
    }

    // Set chain terminator bits: the last symbol in each bucket gets bit 0 set.
    // Walk backwards through the sorted symbols. When the next symbol has a
    // different bucket (or we're at the end), the current one is a chain end.
    for pos in (0..nsyms).rev() {
        let h = hashes[indices[pos]];
        let bucket = h % nbuckets;
        let is_last = if pos + 1 >= nsyms {
            true
        } else {
            let next_bucket = hashes[indices[pos + 1]] % nbuckets;
            next_bucket != bucket
        };
        if is_last {
            chain[pos] |= 1;
        }
    }

    // Serialize
    let mut data = Vec::new();
    data.extend_from_slice(&nbuckets.to_le_bytes());
    data.extend_from_slice(&symoffset.to_le_bytes());
    data.extend_from_slice(&bloom_size.to_le_bytes());
    data.extend_from_slice(&bloom_shift.to_le_bytes());
    for &word in &bloom {
        data.extend_from_slice(&word.to_le_bytes());
    }
    for &b in &buckets {
        data.extend_from_slice(&b.to_le_bytes());
    }
    for &c in &chain {
        data.extend_from_slice(&c.to_le_bytes());
    }

    (data, indices)
}

/// Find a versioned soname for a library (e.g., "c" -> "libc.so.6").
/// Returns None if only an unversioned .so is found.
pub fn find_versioned_soname(dir: &str, libname: &str) -> Option<String> {
    let pattern = format!("lib{}.so.", libname);
    let mut best: Option<String> = None;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().into_owned();
            if name_str.starts_with(&pattern)
                && (best.is_none() || name_str.len() < best.as_ref().unwrap().len())
            {
                best = Some(name_str);
            }
        }
    }
    best
}

/// Resolve archive members: iteratively pull in members that define currently-undefined symbols.
pub fn resolve_archive_members(
    members: Vec<(String, ElfObject)>,
    input_objs: &mut Vec<(String, ElfObject)>,
    defined_syms: &mut std::collections::HashSet<String>,
    undefined_syms: &mut std::collections::HashSet<String>,
) {
    let mut pool = members;
    loop {
        let mut added_any = false;
        let mut remaining = Vec::new();
        for (name, obj) in pool {
            let needed = obj.symbols.iter().any(|sym| {
                sym.shndx != SHN_UNDEF
                    && sym.binding() != STB_LOCAL
                    && !sym.name.is_empty()
                    && undefined_syms.contains(&sym.name)
            });
            if needed {
                for sym in &obj.symbols {
                    if sym.shndx != SHN_UNDEF && sym.binding() != STB_LOCAL && !sym.name.is_empty()
                    {
                        defined_syms.insert(sym.name.clone());
                        undefined_syms.remove(&sym.name);
                    }
                }
                for sym in &obj.symbols {
                    if sym.shndx == SHN_UNDEF
                        && !sym.name.is_empty()
                        && sym.binding() != STB_LOCAL
                        && !defined_syms.contains(&sym.name)
                    {
                        undefined_syms.insert(sym.name.clone());
                    }
                }
                input_objs.push((name, obj));
                added_any = true;
            } else {
                remaining.push((name, obj));
            }
        }
        if !added_any || remaining.is_empty() {
            break;
        }
        pool = remaining;
    }
}

/// Canonical output section ordering for layout.
pub fn section_order(name: &str, flags: u64) -> u64 {
    match name {
        ".text" => 100,
        ".rodata" => 200,
        ".eh_frame_hdr" => 250,
        ".eh_frame" => 260,
        ".preinit_array" => 500,
        ".init_array" => 510,
        ".fini_array" => 520,
        ".data" => 600,
        ".sdata" => 650,
        ".bss" | ".sbss" => 700,
        _ if flags & SHF_EXECINSTR != 0 => 150,
        _ if flags & SHF_WRITE == 0 => 300,
        _ => 600,
    }
}

/// Decode a ULEB128 value from data at offset.
#[allow(dead_code)] // Infrastructure for RISC-V linker relaxation (R_RISCV_SET_ULEB128/R_RISCV_SUB_ULEB128)
pub fn decode_uleb128(data: &[u8], off: usize) -> u64 {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    let mut i = off;
    while i < data.len() {
        let byte = data[i];
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        i += 1;
    }
    result
}

/// Encode a ULEB128 value in place, reusing the same number of bytes as the
/// existing ULEB128 at that offset.
#[allow(dead_code)] // Infrastructure for RISC-V linker relaxation (R_RISCV_SET_ULEB128/R_RISCV_SUB_ULEB128)
pub fn encode_uleb128_in_place(data: &mut [u8], off: usize, value: u64) {
    // Count how many bytes the existing ULEB128 occupies.
    let mut num_bytes = 0;
    let mut i = off;
    while i < data.len() {
        num_bytes += 1;
        if data[i] & 0x80 == 0 {
            break;
        }
        i += 1;
    }
    // Encode the new value in the same number of bytes.
    let mut val = value;
    for j in 0..num_bytes {
        let idx = off + j;
        if idx >= data.len() {
            break;
        }
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        if j < num_bytes - 1 {
            byte |= 0x80;
        }
        data[idx] = byte;
    }
}
