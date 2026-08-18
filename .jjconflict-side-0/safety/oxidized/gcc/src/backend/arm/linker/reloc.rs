//! AArch64 relocation application.
//!
//! Applies ELF relocations to the output buffer after section layout
//! has been determined.

use super::elf::*;
use super::types::GlobalSymbol;
use crate::backend::linker_common::OutputSection;
use std::collections::HashMap;

// TLS relocation types (AArch64 Local Exec model for static linking)
const R_AARCH64_TLSLE_ADD_TPREL_HI12: u32 = 549;
const R_AARCH64_TLSLE_ADD_TPREL_LO12: u32 = 550;
const R_AARCH64_TLSLE_ADD_TPREL_LO12_NC: u32 = 551;
const R_AARCH64_TLSLE_MOVW_TPREL_G0: u32 = 544;
const R_AARCH64_TLSLE_MOVW_TPREL_G0_NC: u32 = 545;
const R_AARCH64_TLSLE_MOVW_TPREL_G1: u32 = 546;
const R_AARCH64_TLSLE_MOVW_TPREL_G1_NC: u32 = 547;
const R_AARCH64_TLSLE_MOVW_TPREL_G2: u32 = 548;

// TLS descriptor / Initial exec (convert to LE for static linking)
const R_AARCH64_TLSDESC_ADR_PAGE21: u32 = 562;
const R_AARCH64_TLSDESC_LD64_LO12: u32 = 563;
const R_AARCH64_TLSDESC_ADD_LO12: u32 = 564;
const R_AARCH64_TLSDESC_CALL: u32 = 569;

// GD (General Dynamic) -> LE relaxation for static linking
const R_AARCH64_TLSGD_ADR_PAGE21: u32 = 513;
const R_AARCH64_TLSGD_ADD_LO12_NC: u32 = 514;

// IE (Initial Exec) TLS
pub const R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21: u32 = 541;
pub const R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC: u32 = 542;

/// TLS layout info needed for relocation processing
pub struct TlsInfo {
    pub tls_addr: u64,
    pub tls_size: u64,
}

/// GOT (Global Offset Table) for static linking.
/// Maps symbol names to GOT entry indices for R_AARCH64_ADR_GOT_PAGE
/// and R_AARCH64_LD64_GOT_LO12_NC relocations.
pub struct GotInfo {
    pub got_addr: u64,
    /// (symbol_key, resolved_address) -- symbol_key is "name" or "sec:obj:sec"
    pub entries: HashMap<String, usize>,
}

impl GotInfo {
    /// Get the address of a GOT entry for a given symbol key.
    pub fn entry_addr(&self, key: &str) -> Option<u64> {
        self.entries
            .get(key)
            .map(|&idx| self.got_addr + (idx as u64) * 8)
    }
}

/// Resolve a symbol's final address given the global symbol table and section map.
///
/// Linker-defined symbols (__bss_start, _edata, _end, __end, etc.) are resolved
/// through the globals table where they are registered by get_standard_linker_symbols().
pub fn resolve_sym(
    obj_idx: usize,
    sym: &Symbol,
    globals: &HashMap<String, GlobalSymbol>,
    section_map: &HashMap<(usize, usize), (usize, u64)>,
    output_sections: &[OutputSection],
) -> u64 {
    if sym.sym_type() == STT_SECTION {
        let si = sym.shndx as usize;
        return section_map
            .get(&(obj_idx, si))
            .map(|&(oi, so)| output_sections[oi].addr + so)
            .unwrap_or(0);
    }
    if !sym.name.is_empty() && !sym.is_local() {
        // All linker-defined symbols (including __bss_start, _edata, _end, __end)
        // are registered in the globals table with defined_in = Some(usize::MAX),
        // so they are resolved through the standard globals lookup below.
        // Local (STB_LOCAL) symbols must NOT be resolved via globals, since a
        // local symbol named e.g. "write" must not be confused with libc's write().
        if let Some(g) = globals.get(&sym.name) {
            if g.defined_in.is_some() {
                return g.value;
            }
        }
        if sym.is_weak() {
            return 0;
        }
    }
    if sym.is_undefined() {
        return 0;
    }
    if sym.shndx == SHN_ABS {
        return sym.value;
    }
    section_map
        .get(&(obj_idx, sym.shndx as usize))
        .map(|&(oi, so)| output_sections[oi].addr + so + sym.value)
        .unwrap_or(sym.value)
}

/// Build a GOT key for a symbol reference in a relocation.
/// Local symbols must be scoped to their object to avoid collisions
/// (e.g., `.LANCHOR3` in different objects referring to different TLS vars).
pub fn got_key(obj_idx: usize, sym: &Symbol) -> String {
    if !sym.name.is_empty() && !sym.is_local() {
        sym.name.clone()
    } else if !sym.name.is_empty() {
        format!("{}@{}", sym.name, obj_idx)
    } else if sym.sym_type() == STT_SECTION {
        format!("__sec_{}_{}", obj_idx, sym.shndx)
    } else {
        format!("__anon_{}_{}", obj_idx, sym.shndx)
    }
}

/// Scan all relocations to find symbols that need GOT entries.
/// This includes both regular GOT references and TLS IE references
/// (which use GOT entries containing the TP offset).
pub fn collect_got_symbols(objects: &[ElfObject]) -> Vec<(String, GotEntryKind)> {
    let mut got_syms: Vec<(String, GotEntryKind)> = Vec::new();
    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            for rela in &objects[obj_idx].relocations[sec_idx] {
                let kind = match rela.rela_type {
                    R_AARCH64_ADR_GOT_PAGE | R_AARCH64_LD64_GOT_LO12_NC => GotEntryKind::Regular,
                    R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 | R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC => {
                        GotEntryKind::TlsIE
                    }
                    _ => continue,
                };
                let si = rela.sym_idx as usize;
                if si < objects[obj_idx].symbols.len() {
                    let key = got_key(obj_idx, &objects[obj_idx].symbols[si]);
                    if !got_syms.iter().any(|(k, _)| k == &key) {
                        got_syms.push((key, kind));
                    }
                }
            }
        }
    }
    got_syms
}

/// What kind of value a GOT entry holds.
#[derive(Clone, Copy, PartialEq)]
pub enum GotEntryKind {
    /// Regular GOT entry: holds the absolute address of the symbol.
    Regular,
    /// TLS IE GOT entry: holds the TP-relative offset of the TLS variable.
    TlsIE,
}

/// Apply all relocations to the output buffer.
pub fn apply_relocations(
    objects: &[ElfObject],
    globals: &HashMap<String, GlobalSymbol>,
    output_sections: &[OutputSection],
    section_map: &HashMap<(usize, usize), (usize, u64)>,
    out: &mut [u8],
    tls_info: &TlsInfo,
    got_info: &GotInfo,
) -> Result<(), String> {
    for obj_idx in 0..objects.len() {
        for sec_idx in 0..objects[obj_idx].sections.len() {
            let relas = &objects[obj_idx].relocations[sec_idx];
            if relas.is_empty() {
                continue;
            }
            let (out_idx, sec_off) = match section_map.get(&(obj_idx, sec_idx)) {
                Some(&v) => v,
                None => continue,
            };
            let sa = output_sections[out_idx].addr;
            let sfo = output_sections[out_idx].file_offset;

            for rela in relas {
                let si = rela.sym_idx as usize;
                if si >= objects[obj_idx].symbols.len() {
                    continue;
                }
                let sym = &objects[obj_idx].symbols[si];
                let p = sa + sec_off + rela.offset;
                let fp = (sfo + sec_off + rela.offset) as usize;
                let a = rela.addend;
                let s = resolve_sym(obj_idx, sym, globals, section_map, output_sections);
                let gkey = got_key(obj_idx, sym);

                apply_one_reloc(
                    out,
                    fp,
                    rela.rela_type,
                    s,
                    a,
                    p,
                    &sym.name,
                    &objects[obj_idx].source_name,
                    tls_info,
                    got_info,
                    &gkey,
                )?;
            }
        }
    }
    Ok(())
}

/// Compute TP offset for AArch64. On AArch64, the TLS block starts at TP + 16
/// (TP points to the DTV, TLS block is right after).
/// For Local Exec: tp_offset = sym_addr - tls_start_addr + 16
fn tprel(s: u64, a: i64, tls_info: &TlsInfo) -> i64 {
    // AArch64 uses variant 1 TLS: TP points to start of TCB (16 bytes),
    // followed by TLS block. So offset from TP = (S+A - tls_base) + 16
    if tls_info.tls_addr == 0 {
        return (s as i64).wrapping_add(a);
    }
    let sa = (s as i64).wrapping_add(a) as u64;
    (sa as i64) - (tls_info.tls_addr as i64) + 16
}

/// Apply a single AArch64 relocation.
pub fn apply_one_reloc(
    out: &mut [u8],
    fp: usize,
    rtype: u32,
    s: u64,
    a: i64,
    p: u64,
    sym_name: &str,
    source: &str,
    tls_info: &TlsInfo,
    got_info: &GotInfo,
    got_key: &str,
) -> Result<(), String> {
    match rtype {
        R_AARCH64_NONE => {}

        // ── Absolute relocations ──
        R_AARCH64_ABS64 => {
            let val = (s as i64).wrapping_add(a) as u64;
            w64(out, fp, val);
        }
        R_AARCH64_ABS32 => {
            let val = (s as i64).wrapping_add(a) as u64;
            w32(out, fp, val as u32);
        }
        R_AARCH64_ABS16 => {
            let val = (s as i64).wrapping_add(a) as u64;
            w16(out, fp, val as u16);
        }

        // ── PC-relative relocations ──
        R_AARCH64_PREL64 => {
            let val = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            w64(out, fp, val as u64);
        }
        R_AARCH64_PREL32 => {
            let val = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            w32(out, fp, val as u32);
        }
        R_AARCH64_PREL16 => {
            let val = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            w16(out, fp, val as u16);
        }

        // ── ADRP: Page-relative high 21 bits ──
        R_AARCH64_ADR_PREL_PG_HI21 => {
            let sa = (s as i64).wrapping_add(a) as u64;
            let page_s = sa & !0xFFF;
            let page_p = p & !0xFFF;
            let offset = page_s as i64 - page_p as i64;
            let imm = offset >> 12;
            encode_adrp(out, fp, imm);
        }

        // ── ADR: PC-relative low 21 bits ──
        R_AARCH64_ADR_PREL_LO21 => {
            let offset = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            encode_adr(out, fp, offset);
        }

        // ── ADD: absolute low 12 bits (no carry check) ──
        R_AARCH64_ADD_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_add_imm12(out, fp, (sa & 0xFFF) as u32);
        }

        // ── Load/store low 12 bits for different sizes ──
        R_AARCH64_LDST8_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 0);
        }
        R_AARCH64_LDST16_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 1);
        }
        R_AARCH64_LDST32_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 2);
        }
        R_AARCH64_LDST64_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 3);
        }
        R_AARCH64_LDST128_ABS_LO12_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 4);
        }

        // ── Branch instructions ──
        R_AARCH64_CALL26 | R_AARCH64_JUMP26 => {
            if fp + 4 > out.len() {
                return Ok(());
            }
            let sa = (s as i64).wrapping_add(a) as u64;
            if sa == 0 {
                // Undefined/weak symbol resolved to 0: replace with NOP
                w32(out, fp, 0xd503201f);
            } else {
                let offset = (sa as i64).wrapping_sub(p as i64);
                let mut insn = read_u32(out, fp);
                let imm26 = ((offset >> 2) as u32) & 0x3ffffff;
                insn = (insn & 0xfc000000) | imm26;
                w32(out, fp, insn);
            }
        }

        // ── Conditional branch (19-bit offset) ──
        R_AARCH64_CONDBR19 => {
            let offset = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            if fp + 4 > out.len() {
                return Ok(());
            }
            let mut insn = read_u32(out, fp);
            let imm19 = ((offset >> 2) as u32) & 0x7ffff;
            insn = (insn & 0xff00001f) | (imm19 << 5);
            w32(out, fp, insn);
        }

        // ── Test and branch (14-bit offset) ──
        R_AARCH64_TSTBR14 => {
            let offset = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            if fp + 4 > out.len() {
                return Ok(());
            }
            let mut insn = read_u32(out, fp);
            let imm14 = ((offset >> 2) as u32) & 0x3fff;
            insn = (insn & 0xfff8001f) | (imm14 << 5);
            w32(out, fp, insn);
        }

        // ── MOVW relocations ──
        R_AARCH64_MOVW_UABS_G0 | R_AARCH64_MOVW_UABS_G0_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_movw(out, fp, (sa & 0xffff) as u32);
        }
        R_AARCH64_MOVW_UABS_G1_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_movw(out, fp, ((sa >> 16) & 0xffff) as u32);
        }
        R_AARCH64_MOVW_UABS_G2_NC => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_movw(out, fp, ((sa >> 32) & 0xffff) as u32);
        }
        R_AARCH64_MOVW_UABS_G3 => {
            let sa = (s as i64).wrapping_add(a) as u64;
            encode_movw(out, fp, ((sa >> 48) & 0xffff) as u32);
        }

        // ── GOT relocations ──
        // Even in static linking, we use a real GOT since the instruction
        // is an LDR (load from memory), not an ADD.
        R_AARCH64_ADR_GOT_PAGE => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                let page_g = got_entry_addr & !0xFFF;
                let page_p = p & !0xFFF;
                let imm = (page_g as i64 - page_p as i64) >> 12;
                encode_adrp(out, fp, imm);
            } else {
                // Fallback: treat like ADR_PREL_PG_HI21
                let sa = (s as i64).wrapping_add(a) as u64;
                let page_s = sa & !0xFFF;
                let page_p = p & !0xFFF;
                let imm = (page_s as i64 - page_p as i64) >> 12;
                encode_adrp(out, fp, imm);
            }
        }
        R_AARCH64_LD64_GOT_LO12_NC => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                encode_ldst_imm12(out, fp, (got_entry_addr & 0xFFF) as u32, 3);
            } else {
                let sa = (s as i64).wrapping_add(a) as u64;
                encode_ldst_imm12(out, fp, (sa & 0xFFF) as u32, 3);
            }
        }

        // ── TLS Local Exec (LE) relocations ──
        // These are used when the TLS variable is in the executable itself.
        // On AArch64 variant 1, tp offset = sym_offset_in_tls + 16 (TCB size)
        R_AARCH64_TLSLE_ADD_TPREL_HI12 => {
            let tp = tprel(s, a, tls_info);
            if std::env::var("LINKER_DEBUG_TLS").is_ok() {
                eprintln!("  TLSLE_HI12: sym='{}' s=0x{:x} a={} tls_addr=0x{:x} tls_size=0x{:x} -> tp=0x{:x}",
                    sym_name, s, a, tls_info.tls_addr, tls_info.tls_size, tp as u64);
            }
            let imm12 = ((tp as u64 >> 12) & 0xFFF) as u32;
            encode_add_imm12(out, fp, imm12);
        }
        R_AARCH64_TLSLE_ADD_TPREL_LO12 | R_AARCH64_TLSLE_ADD_TPREL_LO12_NC => {
            let tp = tprel(s, a, tls_info);
            if std::env::var("LINKER_DEBUG_TLS").is_ok() {
                eprintln!(
                    "  TLSLE_LO12: sym='{}' s=0x{:x} a={} tls_addr=0x{:x} -> tp=0x{:x}",
                    sym_name, s, a, tls_info.tls_addr, tp as u64
                );
            }
            let imm12 = (tp as u64 & 0xFFF) as u32;
            encode_add_imm12(out, fp, imm12);
        }
        R_AARCH64_TLSLE_MOVW_TPREL_G0 | R_AARCH64_TLSLE_MOVW_TPREL_G0_NC => {
            let tp = tprel(s, a, tls_info);
            encode_movw(out, fp, (tp as u64 & 0xffff) as u32);
        }
        R_AARCH64_TLSLE_MOVW_TPREL_G1 | R_AARCH64_TLSLE_MOVW_TPREL_G1_NC => {
            let tp = tprel(s, a, tls_info);
            encode_movw(out, fp, ((tp as u64 >> 16) & 0xffff) as u32);
        }
        R_AARCH64_TLSLE_MOVW_TPREL_G2 => {
            let tp = tprel(s, a, tls_info);
            encode_movw(out, fp, ((tp as u64 >> 32) & 0xffff) as u32);
        }

        // ── TLS IE (Initial Exec) via GOT for static linking ──
        // Instead of relaxing to MOVZ/MOVK (which can break if registers
        // differ between ADRP and LDR), we use real GOT entries that hold
        // the pre-computed TP offset. The instructions remain ADRP+LDR
        // pointing at our GOT entry.
        R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                let page_g = got_entry_addr & !0xFFF;
                let page_p = p & !0xFFF;
                let imm = (page_g as i64 - page_p as i64) >> 12;
                encode_adrp(out, fp, imm);
            } else {
                // Fallback: relax to MOVZ
                let tp = tprel(s, a, tls_info);
                if fp + 4 > out.len() {
                    return Ok(());
                }
                let insn = read_u32(out, fp);
                let rd = insn & 0x1f;
                let imm16 = ((tp as u64 >> 16) & 0xffff) as u32;
                let new_insn = 0xd2a00000 | (imm16 << 5) | rd;
                w32(out, fp, new_insn);
            }
        }
        R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC => {
            if let Some(got_entry_addr) = got_info.entry_addr(got_key) {
                encode_ldst_imm12(out, fp, (got_entry_addr & 0xFFF) as u32, 3);
            } else {
                // Fallback: relax to MOVK
                let tp = tprel(s, a, tls_info);
                if fp + 4 > out.len() {
                    return Ok(());
                }
                let insn = read_u32(out, fp);
                let rd = insn & 0x1f;
                let imm16 = (tp as u64 & 0xffff) as u32;
                let new_insn = 0xf2800000 | (imm16 << 5) | rd;
                w32(out, fp, new_insn);
            }
        }

        // ── TLSDESC relaxation to LE for static linking ──
        R_AARCH64_TLSDESC_ADR_PAGE21 => {
            // Replace ADRP with MOVZ Xd, #tprel_g1, LSL #16
            let tp = tprel(s, a, tls_info);
            if fp + 4 > out.len() {
                return Ok(());
            }
            let insn = read_u32(out, fp);
            let rd = insn & 0x1f;
            let imm16 = ((tp as u64 >> 16) & 0xffff) as u32;
            let new_insn = 0xd2a00000 | (imm16 << 5) | rd;
            w32(out, fp, new_insn);
        }
        R_AARCH64_TLSDESC_LD64_LO12 => {
            // Replace LDR with MOVK Xd, #tprel_lo
            let tp = tprel(s, a, tls_info);
            if fp + 4 > out.len() {
                return Ok(());
            }
            let insn = read_u32(out, fp);
            let rd = insn & 0x1f;
            let imm16 = (tp as u64 & 0xffff) as u32;
            let new_insn = 0xf2800000 | (imm16 << 5) | rd;
            w32(out, fp, new_insn);
        }
        R_AARCH64_TLSDESC_ADD_LO12 => {
            // NOP (the value is already in the register from MOVZ+MOVK)
            if fp + 4 > out.len() {
                return Ok(());
            }
            w32(out, fp, 0xd503201f); // NOP
        }
        R_AARCH64_TLSDESC_CALL => {
            // NOP (no runtime call needed for static linking)
            if fp + 4 > out.len() {
                return Ok(());
            }
            w32(out, fp, 0xd503201f); // NOP
        }

        // ── TLS GD -> LE relaxation ──
        // TODO: full GD relaxation also needs to NOP the BL __tls_get_addr that follows
        R_AARCH64_TLSGD_ADR_PAGE21 => {
            let tp = tprel(s, a, tls_info);
            if fp + 4 > out.len() {
                return Ok(());
            }
            let insn = read_u32(out, fp);
            let rd = insn & 0x1f;
            let imm16 = ((tp as u64 >> 16) & 0xffff) as u32;
            let new_insn = 0xd2a00000 | (imm16 << 5) | rd;
            w32(out, fp, new_insn);
        }
        R_AARCH64_TLSGD_ADD_LO12_NC => {
            let tp = tprel(s, a, tls_info);
            if fp + 4 > out.len() {
                return Ok(());
            }
            let insn = read_u32(out, fp);
            let rd = insn & 0x1f;
            let imm16 = (tp as u64 & 0xffff) as u32;
            let new_insn = 0xf2800000 | (imm16 << 5) | rd;
            w32(out, fp, new_insn);
        }

        other => {
            return Err(format!(
                "unsupported AArch64 relocation type {} for '{}' in {}",
                other, sym_name, source
            ));
        }
    }
    Ok(())
}

// ── Instruction encoding helpers ───────────────────────────────────────

pub(super) fn encode_adrp(out: &mut [u8], fp: usize, imm: i64) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    let immlo = (imm as u32) & 0x3;
    let immhi = ((imm as u32) >> 2) & 0x7ffff;
    insn = (insn & 0x9f00001f) | (immlo << 29) | (immhi << 5);
    w32(out, fp, insn);
}

pub(super) fn encode_adr(out: &mut [u8], fp: usize, offset: i64) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    let imm = offset as u32;
    let immlo = imm & 0x3;
    let immhi = (imm >> 2) & 0x7ffff;
    insn = (insn & 0x9f00001f) | (immlo << 29) | (immhi << 5);
    w32(out, fp, insn);
}

pub(super) fn encode_add_imm12(out: &mut [u8], fp: usize, imm12: u32) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    insn = (insn & 0xffc003ff) | ((imm12 & 0xfff) << 10);
    w32(out, fp, insn);
}

pub(super) fn encode_ldst_imm12(out: &mut [u8], fp: usize, lo12: u32, shift: u32) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    let imm12 = (lo12 >> shift) & 0xfff;
    insn = (insn & 0xffc003ff) | (imm12 << 10);
    w32(out, fp, insn);
}

pub(super) fn encode_movw(out: &mut [u8], fp: usize, imm16: u32) {
    if fp + 4 > out.len() {
        return;
    }
    let mut insn = read_u32(out, fp);
    insn = (insn & 0xffe0001f) | ((imm16 & 0xffff) << 5);
    w32(out, fp, insn);
}
