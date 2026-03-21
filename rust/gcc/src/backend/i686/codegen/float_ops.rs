//! I686Codegen: F128 negation.

use super::emit::I686Codegen;
use crate::emit;
use crate::ir::reexports::{Operand, Value};

impl I686Codegen {
    pub(super) fn emit_f128_neg_impl(&mut self, dest: &Value, src: &Operand) {
        self.emit_f128_load_to_x87(src);
        self.state.emit("    fchs");
        if let Some(slot) = self.state.get_slot(dest.0) {
            let sr = self.slot_ref(slot);
            emit!(self.state, "    fstpt {}", sr);
            self.state.f128_direct_slots.insert(dest.0);
        }
        self.state.reg_cache.invalidate_acc();
    }
}
