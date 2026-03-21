//! System and privileged instruction encoders for i686.
//!
//! Handles prefetch, port I/O, INVLPG, VERW, LSL, descriptor table
//! operations, control register moves, segment register moves,
//! and other system-level instructions.

use super::*;

impl super::InstructionEncoder {
    /// Encode prefetch instructions (0F 18 /hint)
    pub(super) fn encode_prefetch(&mut self, ops: &[Operand], hint: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("prefetch requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x18]);
                self.encode_modrm_mem(hint, mem)
            }
            _ => Err("prefetch requires memory operand".to_string()),
        }
    }

    /// Encode prefetchw (0F 0D /1)
    pub(super) fn encode_prefetch_0f0d(&mut self, ops: &[Operand], hint: u8) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("prefetchw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x0D]);
                self.encode_modrm_mem(hint, mem)
            }
            _ => Err("prefetchw requires memory operand".to_string()),
        }
    }

    /// Encode OUT instruction: outb/outw/outl
    pub(super) fn encode_out(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        let size: u8 = match mnemonic {
            "outb" => 1,
            "outw" => 2,
            "outl" => 4,
            _ => return Err(format!("unknown out mnemonic: {}", mnemonic)),
        };

        // Handle zero-operand form (implicit operands)
        if ops.is_empty() {
            if size == 2 {
                self.bytes.push(0x66);
            }
            self.bytes.push(if size == 1 { 0xEE } else { 0xEF });
            return Ok(());
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 0 or 2 operands", mnemonic));
        }

        match (&ops[0], &ops[1]) {
            (Operand::Register(_src), Operand::Register(_dst)) => {
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xEE } else { 0xEF });
                Ok(())
            }
            (Operand::Register(_src), Operand::Immediate(ImmediateValue::Integer(val))) => {
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xE6 } else { 0xE7 });
                self.bytes.push(*val as u8);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Encode IN instruction: inb/inw/inl
    pub(super) fn encode_in(&mut self, ops: &[Operand], mnemonic: &str) -> Result<(), String> {
        let size: u8 = match mnemonic {
            "inb" => 1,
            "inw" => 2,
            "inl" => 4,
            _ => return Err(format!("unknown in mnemonic: {}", mnemonic)),
        };

        if ops.is_empty() {
            if size == 2 {
                self.bytes.push(0x66);
            }
            self.bytes.push(if size == 1 { 0xEC } else { 0xED });
            return Ok(());
        }

        if ops.len() != 2 {
            return Err(format!("{} requires 0 or 2 operands", mnemonic));
        }

        match (&ops[0], &ops[1]) {
            (Operand::Register(_src), Operand::Register(_dst)) => {
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xEC } else { 0xED });
                Ok(())
            }
            (Operand::Immediate(ImmediateValue::Integer(val)), Operand::Register(_dst)) => {
                if size == 2 {
                    self.bytes.push(0x66);
                }
                self.bytes.push(if size == 1 { 0xE4 } else { 0xE5 });
                self.bytes.push(*val as u8);
                Ok(())
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Encode INVLPG: 0F 01 /7 (memory operand)
    pub(super) fn encode_invlpg(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("invlpg requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(7, mem)
            }
            _ => Err("invlpg requires memory operand".to_string()),
        }
    }

    /// Encode VERW: 0F 00 /5
    pub(super) fn encode_verw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("verw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x00]);
                self.encode_modrm_mem(5, mem)
            }
            Operand::Register(reg) => {
                let rm = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x00]);
                self.bytes.push(self.modrm(3, 5, rm));
                Ok(())
            }
            _ => Err("verw requires memory or register operand".to_string()),
        }
    }

    /// Encode LSL (Load Segment Limit): 0F 03 /r
    pub(super) fn encode_lsl(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("lsl requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let is_16 = matches!(
                    src.name.as_str(),
                    "ax" | "bx" | "cx" | "dx" | "si" | "di" | "sp" | "bp"
                );
                if is_16 {
                    self.bytes.push(0x66);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x03]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x03]);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported lsl operands".to_string()),
        }
    }

    /// Encode SGDT/SIDT/LGDT/LIDT: 0F 01 /N (memory operand)
    pub(super) fn encode_system_table(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 1 {
            return Err(format!("{} requires 1 operand", mnemonic));
        }
        // Strip optional 'l' suffix (e.g., "lgdtl" -> "lgdt")
        let base = mnemonic.strip_suffix('l').unwrap_or(mnemonic);
        let reg_ext = match base {
            "sgdt" => 0,
            "sidt" => 1,
            "lgdt" => 2,
            "lidt" => 3,
            _ => return Err(format!("unknown system table instruction: {}", mnemonic)),
        };
        match &ops[0] {
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(reg_ext, mem)
            }
            // Label as absolute memory reference: lgdtl tr_gdt
            Operand::Label(label) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                // mod=00, rm=101 for disp32 (no base register)
                self.bytes.push(self.modrm(0, reg_ext, 5));
                self.add_relocation_for_label(label, R_386_32);
                self.bytes.extend_from_slice(&[0, 0, 0, 0]);
                Ok(())
            }
            _ => Err(format!("{} requires memory operand", mnemonic)),
        }
    }

    /// Encode LMSW (Load Machine Status Word): 0F 01 /6
    /// Accepts a 16-bit register or memory operand.
    pub(super) fn encode_lmsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("lmsw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let rm = reg_num(&reg.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.bytes.push(self.modrm(3, 6, rm));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(6, mem)
            }
            _ => Err("lmsw requires register or memory operand".to_string()),
        }
    }

    /// Encode SMSW (Store Machine Status Word): 0F 01 /4
    /// Accepts a 16-bit register or memory operand.
    /// Register form gets a 66h prefix for 16-bit operand size.
    pub(super) fn encode_smsw(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("smsw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                let rm = reg_num(&reg.name).ok_or("bad register")?;
                // 16-bit register form needs operand size prefix
                let is_16 = matches!(
                    reg.name.as_str(),
                    "ax" | "bx" | "cx" | "dx" | "si" | "di" | "sp" | "bp"
                );
                if is_16 {
                    self.bytes.push(0x66);
                }
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.bytes.push(self.modrm(3, 4, rm));
                Ok(())
            }
            Operand::Memory(mem) => {
                self.bytes.extend_from_slice(&[0x0F, 0x01]);
                self.encode_modrm_mem(4, mem)
            }
            _ => Err("smsw requires register or memory operand".to_string()),
        }
    }

    /// Encode MOV to/from control register: 0F 20 /r (read) or 0F 22 /r (write)
    pub(super) fn encode_mov_cr(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov cr requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            (Operand::Register(cr), Operand::Register(gp)) if is_control_reg(&cr.name) => {
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x20]);
                self.bytes.push(self.modrm(3, cr_num, gp_num));
                Ok(())
            }
            (Operand::Register(gp), Operand::Register(cr)) if is_control_reg(&cr.name) => {
                let cr_num = control_reg_num(&cr.name).ok_or("bad control register")?;
                let gp_num = reg_num(&gp.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&[0x0F, 0x22]);
                self.bytes.push(self.modrm(3, cr_num, gp_num));
                Ok(())
            }
            _ => Err("unsupported mov cr operands".to_string()),
        }
    }

    /// Encode MOV to/from segment register
    pub(super) fn encode_mov_seg(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("mov seg requires 2 operands".to_string());
        }

        let seg_num = |name: &str| -> Option<u8> {
            match name {
                "es" => Some(0),
                "cs" => Some(1),
                "ss" => Some(2),
                "ds" => Some(3),
                "fs" => Some(4),
                "gs" => Some(5),
                _ => None,
            }
        };

        match (&ops[0], &ops[1]) {
            // mov %sreg, %reg32
            (Operand::Register(src), Operand::Register(dst)) if is_segment_reg(&src.name) => {
                let sr = seg_num(&src.name).ok_or("bad segment register")?;
                let gp = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.push(0x8C);
                self.bytes.push(self.modrm(3, sr, gp));
                Ok(())
            }
            // mov %reg32, %sreg
            (Operand::Register(src), Operand::Register(dst)) if is_segment_reg(&dst.name) => {
                let gp = reg_num(&src.name).ok_or("bad register")?;
                let sr = seg_num(&dst.name).ok_or("bad segment register")?;
                self.bytes.push(0x8E);
                self.bytes.push(self.modrm(3, sr, gp));
                Ok(())
            }
            // mov %sreg, mem
            (Operand::Register(src), Operand::Memory(mem)) if is_segment_reg(&src.name) => {
                let sr = seg_num(&src.name).ok_or("bad segment register")?;
                self.bytes.push(0x8C);
                self.encode_modrm_mem(sr, mem)
            }
            // mov mem, %sreg
            (Operand::Memory(mem), Operand::Register(dst)) if is_segment_reg(&dst.name) => {
                let sr = seg_num(&dst.name).ok_or("bad segment register")?;
                self.bytes.push(0x8E);
                self.encode_modrm_mem(sr, mem)
            }
            _ => Err("unsupported mov seg operands".to_string()),
        }
    }

    /// Encode popw (16-bit pop)
    pub(super) fn encode_pop16(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 1 {
            return Err("popw requires 1 operand".to_string());
        }
        match &ops[0] {
            Operand::Register(reg) => {
                if is_segment_reg(&reg.name) {
                    // Segment register pops don't use 0x66 prefix
                    match reg.name.as_str() {
                        "es" => {
                            self.bytes.push(0x07);
                            Ok(())
                        }
                        "ss" => {
                            self.bytes.push(0x17);
                            Ok(())
                        }
                        "ds" => {
                            self.bytes.push(0x1F);
                            Ok(())
                        }
                        "fs" => {
                            self.bytes.extend_from_slice(&[0x0F, 0xA1]);
                            Ok(())
                        }
                        "gs" => {
                            self.bytes.extend_from_slice(&[0x0F, 0xA9]);
                            Ok(())
                        }
                        _ => Err(format!("cannot pop to {}", reg.name)),
                    }
                } else {
                    let num = reg_num(&reg.name).ok_or("bad register")?;
                    self.bytes.push(0x66);
                    self.bytes.push(0x58 + num);
                    Ok(())
                }
            }
            _ => Err("unsupported popw operand".to_string()),
        }
    }

    /// Encode 16-bit BSF/BSR: bsfw/bsrw
    pub(super) fn encode_bsr_bsf_16(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        let opcode = match mnemonic {
            "bsrw" => [0x0F, 0xBD],
            "bsfw" => [0x0F, 0xBC],
            _ => return Err(format!("unknown bit scan: {}", mnemonic)),
        };
        self.bytes.push(0x66); // 16-bit operand size prefix
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                self.bytes.extend_from_slice(&opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }
}
