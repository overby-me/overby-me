use super::*;

impl super::InstructionEncoder {
    // ---- VEX encoding helpers for AVX ----

    /// Emit a 2-byte or 3-byte VEX prefix.
    /// pp: 0=none, 1=66, 2=F3, 3=F2
    /// mm: 1=0F, 2=0F38, 3=0F3A
    /// w: 0 or 1 (VEX.W)
    /// vvvv: complement of source register number (15 - reg_num, or 15 if none)
    /// l: 0=128, 1=256
    /// r, x, b: VEX extension bits (inverted from REX)
    pub(crate) fn emit_vex(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        mm: u8,
        w: u8,
        vvvv: u8,
        l: u8,
        pp: u8,
    ) {
        let r_bit = if r { 0 } else { 1 };
        let x_bit = if x { 0 } else { 1 };
        let b_bit = if b { 0 } else { 1 };
        let vvvv_inv = (!vvvv) & 0xF;

        // Use 2-byte VEX if possible: mm=1, w=0, x=0, b=0
        if mm == 1 && w == 0 && !x && !b {
            self.bytes.push(0xC5);
            let byte2 = (r_bit << 7) | (vvvv_inv << 3) | (l << 2) | pp;
            self.bytes.push(byte2);
        } else {
            // 3-byte VEX
            self.bytes.push(0xC4);
            let byte1 = (r_bit << 7) | (x_bit << 6) | (b_bit << 5) | mm;
            let byte2 = (w << 7) | (vvvv_inv << 3) | (l << 2) | pp;
            self.bytes.push(byte1);
            self.bytes.push(byte2);
        }
    }

    /// Emit EVEX 4-byte prefix.
    /// Parameters match VEX but with additional EVEX-specific fields.
    /// ll: 00=128, 01=256, 10=512
    /// TODO: z (merge-masking) and aaa (opmask register k1-k7) are not yet used.
    /// TODO: r_prime and v_prime are passed as false; zmm16-zmm31 won't encode correctly.
    pub(crate) fn emit_evex(
        &mut self,
        r: bool,
        x: bool,
        b: bool,
        r_prime: bool,
        mm: u8,
        w: u8,
        vvvv: u8,
        v_prime: bool,
        pp: u8,
        ll: u8,
        _z: bool,
        _aaa: u8,
    ) {
        let r_bit = if r { 0u8 } else { 1 };
        let x_bit = if x { 0u8 } else { 1 };
        let b_bit = if b { 0u8 } else { 1 };
        let r_prime_bit = if r_prime { 0u8 } else { 1 };
        let vvvv_inv = (!vvvv) & 0xF;
        let v_prime_bit = if v_prime { 0u8 } else { 1 };

        self.bytes.push(0x62); // EVEX prefix indicator

        // Byte 1: R X B R' 0 0 mm
        let byte1 = (r_bit << 7) | (x_bit << 6) | (b_bit << 5) | (r_prime_bit << 4) | mm;
        self.bytes.push(byte1);

        // Byte 2: W vvvv 1 pp
        let byte2 = (w << 7) | (vvvv_inv << 3) | (1 << 2) | pp;
        self.bytes.push(byte2);

        // Byte 3: z L'L b V' aaa
        let byte3 = (ll << 5) | (v_prime_bit << 3);
        self.bytes.push(byte3);
    }

    /// Determine EVEX L'L from operands: 00=128(xmm), 01=256(ymm), 10=512(zmm)
    pub(crate) fn evex_ll_from_ops(&self, ops: &[Operand]) -> u8 {
        for op in ops {
            if let Operand::Register(r) = op {
                let name = r.name.to_lowercase();
                if name.starts_with("zmm") {
                    return 0b10;
                }
                if name.starts_with("ymm") {
                    return 0b01;
                }
            }
        }
        0b00 // default to 128-bit
    }

    /// Encode EVEX 3-operand instruction (e.g., vpxord, vpandd, etc.)
    /// Operands in AT&T order: src, vvvv, dst
    pub(crate) fn encode_evex_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX 3-op requires 3 operands".to_string());
        }
        let ll = self.evex_ll_from_ops(ops);

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                // mm=1 (0F map)
                self.emit_evex(r, false, b, false, 1, w, vvvv_enc, false, pp, ll, false, 0);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_evex(r, x, b_ext, false, 1, w, vvvv_enc, false, pp, ll, false, 0);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported EVEX 3-op operands".to_string()),
        }
    }

    /// Encode EVEX rotate-by-immediate instructions (vprold, vprolq, vprord, vprorq).
    /// AT&T syntax: `vprold $imm8, %src, %dst`
    ///   ops[0] = imm8 (rotation count)
    ///   ops[1] = src  (in ModRM r/m field)
    ///   ops[2] = dst  (in EVEX.vvvv field)
    /// Extension digit `ext` goes in ModRM reg field (/0 for ror, /1 for rol).
    pub(crate) fn encode_evex_rotate_imm(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        ext: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("EVEX rotate requires 3 operands (imm, src, dst)".to_string());
        }
        let ll = self.evex_ll_from_ops(ops);

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let b = needs_vex_ext(&src.name);
                let dst_ext = needs_vex_ext(&dst.name);
                let vvvv_enc = dst_num | (if dst_ext { 8 } else { 0 });
                // pp=1 (66), mm=1 (0F map), no R extension needed for reg field (it's a fixed /ext)
                self.emit_evex(
                    false, false, b, false, 1, w, vvvv_enc, false, 1, ll, false, 0,
                );
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, ext, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported EVEX rotate operands".to_string()),
        }
    }

    /// Determine VEX L (vector length) from operand: 0=128(xmm), 1=256(ymm)
    pub(crate) fn vex_l_from_ops(&self, ops: &[Operand]) -> u8 {
        for op in ops {
            match op {
                Operand::Register(r) if is_ymm(&r.name) => return 1,
                _ => {}
            }
        }
        0
    }

    /// Encode AVX vmovdqa/vmovdqu (load/store with 66/F3 prefix)
    pub(crate) fn encode_avx_mov(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        is_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX mov requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if is_66 { 1 } else { 2 }; // 66 -> pp=1, F3 -> pp=2

        match (&ops[0], &ops[1]) {
            // load: mem/reg -> xmm/ymm
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.encode_modrm_mem(dst_num, mem)
            }
            // store: xmm/ymm -> mem
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(store_op);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported AVX mov operands".to_string()),
        }
    }

    /// Encode AVX vmovaps/vmovapd/vmovups/vmovupd (no mandatory prefix, or 66 prefix)
    pub(crate) fn encode_avx_mov_np(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        is_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX mov requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if is_66 { 1 } else { 0 };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(load_op);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(store_op);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported AVX mov operands".to_string()),
        }
    }

    /// Encode AVX 3-operand instruction with 66 prefix (or no prefix): op src, vvvv, dst
    /// Format: VEX.NDS.128/256.66.0F opcode /r
    pub(crate) fn encode_avx_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            // src_reg, vvvv_reg, dst_reg
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // mem, vvvv_reg, dst_reg
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// Encode AVX 3-operand with no 66 prefix
    pub(crate) fn encode_avx_3op_np(&mut self, ops: &[Operand], opcode: u8) -> Result<(), String> {
        self.encode_avx_3op(ops, opcode, false)
    }

    /// Encode AVX 3-operand in 0F38 map
    pub(crate) fn encode_avx_3op_38(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX 3-op requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 2, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 2, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 3-op operands".to_string()),
        }
    }

    /// Encode AVX 3-operand in 0F map (mm=1) with imm8 (vshufps, vshufpd, etc.)
    pub(crate) fn encode_avx_3op_0f_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 3-op+imm8 requires 4 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp); // mm=1 (0F map)
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 3-op+imm8 operands".to_string()),
        }
    }

    /// Encode AVX 2-operand in 0F38 map (e.g., vpabsb src, dst with vvvv=0)
    pub(crate) fn encode_avx_2op_38(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX 2-op requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, 0, 0, l, pp); // vvvv=0 for 2-operand
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 2-op operands".to_string()),
        }
    }

    /// Encode AVX 2-operand in 0F map (e.g., vmovddup, vmovshdup, vmovsldup)
    /// pp: 0=NP, 1=66, 2=F3, 3=F2
    pub(crate) fn encode_avx_2op_0f(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX 2-op requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp); // mm=1 (0F), vvvv=0
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX 2-op operands".to_string()),
        }
    }

    /// Encode AVX scalar comparison (vcmpss/vcmpsd) with F3/F2 prefix
    /// pp: 2=F3 (vcmpss), 3=F2 (vcmpsd)
    pub(crate) fn encode_avx_cmp_scalar(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX scalar cmp requires 4 operands (imm8, src, vvvv, dst)".to_string());
        }
        let l = 0; // LIG, use 128-bit

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX scalar cmp operands".to_string()),
        }
    }

    /// Encode AVX scalar 3-operand instruction (e.g. vmulss, vaddss)
    /// pp: 2=F3 (single), 3=F2 (double)
    pub(crate) fn encode_avx_scalar_3op(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX scalar 3-op requires 3 operands".to_string());
        }
        let l = 0; // LIG - always 128-bit for scalar

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported AVX scalar 3-op operands".to_string()),
        }
    }

    /// Encode AVX scalar move (vmovss/vmovsd) - handles both 2-op (load/store) and 3-op (merge) forms
    /// pp: 2=F3 (vmovss), 3=F2 (vmovsd)
    pub(crate) fn encode_avx_scalar_mov(
        &mut self,
        ops: &[Operand],
        load_op: u8,
        store_op: u8,
        pp: u8,
    ) -> Result<(), String> {
        match ops.len() {
            2 => {
                // 2-operand load/store form (no vvvv merge)
                match (&ops[0], &ops[1]) {
                    (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&dst.name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 1, 0, 0, 0, pp); // vvvv=0, L=0
                        self.bytes.push(load_op);
                        self.encode_modrm_mem(dst_num, mem)
                    }
                    (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&src.name);
                        let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                        let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                        self.emit_vex(r, x, b_ext, 1, 0, 0, 0, pp); // vvvv=0, L=0
                        self.bytes.push(store_op);
                        self.encode_modrm_mem(src_num, mem)
                    }
                    (Operand::Register(src), Operand::Register(dst))
                        if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
                    {
                        // reg-reg: use load form
                        let src_num = reg_num(&src.name).ok_or("bad register")?;
                        let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                        let r = needs_vex_ext(&dst.name);
                        let b = needs_vex_ext(&src.name);
                        self.emit_vex(r, false, b, 1, 0, 0, 0, pp);
                        self.bytes.push(load_op);
                        self.bytes.push(self.modrm(3, dst_num, src_num));
                        Ok(())
                    }
                    _ => Err("unsupported AVX scalar mov 2-op operands".to_string()),
                }
            }
            3 => {
                // 3-operand merge form (VEX.NDS)
                self.encode_avx_scalar_3op(ops, load_op, pp)
            }
            _ => Err("AVX scalar mov requires 2 or 3 operands".to_string()),
        }
    }

    /// Encode AVX shuffle in 0F3A map (e.g. vpermilps, vpermilpd with immediate)
    /// Format: VEX.128/256.66.0F3A opcode /r ib
    pub(crate) fn encode_avx_shuffle_3a(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX shuffle 3A requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 3, 0, 0, l, pp); // mm=3 (0F3A), vvvv=0
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX shuffle 3A operands".to_string()),
        }
    }

    /// Parse SSE comparison predicate from pseudo-op mnemonic.
    /// Returns (predicate, suffix) e.g. "cmpnleps" -> Some((6, "ps"))
    pub(crate) fn parse_sse_cmp_pseudo(mnemonic: &str) -> Option<(u8, &str)> {
        if !mnemonic.starts_with("cmp") {
            return None;
        }
        let rest = &mnemonic[3..];
        // Try to match a suffix (ps, pd, ss, sd)
        let suffixes = ["ps", "pd", "ss", "sd"];
        for suffix in &suffixes {
            if let Some(pred_str) = rest.strip_suffix(*suffix) {
                let pred = match pred_str {
                    "eq" => 0,
                    "lt" => 1,
                    "le" => 2,
                    "unord" => 3,
                    "neq" => 4,
                    "nlt" => 5,
                    "nle" => 6,
                    "ord" => 7,
                    _ => continue,
                };
                return Some((pred, suffix));
            }
        }
        None
    }

    /// Try to encode an SSE comparison pseudo-op (e.g. cmpnleps -> cmpps $6, src, dst)
    pub(crate) fn try_encode_sse_cmp_pseudo(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<Option<()>, String> {
        let (pred, suffix) = match Self::parse_sse_cmp_pseudo(mnemonic) {
            Some(v) => v,
            None => return Ok(None),
        };
        let opcode: &[u8] = match suffix {
            "ps" => &[0x0F, 0xC2],
            "pd" => &[0x66, 0x0F, 0xC2],
            "ss" => &[0xF3, 0x0F, 0xC2],
            "sd" => &[0xF2, 0x0F, 0xC2],
            _ => return Ok(None),
        };
        if ops.len() != 2 {
            return Err(format!("{} requires 2 operands", mnemonic));
        }
        // Encode as the base instruction with an implicit immediate predicate
        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let prefix_len = opcode.iter().position(|&b| b == 0x0F).unwrap_or(0);
                for &b in &opcode[..prefix_len] {
                    self.bytes.push(b);
                }
                self.emit_rex_rr(0, &dst.name, &src.name);
                self.bytes.extend_from_slice(&opcode[prefix_len..]);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(pred);
                Ok(Some(()))
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let prefix_len = opcode.iter().position(|&b| b == 0x0F).unwrap_or(0);
                for &b in &opcode[..prefix_len] {
                    self.bytes.push(b);
                }
                self.emit_rex_rm(0, &dst.name, mem);
                self.bytes.extend_from_slice(&opcode[prefix_len..]);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(pred);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(Some(()))
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Parse AVX comparison predicate from pseudo-op mnemonic.
    /// Returns (predicate, suffix) e.g. "vcmpnleps" -> Some((6, "ps"))
    pub(crate) fn parse_avx_cmp_pseudo(mnemonic: &str) -> Option<(u8, &str)> {
        if !mnemonic.starts_with("vcmp") {
            return None;
        }
        let rest = &mnemonic[4..];
        let suffixes = ["ps", "pd", "ss", "sd"];
        for suffix in &suffixes {
            if let Some(pred_str) = rest.strip_suffix(*suffix) {
                let pred = match pred_str {
                    "eq" => 0,
                    "lt" => 1,
                    "le" => 2,
                    "unord" => 3,
                    "neq" => 4,
                    "nlt" => 5,
                    "nle" => 6,
                    "ord" => 7,
                    // AVX extended predicates (8-31)
                    "eq_uq" => 8,
                    "nge" => 9,
                    "ngt" => 10,
                    "false" => 11,
                    "neq_oq" => 12,
                    "ge" => 13,
                    "gt" => 14,
                    "true" => 15,
                    "eq_os" => 16,
                    "lt_oq" => 17,
                    "le_oq" => 18,
                    "unord_s" => 19,
                    "neq_us" => 20,
                    "nlt_uq" => 21,
                    "nle_uq" => 22,
                    "ord_s" => 23,
                    "eq_us" => 24,
                    "nge_uq" => 25,
                    "ngt_uq" => 26,
                    "false_os" => 27,
                    "neq_os" => 28,
                    "ge_oq" => 29,
                    "gt_oq" => 30,
                    "true_us" => 31,
                    _ => continue,
                };
                return Some((pred, suffix));
            }
        }
        None
    }

    /// Try to encode an AVX comparison pseudo-op (e.g. vcmpnleps -> vcmpps $6, src, vvvv, dst)
    pub(crate) fn try_encode_avx_cmp_pseudo(
        &mut self,
        ops: &[Operand],
        mnemonic: &str,
    ) -> Result<Option<()>, String> {
        let (pred, suffix) = match Self::parse_avx_cmp_pseudo(mnemonic) {
            Some(v) => v,
            None => return Ok(None),
        };
        // AVX pseudo-ops take 3 operands: src, vvvv, dst (no explicit immediate)
        if ops.len() != 3 {
            return Err(format!("{} requires 3 operands", mnemonic));
        }
        let (has_66, pp_scalar) = match suffix {
            "ps" => (false, None),
            "pd" => (true, None),
            "ss" => (false, Some(2u8)), // F3
            "sd" => (false, Some(3u8)), // F2
            _ => return Ok(None),
        };

        let l = self.vex_l_from_ops(ops);
        let pp = match pp_scalar {
            Some(p) => p,
            None => {
                if has_66 {
                    1
                } else {
                    0
                }
            }
        };

        match (&ops[0], &ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(vvvv), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(0xC2);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(pred);
                Ok(Some(()))
            }
            (Operand::Memory(mem), Operand::Register(vvvv), Operand::Register(dst)) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 1, 0, vvvv_enc, l, pp);
                self.bytes.push(0xC2);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(pred);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(Some(()))
            }
            _ => Err(format!("unsupported {} operands", mnemonic)),
        }
    }

    /// Encode AVX 3-operand in 0F3A map with imm8
    pub(crate) fn encode_avx_3op_3a_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 3-op+imm8 requires 4 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX 3-op+imm8 operands".to_string()),
        }
    }

    /// Encode AVX vbroadcastss/vbroadcastsd
    pub(crate) fn encode_avx_broadcast(
        &mut self,
        ops: &[Operand],
        opcode: &[u8],
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vbroadcast requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);

        match (&ops[0], &ops[1]) {
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                // VEX.256.66.0F38 opcode /r
                self.emit_vex(r, x, b_ext, 2, 0, 0, l, 1);
                self.bytes.extend_from_slice(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, 0, 0, l, 1);
                self.bytes.extend_from_slice(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported vbroadcast operands".to_string()),
        }
    }

    /// Encode AVX pshufd-like (imm8 + 2 register operands)
    pub(crate) fn encode_avx_shuffle(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX shuffle requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX shuffle operands".to_string()),
        }
    }

    /// Encode AVX vpmovmskb-like (xmm->gp)
    pub(crate) fn encode_avx_extract_gp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("AVX extract requires 2 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1]) {
            (Operand::Register(src), Operand::Register(dst)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode AVX vmovd
    pub(crate) fn encode_avx_movd(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vmovd requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            // GP -> XMM: VEX.128.66.0F 6E /r
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 1);
                self.bytes.push(0x6E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // XMM -> GP: VEX.128.66.0F 7E /r
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && !is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 1);
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            // mem -> XMM: VEX.128.66.0F 6E /r
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0x6E);
                self.encode_modrm_mem(dst_num, mem)
            }
            // XMM -> mem: VEX.128.66.0F 7E /r
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0x7E);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported vmovd operands".to_string()),
        }
    }

    /// Encode AVX vmovq
    pub(crate) fn encode_avx_movq(&mut self, ops: &[Operand]) -> Result<(), String> {
        if ops.len() != 2 {
            return Err("vmovq requires 2 operands".to_string());
        }
        match (&ops[0], &ops[1]) {
            // GP64 -> XMM: VEX.128.66.0F.W1 6E /r
            (Operand::Register(src), Operand::Register(dst))
                if !is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 1, 0, 0, 1);
                self.bytes.push(0x6E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // XMM -> GP64: VEX.128.66.0F.W1 7E /r
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && !is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 1, 1, 0, 0, 1);
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                Ok(())
            }
            // XMM -> XMM: VEX.128.F3.0F 7E /r
            (Operand::Register(src), Operand::Register(dst))
                if is_xmm_or_ymm(&src.name) && is_xmm_or_ymm(&dst.name) =>
            {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 1, 0, 0, 0, 2);
                self.bytes.push(0x7E);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            // mem -> XMM: VEX.128.F3.0F 7E /r
            (Operand::Memory(mem), Operand::Register(dst)) if is_xmm_or_ymm(&dst.name) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 2);
                self.bytes.push(0x7E);
                self.encode_modrm_mem(dst_num, mem)
            }
            // XMM -> mem: VEX.128.66.0F D6 /r
            (Operand::Register(src), Operand::Memory(mem)) if is_xmm_or_ymm(&src.name) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 1, 0, 0, 0, 1);
                self.bytes.push(0xD6);
                self.encode_modrm_mem(src_num, mem)
            }
            _ => Err("unsupported vmovq operands".to_string()),
        }
    }

    /// Encode AVX shift instructions (imm8 form or xmm form)
    pub(crate) fn encode_avx_shift(
        &mut self,
        ops: &[Operand],
        reg_op: u8,
        imm_ext: u8,
        imm_op: u8,
        has_66: bool,
    ) -> Result<(), String> {
        let pp = if has_66 { 1 } else { 0 };
        if ops.len() == 3 {
            match (&ops[0], &ops[1], &ops[2]) {
                // $imm, %xmm_src, %xmm_dst  (immediate shift, dst = vvvv)
                (
                    Operand::Immediate(ImmediateValue::Integer(imm)),
                    Operand::Register(src),
                    Operand::Register(dst),
                ) => {
                    let src_num = reg_num(&src.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    let l = if is_ymm(&src.name) || is_ymm(&dst.name) {
                        1
                    } else {
                        0
                    };
                    let b = needs_vex_ext(&src.name);
                    let vvvv_enc = dst_num | (if needs_vex_ext(&dst.name) { 8 } else { 0 });
                    self.emit_vex(false, false, b, 1, 0, vvvv_enc, l, pp);
                    self.bytes.push(imm_op);
                    self.bytes.push(self.modrm(3, imm_ext, src_num));
                    self.bytes.push(*imm as u8);
                    Ok(())
                }
                // %xmm_count, %xmm_src(vvvv), %xmm_dst
                (Operand::Register(count), Operand::Register(vvvv), Operand::Register(dst))
                    if is_xmm_or_ymm(&count.name) =>
                {
                    let count_num = reg_num(&count.name).ok_or("bad register")?;
                    let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                    let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                    let l = if is_ymm(&vvvv.name) || is_ymm(&dst.name) {
                        1
                    } else {
                        0
                    };
                    let r = needs_vex_ext(&dst.name);
                    let b = needs_vex_ext(&count.name);
                    let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                    self.emit_vex(r, false, b, 1, 0, vvvv_enc, l, pp);
                    self.bytes.push(reg_op);
                    self.bytes.push(self.modrm(3, dst_num, count_num));
                    Ok(())
                }
                _ => Err("unsupported AVX shift operands".to_string()),
            }
        } else {
            Err("AVX shift requires 3 operands".to_string())
        }
    }

    /// Encode x87 register-register arithmetic (fadd, fmul, fsub, fdiv).
    ///
    /// In AT&T syntax:
    ///   fadd %st(i), %st    -> D8 (base_st0 + i)  -- st(0) = st(0) op st(i)
    ///   fadd %st, %st(i)    -> DC (base_sti + i)  -- st(i) = st(i) op st(0)
    ///   fadd %st(i)         -> D8 (base_st0 + i)  -- shorthand for fadd %st(i), %st
    ///
    /// `opcode_st0` = D8 (reg field in modrm for st(0) as dest)
    /// `opcode_sti` = DC (reg field in modrm for st(i) as dest)
    /// `base_modrm` = base for the modrm second byte (e.g., 0xC0 for fadd)
    pub(crate) fn encode_x87_arith_reg(
        &mut self,
        ops: &[Operand],
        opcode_st0: u8,
        opcode_sti: u8,
        base_modrm: u8,
    ) -> Result<(), String> {
        match ops.len() {
            0 => {
                // Default: fadd %st(1), %st (i.e., st(0) = st(0) op st(1))
                self.bytes.extend_from_slice(&[opcode_st0, base_modrm + 1]);
                Ok(())
            }
            1 => {
                // fadd %st(i) -> st(0) = st(0) op st(i)
                match &ops[0] {
                    Operand::Register(reg) => {
                        let n = parse_st_num(&reg.name)?;
                        self.bytes.extend_from_slice(&[opcode_st0, base_modrm + n]);
                        Ok(())
                    }
                    _ => Err("x87 arith requires st register operand".to_string()),
                }
            }
            2 => {
                // Two operands: fadd %st(i), %st or fadd %st, %st(i)
                match (&ops[0], &ops[1]) {
                    (Operand::Register(src), Operand::Register(dst)) => {
                        let src_n = parse_st_num(&src.name)?;
                        let dst_n = parse_st_num(&dst.name)?;
                        if dst_n == 0 {
                            // fadd %st(i), %st -> D8 (base + i)
                            self.bytes
                                .extend_from_slice(&[opcode_st0, base_modrm + src_n]);
                        } else if src_n == 0 {
                            // fadd %st, %st(i) -> DC (base + i)
                            // Note: for fsub/fdiv, the DC form uses reversed base
                            // DC E0+i for fsubr, DC E8+i for fsub (swapped!)
                            // But base_modrm is from the D8 encoding perspective.
                            // The DC form for reverse direction:
                            // fadd: DC C0+i, fmul: DC C8+i, fsub: DC E8+i, fdiv: DC F8+i
                            // (fsub/fdiv swap: D8 E0 = fsub st(i),st; DC E8 = fsub st,st(i))
                            let dc_modrm = match base_modrm {
                                0xC0 => 0xC0, // fadd
                                0xC8 => 0xC8, // fmul
                                0xE0 => 0xE8, // fsub -> fsubr encoding in DC
                                0xF0 => 0xF8, // fdiv -> fdivr encoding in DC
                                _ => base_modrm,
                            };
                            self.bytes
                                .extend_from_slice(&[opcode_sti, dc_modrm + dst_n]);
                        } else {
                            return Err("x87 arith: one operand must be st(0)".to_string());
                        }
                        Ok(())
                    }
                    _ => Err("x87 arith requires st register operands".to_string()),
                }
            }
            _ => Err("x87 arith requires 0-2 operands".to_string()),
        }
    }

    /// Encode fxch (exchange st(0) with st(i)).
    pub(crate) fn encode_fxch(&mut self, ops: &[Operand]) -> Result<(), String> {
        let n = match ops.len() {
            0 => 1, // fxch defaults to st(1)
            1 => match &ops[0] {
                Operand::Register(reg) => parse_st_num(&reg.name)?,
                _ => return Err("fxch requires st register".to_string()),
            },
            _ => return Err("fxch requires 0 or 1 operand".to_string()),
        };
        self.bytes.extend_from_slice(&[0xD9, 0xC8 + n]);
        Ok(())
    }

    /// Infer BMI2 W bit (0=32-bit, 1=64-bit) from destination register.
    pub(crate) fn bmi2_infer_w(&self, ops: &[Operand]) -> u8 {
        // Check destination (last operand) for register size
        if let Some(Operand::Register(r)) = ops.last() {
            if is_reg64(&r.name) {
                return 1;
            }
        }
        // Check other register operands
        for op in ops {
            if let Operand::Register(r) = op {
                if is_reg64(&r.name) {
                    return 1;
                }
                if is_reg32(&r.name) {
                    return 0;
                }
            }
        }
        1 // default to 64-bit
    }

    /// Encode AVX extract with imm8 (vextracti128, vextractf128)
    /// Format: VEX.256.66.0F3A opcode /r ib
    /// AT&T: $imm8, %src_ymm, %dst_xmm/mem
    pub(crate) fn encode_avx_extract_imm8(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX extract requires 3 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 3, 0, 0, 1, pp); // L=1 (256-bit source)
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, 1, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode AVX shuffle in 0F3A map with W=1 (vpermq, vpermpd)
    /// Format: VEX.256.66.0F3A.W1 opcode /r ib
    pub(crate) fn encode_avx_shuffle_3a_w1(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX permq requires 3 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 3, 1, 0, l, pp); // W=1
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(dst),
            ) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 1, 0, l, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX permq operands".to_string()),
        }
    }

    /// Encode AVX 4-operand instruction in 0F3A map (vblendvps, vblendvpd, vpblendvb)
    /// AT&T: $imm/mask, src, vvvv, dst -> actually: src_mask, src, vvvv, dst
    /// Intel: dst, vvvv, src, mask_reg
    /// VEX.NDS.128/256.66.0F3A opcode /r /is4
    pub(crate) fn encode_avx_4op_3a(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX 4-op requires 4 operands".to_string());
        }
        let l = self.vex_l_from_ops(ops);
        let pp = if has_66 { 1 } else { 0 };

        // AT&T: %mask, %src, %vvvv, %dst
        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Register(mask),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let mask_num = reg_num(&mask.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, l, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                // is4: mask register encoded in imm8[7:4]
                let mask_full = mask_num | (if needs_vex_ext(&mask.name) { 8 } else { 0 });
                self.bytes.push((mask_full & 0xF) << 4);
                Ok(())
            }
            _ => Err("unsupported AVX 4-op operands".to_string()),
        }
    }

    /// Encode AVX insert from GP register (vpinsrb, vpinsrd) via 0F3A map
    /// AT&T: $imm8, %gp/%mem, %xmm_vvvv, %xmm_dst
    pub(crate) fn encode_avx_insert_gp(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX insert requires 4 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Memory(mem),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, x, b_ext, 3, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX insert operands".to_string()),
        }
    }

    /// Encode AVX insert via 0F map (vpinsrw)
    pub(crate) fn encode_avx_insert_gp_0f(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX insert requires 4 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 1, 0, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX insert operands".to_string()),
        }
    }

    /// Encode AVX insert with W=1 (vpinsrq)
    pub(crate) fn encode_avx_insert_gp_w1(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 4 {
            return Err("AVX insert requires 4 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2], &ops[3]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(vvvv),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let vvvv_num = reg_num(&vvvv.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv.name) { 8 } else { 0 });
                self.emit_vex(r, false, b, 3, 1, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            _ => Err("unsupported AVX insert operands".to_string()),
        }
    }

    /// Encode AVX extract byte/dword (vpextrb, vpextrd) via 0F3A map
    pub(crate) fn encode_avx_extract_byte(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        has_66: bool,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("AVX extract requires 3 operands".to_string());
        }
        let pp = if has_66 { 1 } else { 0 };

        match (&ops[0], &ops[1], &ops[2]) {
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Register(dst),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b = needs_vex_ext(&dst.name);
                self.emit_vex(r, false, b, 3, 0, 0, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, src_num, dst_num));
                self.bytes.push(*imm as u8);
                Ok(())
            }
            (
                Operand::Immediate(ImmediateValue::Integer(imm)),
                Operand::Register(src),
                Operand::Memory(mem),
            ) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let r = needs_vex_ext(&src.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, 0, 0, 0, pp);
                self.bytes.push(opcode);
                let rc = self.relocations.len();
                self.encode_modrm_mem(src_num, mem)?;
                self.bytes.push(*imm as u8);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("unsupported AVX extract operands".to_string()),
        }
    }

    /// Encode BMI2 3-operand GPR instructions (shrxq, shlxq, sarxq, etc.).
    ///
    /// AT&T syntax: `shrxq %src_shift, %r/m, %dst`
    ///   ops[0] = shift count register → VEX.vvvv
    ///   ops[1] = source r/m → ModRM r/m
    ///   ops[2] = destination → ModRM reg
    ///
    /// All use 0F38 map (mm=2). pp selects prefix: 0=NP, 1=66, 2=F3, 3=F2.
    pub(crate) fn encode_bmi2_shift(
        &mut self,
        ops: &[Operand],
        opcode: u8,
        pp: u8,
        w: u8,
    ) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("BMI2 instruction requires 3 operands".to_string());
        }

        let vvvv_reg = match &ops[0] {
            Operand::Register(r) => r,
            _ => return Err("BMI2: first operand must be register".to_string()),
        };
        let vvvv_num = reg_num(&vvvv_reg.name).ok_or("bad vvvv register")?;
        let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv_reg.name) { 8 } else { 0 });

        match (&ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, w, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, w, vvvv_enc, 0, pp);
                self.bytes.push(opcode);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("BMI2: unsupported operand combination".to_string()),
        }
    }

    /// Encode BMI1 ANDN: andnl %src2, %src1, %dst → dst = ~src1 & src2
    /// AT&T operand order: ops[0]=src2(r/m), ops[1]=src1(vvvv), ops[2]=dst(reg)
    pub(crate) fn encode_bmi_andn(&mut self, ops: &[Operand], w: u8) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("andn requires 3 operands".to_string());
        }

        // ops[1] = src1 → VEX.vvvv
        let vvvv_reg = match &ops[1] {
            Operand::Register(r) => r,
            _ => return Err("andn: second operand must be register".to_string()),
        };
        let vvvv_num = reg_num(&vvvv_reg.name).ok_or("bad vvvv register")?;
        let vvvv_enc = vvvv_num | (if needs_vex_ext(&vvvv_reg.name) { 8 } else { 0 });

        match (&ops[0], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                self.emit_vex(r, false, b, 2, w, vvvv_enc, 0, 0); // NP.0F38
                self.bytes.push(0xF2);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 2, w, vvvv_enc, 0, 0);
                self.bytes.push(0xF2);
                self.encode_modrm_mem(dst_num, mem)
            }
            _ => Err("unsupported andn operands".to_string()),
        }
    }

    /// Encode BMI2 rorx (rotate right logical with immediate).
    /// AT&T syntax: `rorxq $imm8, %r/m, %dst`
    ///   ops[0] = immediate
    ///   ops[1] = source r/m
    ///   ops[2] = destination
    pub(crate) fn encode_bmi2_rorx(&mut self, ops: &[Operand], w: u8) -> Result<(), String> {
        if ops.len() != 3 {
            return Err("rorx requires 3 operands".to_string());
        }

        let imm = match &ops[0] {
            Operand::Immediate(ImmediateValue::Integer(val)) => *val as u8,
            _ => return Err("rorx: first operand must be immediate".to_string()),
        };

        match (&ops[1], &ops[2]) {
            (Operand::Register(src), Operand::Register(dst)) => {
                let src_num = reg_num(&src.name).ok_or("bad src register")?;
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b = needs_vex_ext(&src.name);
                // VEX.LZ.F2.0F3A.Wx F0 /r imm8 (mm=3 for 0F3A, pp=3 for F2)
                self.emit_vex(r, false, b, 3, w, 0, 0, 3);
                self.bytes.push(0xF0);
                self.bytes.push(self.modrm(3, dst_num, src_num));
                self.bytes.push(imm);
                Ok(())
            }
            (Operand::Memory(mem), Operand::Register(dst)) => {
                let dst_num = reg_num(&dst.name).ok_or("bad dst register")?;
                let r = needs_vex_ext(&dst.name);
                let b_ext = mem.base.as_ref().is_some_and(|b| needs_vex_ext(&b.name));
                let x = mem.index.as_ref().is_some_and(|i| needs_vex_ext(&i.name));
                self.emit_vex(r, x, b_ext, 3, w, 0, 0, 3);
                self.bytes.push(0xF0);
                let rc = self.relocations.len();
                self.encode_modrm_mem(dst_num, mem)?;
                self.bytes.push(imm);
                self.adjust_rip_reloc_addend(rc, 1);
                Ok(())
            }
            _ => Err("rorx: unsupported operand combination".to_string()),
        }
    }
}
