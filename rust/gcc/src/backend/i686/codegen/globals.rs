//! I686Codegen: global address operations (global, label, TLS).

use super::emit::I686Codegen;
use crate::emit;
use crate::ir::reexports::Value;

impl I686Codegen {
    pub(super) fn emit_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.pic_mode {
            if self.state.needs_got(name) {
                emit!(self.state, "    movl {}@GOT(%ebx), %eax", name);
            } else {
                emit!(self.state, "    leal {}@GOTOFF(%ebx), %eax", name);
            }
        } else {
            emit!(self.state, "    movl ${}, %eax", name);
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_label_addr_impl(&mut self, dest: &Value, label: &str) {
        if self.state.pic_mode {
            emit!(self.state, "    leal {}@GOTOFF(%ebx), %eax", label);
        } else {
            emit!(self.state, "    movl ${}, %eax", label);
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }

    pub(super) fn emit_tls_global_addr_impl(&mut self, dest: &Value, name: &str) {
        if self.state.pic_mode {
            emit!(self.state, "    movl {}@GOTNTPOFF(%ebx), %eax", name);
            self.state.emit("    addl %gs:0, %eax");
        } else {
            self.state.emit("    movl %gs:0, %eax");
            emit!(self.state, "    addl ${}@NTPOFF, %eax", name);
        }
        self.state.reg_cache.invalidate_acc();
        self.store_eax_to(dest);
    }
}
