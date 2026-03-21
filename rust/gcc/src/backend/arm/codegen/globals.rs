//! ArmCodegen: global address operations.

use super::emit::ArmCodegen;
use crate::ir::reexports::Value;

impl ArmCodegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.needs_got_aarch64(name) {
            self.state
                .emit_fmt(format_args!("    adrp x0, :got:{}", name));
            self.state
                .emit_fmt(format_args!("    ldr x0, [x0, :got_lo12:{}]", name));
        } else {
            self.state.emit_fmt(format_args!("    adrp x0, {}", name));
            self.state
                .emit_fmt(format_args!("    add x0, x0, :lo12:{}", name));
        }
        self.store_x0_to(dest);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        self.state.emit_fmt(format_args!("    adrp x0, {}", label));
        self.state
            .emit_fmt(format_args!("    add x0, x0, :lo12:{}", label));
        self.store_x0_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        self.state.emit("    mrs x0, tpidr_el0");
        self.state
            .emit_fmt(format_args!("    add x0, x0, :tprel_hi12:{}", name));
        self.state
            .emit_fmt(format_args!("    add x0, x0, :tprel_lo12_nc:{}", name));
        self.store_x0_to(dest);
    }
}
