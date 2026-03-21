//! RiscvCodegen: global address loading, PIC-relative.

use super::emit::RiscvCodegen;
use crate::ir::reexports::Value;

impl RiscvCodegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.needs_got(name) {
            // PIC mode, external symbol: use `la` which expands to GOT-indirect
            // auipc + ld with R_RISCV_GOT_HI20 relocation.
            self.state.emit_fmt(format_args!("    la t0, {}", name));
        } else {
            // Non-PIC mode or local symbol: use `lla` (local load address) which
            // expands to PC-relative auipc + addi with R_RISCV_PCREL_HI20.
            self.state.emit_fmt(format_args!("    lla t0, {}", name));
        }
        self.store_t0_to(dest);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        // Labels are always local symbols -- use lla (local load address) to avoid
        // GOT indirection that `la` may produce in PIC mode.
        self.state.emit_fmt(format_args!("    lla t0, {}", label));
        self.store_t0_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        // Local Exec TLS model for RISC-V:
        // lui t0, %tprel_hi(x)
        // add t0, t0, tp, %tprel_add(x)
        // addi t0, t0, %tprel_lo(x)
        self.state
            .emit_fmt(format_args!("    lui t0, %tprel_hi({})", name));
        self.state
            .emit_fmt(format_args!("    add t0, t0, tp, %tprel_add({})", name));
        self.state
            .emit_fmt(format_args!("    addi t0, t0, %tprel_lo({})", name));
        self.store_t0_to(dest);
    }
}
