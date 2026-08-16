//! Phase 6: Relocation application.
//!
//! Applies RISC-V relocations to merged section data. Handles all relocation
//! types from the RISC-V ELF psABI, including PC-relative, GOT, TLS, and
//! compressed instruction relocations.
//!
//! Shared between executable and shared library linking, with context-specific
//! behavior controlled by `RelocContext`.

use super::elf_read::*;
use super::relocations::{
    got_sym_key, patch_b_type, patch_cb_type, patch_cj_type, patch_i_type, patch_j_type,
    patch_s_type, patch_u_type, resolve_symbol_value, GlobalSym, MergedSection, R_RISCV_32,
    R_RISCV_32_PCREL, R_RISCV_64, R_RISCV_ADD16, R_RISCV_ADD32, R_RISCV_ADD64, R_RISCV_ADD8,
    R_RISCV_ALIGN, R_RISCV_BRANCH, R_RISCV_CALL_PLT, R_RISCV_GOT_HI20, R_RISCV_HI20, R_RISCV_JAL,
    R_RISCV_LO12_I, R_RISCV_LO12_S, R_RISCV_PCREL_HI20, R_RISCV_PCREL_LO12_I, R_RISCV_PCREL_LO12_S,
    R_RISCV_RELAX, R_RISCV_RVC_BRANCH, R_RISCV_RVC_JUMP, R_RISCV_SET16, R_RISCV_SET32,
    R_RISCV_SET6, R_RISCV_SET8, R_RISCV_SET_ULEB128, R_RISCV_SUB16, R_RISCV_SUB32, R_RISCV_SUB6,
    R_RISCV_SUB64, R_RISCV_SUB8, R_RISCV_SUB_ULEB128, R_RISCV_TLS_GD_HI20, R_RISCV_TLS_GOT_HI20,
    R_RISCV_TPREL_ADD, R_RISCV_TPREL_HI20, R_RISCV_TPREL_LO12_I, R_RISCV_TPREL_LO12_S,
};
use std::collections::{HashMap, HashSet};

/// Context for relocation application, providing all the resolved addresses
/// and symbol tables needed to patch instructions.
pub struct RelocContext<'a> {
    pub sec_mapping: &'a HashMap<(usize, usize), (usize, u64)>,
    pub section_vaddrs: &'a [u64],
    pub local_sym_vaddrs: &'a [Vec<u64>],
    pub global_syms: &'a HashMap<String, GlobalSym>,
    pub got_vaddr: u64,
    pub got_symbols: &'a [String],
    pub got_plt_vaddr: u64,
    pub tls_vaddr: u64,
    /// For executable linking: GD->LE TLS relaxation info (auipc_vaddr -> (sym_value, addend))
    pub gd_tls_relax_info: &'a HashMap<u64, (u64, i64)>,
    /// For executable linking: vaddrs of __tls_get_addr calls to NOP out
    pub gd_tls_call_nop: &'a HashSet<u64>,
    /// Whether to collect R_RISCV_RELATIVE dynamic relocations (shared lib mode)
    pub collect_relatives: bool,
    /// For shared library linking: additional GOT offset mapping
    pub got_sym_offsets: &'a HashMap<String, u64>,
    /// For shared library linking: PLT stub addresses for undefined symbols
    pub plt_sym_addrs: &'a HashMap<String, u64>,
}

/// Result of applying relocations to one object.
pub struct RelocResult {
    /// Dynamic RELATIVE relocations (offset, addend) collected during shared lib linking
    pub relative_entries: Vec<(u64, u64)>,
}

/// Apply all relocations for all input objects to the merged section data.
pub fn apply_relocations(
    input_objs: &[(String, super::elf_read::ElfObject)],
    merged_sections: &mut [MergedSection],
    ctx: &RelocContext,
) -> Result<RelocResult, String> {
    let mut result = RelocResult {
        relative_entries: Vec::new(),
    };

    for (obj_idx, (obj_name, obj)) in input_objs.iter().enumerate() {
        for (sec_idx, relocs) in obj.relocations.iter().enumerate() {
            if relocs.is_empty() {
                continue;
            }
            let (merged_idx, sec_offset) = match ctx.sec_mapping.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };

            let ms_vaddr = ctx.section_vaddrs[merged_idx];

            for reloc in relocs {
                let offset = sec_offset + reloc.offset;
                let p = ms_vaddr + offset; // PC (address of relocation site)

                let sym_idx = reloc.sym_idx as usize;
                if sym_idx >= obj.symbols.len() {
                    continue;
                }
                let sym = &obj.symbols[sym_idx];

                // Resolve symbol value
                let s = resolve_sym_value(sym, sym_idx, obj_idx, obj, ctx);

                let a = reloc.addend;
                let data = &mut merged_sections[merged_idx].data;
                let off = offset as usize;

                apply_one_reloc(
                    reloc.rela_type,
                    data,
                    off,
                    s,
                    a,
                    p,
                    sym,
                    sym_idx,
                    obj_idx,
                    obj,
                    sec_idx,
                    sec_offset,
                    obj_name,
                    ctx,
                    &mut result,
                )?;
            }
        }
    }

    Ok(result)
}

/// Resolve the value of a symbol for relocation purposes.
fn resolve_sym_value(
    sym: &super::elf_read::Symbol,
    sym_idx: usize,
    obj_idx: usize,
    obj: &super::elf_read::ElfObject,
    ctx: &RelocContext,
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        if (sym.shndx as usize) < obj.sections.len() {
            if let Some(&(mi, mo)) = ctx.sec_mapping.get(&(obj_idx, sym.shndx as usize)) {
                return ctx.section_vaddrs[mi] + mo;
            }
        }
        0
    } else if !sym.name.is_empty() && sym.binding() != STB_LOCAL {
        if let Some(gs) = ctx.global_syms.get(&sym.name) {
            if gs.needs_plt || gs.defined {
                gs.value
            } else {
                0
            }
        } else {
            0
        }
    } else {
        ctx.local_sym_vaddrs
            .get(obj_idx)
            .and_then(|v| v.get(sym_idx))
            .copied()
            .unwrap_or(0)
    }
}

/// Look up a GOT entry address for the given symbol.
fn lookup_got_entry(
    sym: &super::elf_read::Symbol,
    obj_idx: usize,
    addend: i64,
    ctx: &RelocContext,
) -> u64 {
    let (sym_name, _) = got_sym_key(obj_idx, sym, addend);

    // Check got_sym_offsets first (shared lib path)
    if let Some(&got_off) = ctx.got_sym_offsets.get(&sym_name) {
        return ctx.got_vaddr + got_off;
    }

    // Check global symbol's got_offset
    if let Some(gs) = ctx.global_syms.get(&sym.name) {
        if let Some(got_off) = gs.got_offset {
            return ctx.got_vaddr + got_off;
        }
        // PLT symbol: use GOT.PLT
        if gs.needs_plt {
            return ctx.got_plt_vaddr + (2 + gs.plt_idx) as u64 * 8;
        }
    }

    // Search got_symbols by name
    if let Some(idx) = ctx.got_symbols.iter().position(|n| n == &sym_name) {
        return ctx.got_vaddr + idx as u64 * 8;
    }

    0
}

/// Apply a single relocation to the merged section data.
fn apply_one_reloc(
    rela_type: u32,
    data: &mut [u8],
    off: usize,
    s: u64,
    a: i64,
    p: u64,
    sym: &super::elf_read::Symbol,
    _sym_idx: usize,
    obj_idx: usize,
    obj: &super::elf_read::ElfObject,
    sec_idx: usize,
    sec_offset: u64,
    obj_name: &str,
    ctx: &RelocContext,
    result: &mut RelocResult,
) -> Result<(), String> {
    match rela_type {
        R_RISCV_RELAX | R_RISCV_ALIGN => { /* hints, skip */ }

        R_RISCV_64 => {
            let val = (s as i64 + a) as u64;
            if off + 8 <= data.len() {
                data[off..off + 8].copy_from_slice(&val.to_le_bytes());
                if ctx.collect_relatives && s != 0 {
                    result.relative_entries.push((p, val));
                }
            }
        }
        R_RISCV_32 => {
            let val = (s as i64 + a) as u32;
            if off + 4 <= data.len() {
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_PCREL_HI20 => {
            let target = s as i64 + a;
            let offset_val = target - p as i64;
            patch_u_type(data, off, offset_val as u32);
        }
        R_RISCV_PCREL_LO12_I => {
            let auipc_addr = s as i64 + a;
            if let Some(&(sym_val, gd_addend)) = ctx.gd_tls_relax_info.get(&(auipc_addr as u64)) {
                let tprel = (sym_val as i64 + gd_addend - ctx.tls_vaddr as i64) as u32;
                patch_i_type(data, off, tprel & 0xFFF);
            } else {
                let hi_val = find_hi20_value_for_reloc(
                    obj,
                    obj_idx,
                    sec_idx,
                    ctx,
                    auipc_addr as u64,
                    sec_offset,
                );
                patch_i_type(data, off, hi_val as u32);
            }
        }
        R_RISCV_PCREL_LO12_S => {
            let auipc_addr = s as i64 + a;
            if let Some(&(sym_val, gd_addend)) = ctx.gd_tls_relax_info.get(&(auipc_addr as u64)) {
                let tprel = (sym_val as i64 + gd_addend - ctx.tls_vaddr as i64) as u32;
                patch_s_type(data, off, tprel & 0xFFF);
            } else {
                let hi_val = find_hi20_value_for_reloc(
                    obj,
                    obj_idx,
                    sec_idx,
                    ctx,
                    auipc_addr as u64,
                    sec_offset,
                );
                patch_s_type(data, off, hi_val as u32);
            }
        }
        R_RISCV_GOT_HI20 => {
            let got_entry_vaddr = lookup_got_entry(sym, obj_idx, a, ctx);
            let offset_val = got_entry_vaddr as i64 + a - p as i64;
            patch_u_type(data, off, offset_val as u32);
        }
        R_RISCV_CALL_PLT => {
            if ctx.gd_tls_call_nop.contains(&p) {
                // GD->LE relaxation: replace call with add a0, a0, tp + nop
                if off + 8 <= data.len() {
                    let add_insn: u32 = 0x00450533; // add a0, a0, tp
                    data[off..off + 4].copy_from_slice(&add_insn.to_le_bytes());
                    let nop: u32 = 0x00000013;
                    data[off + 4..off + 8].copy_from_slice(&nop.to_le_bytes());
                }
            } else {
                let target = if !sym.name.is_empty() && sym.binding() != STB_LOCAL {
                    // For undefined symbols in shared libs, redirect through PLT stub
                    if let Some(&plt_addr) = ctx.plt_sym_addrs.get(&sym.name) {
                        plt_addr as i64
                    } else {
                        ctx.global_syms
                            .get(&sym.name)
                            .map(|gs| gs.value as i64)
                            .unwrap_or(s as i64)
                    }
                } else {
                    s as i64
                };
                let offset_val = target + a - p as i64;
                let hi = ((offset_val + 0x800) >> 12) & 0xFFFFF;
                let lo = offset_val & 0xFFF;
                if off + 8 <= data.len() {
                    let auipc = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                    let auipc = (auipc & 0xFFF) | ((hi as u32) << 12);
                    data[off..off + 4].copy_from_slice(&auipc.to_le_bytes());
                    let jalr = u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap());
                    let jalr = (jalr & 0x000FFFFF) | ((lo as u32) << 20);
                    data[off + 4..off + 8].copy_from_slice(&jalr.to_le_bytes());
                }
            }
        }
        R_RISCV_BRANCH => {
            let offset_val = (s as i64 + a - p as i64) as u32;
            patch_b_type(data, off, offset_val);
        }
        R_RISCV_JAL => {
            let offset_val = (s as i64 + a - p as i64) as u32;
            patch_j_type(data, off, offset_val);
        }
        R_RISCV_RVC_BRANCH => {
            let offset_val = (s as i64 + a - p as i64) as u32;
            patch_cb_type(data, off, offset_val);
        }
        R_RISCV_RVC_JUMP => {
            let offset_val = (s as i64 + a - p as i64) as u32;
            patch_cj_type(data, off, offset_val);
        }
        R_RISCV_HI20 => {
            let val = (s as i64 + a) as u32;
            let hi = (val.wrapping_add(0x800)) & 0xFFFFF000;
            if off + 4 <= data.len() {
                let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let insn = (insn & 0xFFF) | hi;
                data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
            }
        }
        R_RISCV_LO12_I => {
            let val = (s as i64 + a) as u32;
            patch_i_type(data, off, val & 0xFFF);
        }
        R_RISCV_LO12_S => {
            let val = (s as i64 + a) as u32;
            patch_s_type(data, off, val & 0xFFF);
        }
        R_RISCV_TPREL_HI20 => {
            let val = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            let hi = (val.wrapping_add(0x800)) & 0xFFFFF000;
            if off + 4 <= data.len() {
                let insn = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let insn = (insn & 0xFFF) | hi;
                data[off..off + 4].copy_from_slice(&insn.to_le_bytes());
            }
        }
        R_RISCV_TPREL_LO12_I => {
            let val = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            patch_i_type(data, off, val & 0xFFF);
        }
        R_RISCV_TPREL_LO12_S => {
            let val = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            patch_s_type(data, off, val & 0xFFF);
        }
        R_RISCV_TPREL_ADD => { /* hint */ }

        R_RISCV_ADD8 => {
            if off < data.len() {
                data[off] = data[off].wrapping_add((s as i64 + a) as u8);
            }
        }
        R_RISCV_ADD16 => {
            if off + 2 <= data.len() {
                let cur = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
                let val = cur.wrapping_add((s as i64 + a) as u16);
                data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_ADD32 => {
            if off + 4 <= data.len() {
                let cur = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let val = cur.wrapping_add((s as i64 + a) as u32);
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_ADD64 => {
            if off + 8 <= data.len() {
                let cur = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                let val = cur.wrapping_add((s as i64 + a) as u64);
                data[off..off + 8].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SUB8 => {
            if off < data.len() {
                data[off] = data[off].wrapping_sub((s as i64 + a) as u8);
            }
        }
        R_RISCV_SUB16 => {
            if off + 2 <= data.len() {
                let cur = u16::from_le_bytes(data[off..off + 2].try_into().unwrap());
                let val = cur.wrapping_sub((s as i64 + a) as u16);
                data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SUB32 => {
            if off + 4 <= data.len() {
                let cur = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
                let val = cur.wrapping_sub((s as i64 + a) as u32);
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SUB64 => {
            if off + 8 <= data.len() {
                let cur = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                let val = cur.wrapping_sub((s as i64 + a) as u64);
                data[off..off + 8].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SET6 => {
            if off < data.len() {
                data[off] = (data[off] & 0xC0) | (((s as i64 + a) as u8) & 0x3F);
            }
        }
        R_RISCV_SUB6 => {
            if off < data.len() {
                let cur = data[off] & 0x3F;
                let val = cur.wrapping_sub((s as i64 + a) as u8) & 0x3F;
                data[off] = (data[off] & 0xC0) | val;
            }
        }
        R_RISCV_SET8 => {
            if off < data.len() {
                data[off] = (s as i64 + a) as u8;
            }
        }
        R_RISCV_SET16 => {
            if off + 2 <= data.len() {
                let val = (s as i64 + a) as u16;
                data[off..off + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SET32 => {
            if off + 4 <= data.len() {
                let val = (s as i64 + a) as u32;
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_32_PCREL => {
            if off + 4 <= data.len() {
                let val = ((s as i64 + a) - p as i64) as u32;
                data[off..off + 4].copy_from_slice(&val.to_le_bytes());
            }
        }
        R_RISCV_SET_ULEB128 => {
            apply_set_uleb128(data, off, (s as i64 + a) as u64);
        }
        R_RISCV_SUB_ULEB128 => {
            apply_sub_uleb128(data, off, (s as i64 + a) as u64);
        }
        R_RISCV_TLS_GD_HI20 => {
            // GD->LE relaxation: rewrite auipc -> lui with tprel
            // TODO: In shared libs, this should emit TLS_DTPMOD64/DTPOFF64 dynamic relocs
            let tprel = (s as i64 + a - ctx.tls_vaddr as i64) as u32;
            let hi = tprel.wrapping_add(0x800) & 0xFFFFF000;
            if off + 4 <= data.len() {
                let lui_insn: u32 = 0x00000537 | hi; // lui a0, hi
                data[off..off + 4].copy_from_slice(&lui_insn.to_le_bytes());
            }
        }
        R_RISCV_TLS_GOT_HI20 => {
            let got_entry_vaddr = lookup_got_entry(sym, obj_idx, a, ctx);
            let offset_val = got_entry_vaddr as i64 + a - p as i64;
            patch_u_type(data, off, offset_val as u32);
        }
        other => {
            if !ctx.collect_relatives {
                // Executable mode: unknown relocation is an error
                return Err(format!(
                    "unsupported RISC-V relocation type {} for symbol '{}' in '{}'",
                    other, sym.name, obj_name
                ));
            }
            // Shared lib mode: skip unknown relocations silently
        }
    }

    Ok(())
}

/// Find the hi20 value for a PCREL_LO12 relocation, dispatching to the
/// appropriate variant based on whether we're building an executable or shared lib.
fn find_hi20_value_for_reloc(
    obj: &super::elf_read::ElfObject,
    obj_idx: usize,
    sec_idx: usize,
    ctx: &RelocContext,
    auipc_addr: u64,
    sec_offset: u64,
) -> i64 {
    if ctx.collect_relatives {
        super::relocations::find_hi20_value_shared(
            obj,
            obj_idx,
            sec_idx,
            ctx.sec_mapping,
            ctx.section_vaddrs,
            ctx.local_sym_vaddrs,
            ctx.global_syms,
            auipc_addr,
            sec_offset,
            ctx.got_vaddr,
            ctx.got_symbols,
        )
    } else {
        super::relocations::find_hi20_value(
            obj,
            obj_idx,
            sec_idx,
            ctx.sec_mapping,
            ctx.section_vaddrs,
            ctx.local_sym_vaddrs,
            ctx.global_syms,
            auipc_addr,
            sec_offset,
            ctx.got_vaddr,
            ctx.got_symbols,
            ctx.got_plt_vaddr,
            ctx.gd_tls_relax_info,
            ctx.tls_vaddr,
        )
    }
}

/// Encode a ULEB128 SET value into data.
fn apply_set_uleb128(data: &mut [u8], off: usize, val: u64) {
    let mut v = val;
    let mut i = off;
    loop {
        if i >= data.len() {
            break;
        }
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            data[i] = byte | 0x80;
        } else {
            data[i] = byte;
            break;
        }
        i += 1;
    }
}

/// Decode a ULEB128 value from data, subtract, and re-encode.
fn apply_sub_uleb128(data: &mut [u8], off: usize, sub_val: u64) {
    // Decode
    let mut cur: u64 = 0;
    let mut shift = 0;
    let mut i = off;
    loop {
        if i >= data.len() {
            break;
        }
        let byte = data[i];
        cur |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        i += 1;
    }
    // Re-encode
    let val = cur.wrapping_sub(sub_val);
    let mut v = val;
    let mut j = off;
    loop {
        if j >= data.len() {
            break;
        }
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v != 0 {
            data[j] = byte | 0x80;
        } else {
            data[j] = byte;
            break;
        }
        j += 1;
    }
}

/// Collect GD TLS relaxation info for static binaries.
///
/// Pre-scans relocations to find R_RISCV_TLS_GD_HI20 entries and their
/// associated __tls_get_addr CALL_PLT entries, so they can be relaxed
/// to Local-Exec (LUI + ADD) during relocation application.
pub fn collect_gd_tls_relax_info(
    input_objs: &[(String, super::elf_read::ElfObject)],
    sec_mapping: &HashMap<(usize, usize), (usize, u64)>,
    section_vaddrs: &[u64],
    local_sym_vaddrs: &[Vec<u64>],
    global_syms: &HashMap<String, GlobalSym>,
    gd_tls_relax_info: &mut HashMap<u64, (u64, i64)>,
    gd_tls_call_nop: &mut HashSet<u64>,
) {
    for (obj_idx, (_, obj)) in input_objs.iter().enumerate() {
        for (sec_idx, relocs) in obj.relocations.iter().enumerate() {
            if relocs.is_empty() {
                continue;
            }
            let (merged_idx, sec_offset) = match sec_mapping.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };
            let ms_vaddr = section_vaddrs[merged_idx];

            for (ri, reloc) in relocs.iter().enumerate() {
                if reloc.rela_type == R_RISCV_TLS_GD_HI20 {
                    let offset = sec_offset + reloc.offset;
                    let auipc_vaddr = ms_vaddr + offset;
                    let sym = &obj.symbols[reloc.sym_idx as usize];
                    let sym_val = resolve_symbol_value(
                        sym,
                        reloc.sym_idx as usize,
                        obj,
                        obj_idx,
                        sec_mapping,
                        section_vaddrs,
                        local_sym_vaddrs,
                        global_syms,
                    );
                    gd_tls_relax_info.insert(auipc_vaddr, (sym_val, reloc.addend));

                    for j in (ri + 1)..relocs.len().min(ri + 8) {
                        let call_reloc = &relocs[j];
                        if call_reloc.rela_type == R_RISCV_CALL_PLT {
                            let call_sym = &obj.symbols[call_reloc.sym_idx as usize];
                            if call_sym.name == "__tls_get_addr" {
                                let call_offset = sec_offset + call_reloc.offset;
                                let call_vaddr = ms_vaddr + call_offset;
                                gd_tls_call_nop.insert(call_vaddr);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}
