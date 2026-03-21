// Re-export hub for all IR types.
//
// The IR is split into focused submodules for maintainability:
// - constants: IrConst, ConstHashKey, float encoding utilities
// - ops: IrBinOp, IrCmpOp, IrUnaryOp, AtomicRmwOp, AtomicOrdering
// - intrinsics: IntrinsicOp (SIMD, crypto, math builtins)
// - instruction: Instruction, Terminator, BasicBlock, BlockId, Value, Operand, CallInfo
// - module: IrModule, IrFunction, IrParam, IrGlobal, GlobalInit
//
// All types are re-exported here so `use crate::ir::reexports::*` imports work.
pub use super::constants::*;
pub use super::instruction::*;
pub use super::intrinsics::*;
pub use super::module::*;
pub use super::ops::*;
