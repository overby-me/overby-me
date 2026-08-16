//! The intrinsics upstream reads as an instruction rather than as a call.
//!
//! A rename gives a call a different callee; these do not survive as calls at
//! all. `@llvm.nvvm.atomic.load.inc.32.p0(ptr %p, i32 %v)` is read as
//! `atomicrmw uinc_wrap ptr %p, i32 %v seq_cst`, and the declaration goes
//! with it: upstream drops it whether or not anything called it.
//!
//! Every row was measured a module at a time, a declaration and one call
//! written out and the instruction read back:
//!
//! ```text
//!   llvm.nvvm.atomic.load.inc.32   atomicrmw uinc_wrap ptr, i32 seq_cst, align 4
//!   llvm.nvvm.atomic.load.dec.32   atomicrmw udec_wrap ptr, i32 seq_cst, align 4
//!   llvm.nvvm.atomic.load.add.f32  atomicrmw fadd ptr, float seq_cst, align 4
//!   llvm.nvvm.atomic.load.add.f64  atomicrmw fadd ptr, double seq_cst, align 8
//! ```
//!
//! The declaration's own types are not consulted, which is the point of the
//! module this exists for: `auto_upgrade_nvvm_intrinsics.ll` declares
//! `i32 @llvm.nvvm.atomic.load.add.f32.p0(ptr, float)` and calls it returning
//! `float`, and upstream never minds because by the time anything checks, the
//! call is an `atomicrmw` whose type came from the value it was given.
//!
//! This table is not a sweep. The four are what upstream's own tests exercise
//! and what one module needed; a rewrite is a fact about an intrinsic's
//! meaning rather than about its name, and there is no oracle that lists them
//! all. Anything not here stays a call.

use crate::instruction::AtomicRmwOp;

/// The read-modify-write operation this name is read as, if it is read as one.
pub fn atomic_rmw_op(name: &str) -> Option<AtomicRmwOp> {
    let base = super::base_name(name);
    match base {
        "llvm.nvvm.atomic.load.inc.32" => Some(AtomicRmwOp::UIncWrap),
        "llvm.nvvm.atomic.load.dec.32" => Some(AtomicRmwOp::UDecWrap),
        "llvm.nvvm.atomic.load.add" => Some(AtomicRmwOp::FAdd),
        _ => None,
    }
}

/// Whether a declaration of this name is one upstream does not write back.
///
/// Measured on its own, because it is not the same question: a declaration
/// nothing calls is dropped too, so the answer does not depend on whether the
/// rewrite had anything to do.
pub fn is_rewritten(name: &str) -> bool {
    atomic_rmw_op(name).is_some()
}
