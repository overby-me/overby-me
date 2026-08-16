//! i386 relocation application for the i686 linker.
//!
//! Applies relocations from input objects to merged output sections.
//! All i386 relocation types are handled here, separated from the main
//! linking logic to keep the code manageable.
//!
//! The relocation types supported include absolute (R_386_32), PC-relative
//! (R_386_PC32, R_386_PLT32), GOT-related (R_386_GOT32, R_386_GOT32X,
//! R_386_GOTPC, R_386_GOTOFF), and TLS relocations.

use std::collections::HashMap;

use super::types::*;

/// Context for relocation application, containing all addresses needed
/// to resolve relocations.
pub(super) struct RelocContext<'a> {
    pub global_symbols: &'a HashMap<String, LinkerSymbol>,
    pub output_sections: &'a mut Vec<OutputSection>,
    pub section_map: &'a SectionMap,
    pub got_base: u32,
    pub got_vaddr: u32,
    pub gotplt_vaddr: u32,
    pub got_reserved: usize,
    pub gotplt_reserved: u32,
    #[allow(dead_code)] // Set by linker layout; available for future PLT-relative relocations
    pub plt_vaddr: u32,
    #[allow(dead_code)] // Set by linker layout; available for future PLT-relative relocations
    pub plt_header_size: u32,
    #[allow(dead_code)] // Set by linker layout; available for future PLT-relative relocations
    pub plt_entry_size: u32,
    pub num_plt: usize,
    pub tls_addr: u32,
    pub tls_mem_size: u32,
    pub has_tls: bool,
}

/// Apply all relocations from input objects to the output sections.
/// Returns a list of text relocations (address, dynsym_index) for symbols using textrel.
pub(super) fn apply_relocations(
    inputs: &[InputObject],
    ctx: &mut RelocContext,
) -> Result<Vec<(u32, String)>, String> {
    let mut text_relocs: Vec<(u32, String)> = Vec::new();
    for (obj_idx, obj) in inputs.iter().enumerate() {
        for sec in &obj.sections {
            if sec.relocations.is_empty() {
                continue;
            }

            let _out_name = match output_section_name(&sec.name, sec.flags, sec.sh_type) {
                Some(n) => n,
                None => continue,
            };
            let (out_sec_idx, sec_base_offset) =
                match ctx.section_map.get(&(obj_idx, sec.input_index)) {
                    Some(&v) => v,
                    None => continue,
                };

            for &(rel_offset, rel_type, sym_idx, addend) in &sec.relocations {
                let tr = apply_one_reloc(
                    obj_idx,
                    obj,
                    sec,
                    out_sec_idx,
                    sec_base_offset,
                    rel_offset,
                    rel_type,
                    sym_idx,
                    addend,
                    ctx,
                )?;
                if let Some(t) = tr {
                    text_relocs.push(t);
                }
            }
        }
    }
    Ok(text_relocs)
}

/// Apply a single relocation.
/// Returns Some((patch_addr, sym_name)) if a text relocation entry is needed.
fn apply_one_reloc(
    obj_idx: usize,
    obj: &InputObject,
    _sec: &InputSection,
    out_sec_idx: usize,
    sec_base_offset: u32,
    rel_offset: u32,
    rel_type: u32,
    sym_idx: u32,
    addend: i32,
    ctx: &mut RelocContext,
) -> Result<Option<(u32, String)>, String> {
    let patch_offset = sec_base_offset + rel_offset;
    let patch_addr = ctx.output_sections[out_sec_idx].addr + patch_offset;

    let sym = if (sym_idx as usize) < obj.symbols.len() {
        &obj.symbols[sym_idx as usize]
    } else {
        return Err(format!("invalid symbol index {} in reloc", sym_idx));
    };

    let sym_addr = resolve_sym_addr(obj_idx, sym, ctx);

    // Check if this symbol goes through PLT
    let is_dyn = !sym.name.is_empty()
        && ctx
            .global_symbols
            .get(&sym.name)
            .map(|gs| gs.is_dynamic && gs.needs_plt)
            .unwrap_or(false);

    let mut relax_got32x = false;
    let mut text_reloc: Option<(u32, String)> = None;

    let value: u32 = match rel_type {
        R_386_NONE => return Ok(None),
        R_386_32 => {
            // Check if this symbol uses text relocations (WEAK dynamic data)
            if !sym.name.is_empty() {
                if let Some(gs) = ctx.global_symbols.get(&sym.name) {
                    if gs.uses_textrel {
                        // Record a text relocation; write 0 for now (dynamic linker fills it)
                        text_reloc = Some((patch_addr, sym.name.clone()));
                        addend as u32
                    } else {
                        (sym_addr as i32 + addend) as u32
                    }
                } else {
                    (sym_addr as i32 + addend) as u32
                }
            } else {
                (sym_addr as i32 + addend) as u32
            }
        }
        R_386_PC32 | R_386_PLT32 => {
            let s = if is_dyn {
                ctx.global_symbols
                    .get(&sym.name)
                    .map(|gs| gs.address)
                    .unwrap_or(0)
            } else {
                sym_addr
            };
            (s as i32 + addend - patch_addr as i32) as u32
        }
        R_386_GOTPC => (ctx.got_base as i32 + addend - patch_addr as i32) as u32,
        R_386_GOTOFF => (sym_addr as i32 + addend - ctx.got_base as i32) as u32,
        R_386_GOT32 | R_386_GOT32X => {
            resolve_got_reloc(sym, sym_addr, addend, rel_type, ctx, &mut relax_got32x)
        }
        R_386_TLS_TPOFF | R_386_TLS_LE => {
            // Negative offset from TP
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
        R_386_TLS_LE_32 | R_386_TLS_TPOFF32 => {
            // rust-gcc emits `add` with TLS_TPOFF32, so compute negative offset
            // (same as TLS_TPOFF/TLS_LE) to match the `add` instruction.
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
        R_386_TLS_IE => resolve_tls_ie(sym, sym_addr, addend, ctx),
        R_386_TLS_GOTIE => resolve_tls_gotie(sym, sym_addr, addend, ctx),
        R_386_TLS_GD => {
            if ctx.has_tls && sym.sym_type == STT_TLS {
                let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
                (tpoff + addend) as u32
            } else {
                addend as u32
            }
        }
        R_386_TLS_DTPMOD32 => 1u32,
        R_386_TLS_DTPOFF32 => {
            if ctx.has_tls {
                (sym_addr as i32 - ctx.tls_addr as i32 + addend) as u32
            } else {
                addend as u32
            }
        }
        other => {
            return Err(format!(
                "unsupported i686 relocation type {} at {}:0x{:x}",
                other, obj.filename, rel_offset
            ));
        }
    };

    // Patch the output section data
    let out_sec = &mut ctx.output_sections[out_sec_idx];
    let off = patch_offset as usize;
    if off + 4 <= out_sec.data.len() {
        // For GOT32X relaxation, rewrite mov (0x8b) → lea (0x8d)
        if relax_got32x && off >= 2 && out_sec.data[off - 2] == 0x8b {
            out_sec.data[off - 2] = 0x8d;
        }
        out_sec.data[off..off + 4].copy_from_slice(&value.to_le_bytes());
    }

    Ok(text_reloc)
}

/// Resolve a symbol's address, handling local, section, and global symbols.
fn resolve_sym_addr(obj_idx: usize, sym: &InputSymbol, ctx: &RelocContext) -> u32 {
    if sym.sym_type == STT_SECTION {
        if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
            match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
                Some(&(sec_out_idx, sec_out_offset)) => {
                    ctx.output_sections[sec_out_idx].addr + sec_out_offset
                }
                None => 0,
            }
        } else {
            0
        }
    } else if sym.name.is_empty() {
        0
    } else if sym.binding == STB_LOCAL {
        // Local symbols resolve per-object via section_map to avoid
        // collisions between identically-named locals (e.g. .LC0).
        resolve_via_section_map(obj_idx, sym, ctx)
    } else {
        match ctx.global_symbols.get(&sym.name) {
            Some(gs) => gs.address,
            None => resolve_via_section_map(obj_idx, sym, ctx),
        }
    }
}

/// Resolve a symbol address through the section map + symbol value.
fn resolve_via_section_map(obj_idx: usize, sym: &InputSymbol, ctx: &RelocContext) -> u32 {
    if sym.section_index != SHN_UNDEF && sym.section_index != SHN_ABS {
        match ctx.section_map.get(&(obj_idx, sym.section_index as usize)) {
            Some(&(sec_out_idx, sec_out_offset)) => {
                ctx.output_sections[sec_out_idx].addr + sec_out_offset + sym.value
            }
            None => sym.value,
        }
    } else if sym.section_index == SHN_ABS {
        sym.value
    } else {
        0
    }
}

/// Resolve R_386_GOT32 or R_386_GOT32X relocations.
pub(super) fn resolve_got_reloc(
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    rel_type: u32,
    ctx: &RelocContext,
    relax_got32x: &mut bool,
) -> u32 {
    if let Some(gs) = ctx.global_symbols.get(&sym.name) {
        if gs.is_dynamic {
            let got_entry_addr = if gs.needs_plt {
                ctx.gotplt_vaddr + (ctx.gotplt_reserved + gs.plt_index as u32) * 4
            } else {
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4
            };
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else if gs.needs_got {
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else if rel_type == R_386_GOT32X {
            *relax_got32x = true;
            (sym_addr as i32 + addend - ctx.got_base as i32) as u32
        } else {
            (sym_addr as i32 + addend - ctx.got_base as i32) as u32
        }
    } else if rel_type == R_386_GOT32X {
        *relax_got32x = true;
        (sym_addr as i32 + addend - ctx.got_base as i32) as u32
    } else {
        (sym_addr as i32 + addend - ctx.got_base as i32) as u32
    }
}

/// Resolve R_386_TLS_IE relocation.
pub(super) fn resolve_tls_ie(
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    ctx: &RelocContext,
) -> u32 {
    if let Some(gs) = ctx.global_symbols.get(&sym.name) {
        if gs.needs_got {
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            (got_entry_addr as i32 + addend) as u32
        } else {
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
    } else {
        addend as u32
    }
}

/// Resolve R_386_TLS_GOTIE relocation.
pub(super) fn resolve_tls_gotie(
    sym: &InputSymbol,
    sym_addr: u32,
    addend: i32,
    ctx: &RelocContext,
) -> u32 {
    if let Some(gs) = ctx.global_symbols.get(&sym.name) {
        if gs.needs_got {
            let got_entry_addr =
                ctx.got_vaddr + (ctx.got_reserved as u32 + (gs.got_index - ctx.num_plt) as u32) * 4;
            (got_entry_addr as i32 + addend - ctx.got_base as i32) as u32
        } else {
            let tpoff = sym_addr as i32 - ctx.tls_addr as i32 - ctx.tls_mem_size as i32;
            (tpoff + addend) as u32
        }
    } else {
        addend as u32
    }
}
