//! The LLVM IR data model.
//!
//! A [`Module`] owns a [`Context`] holding its interned types and constants,
//! a list of globals and functions, an attribute-group table and a metadata
//! table. Everything is addressed by small integer ids rather than by
//! reference, so the whole structure is `Send + Sync`, cheap to copy around
//! and free of the borrow tangles a pointer graph would cause.
//!
//! The dialect is LLVM 21 with opaque pointers. Constructs from older
//! dialects are not accepted anywhere, and constructs this crate does not
//! model yet are errors rather than omissions.
//!
//! ```
//! use llvm_ir::{Module, Name};
//! use llvm_ir::function::{BasicBlock, Function};
//! use llvm_ir::instruction::{InstKind, Instruction};
//!
//! let mut module = Module::new();
//! let i32 = module.ctx.int_type(32);
//! let zero = module.ctx.const_int_of(32, 0);
//!
//! let mut main = Function::new(Name::named("main"), i32);
//! let entry = main.add_block(BasicBlock::default());
//! let ret = main.add_instruction(Instruction::new(
//!     module.ctx.void_type(),
//!     InstKind::Ret(Some((i32, llvm_ir::Value::Constant(zero)))),
//! ));
//! main.block_mut(entry).instructions.push(ret);
//! module.add_function(main);
//! ```

pub mod attribute;
pub mod constant;
pub mod context;
pub mod function;
pub mod global;
pub mod instruction;
mod keyword;
pub mod layout;
pub mod metadata;
pub mod module;
pub mod types;
pub mod value;

pub use context::Context;
pub use module::Module;
pub use types::{StructId, TypeId, TypeKind};
pub use value::{
    AliasId, BlockId, FunctionId, GlobalRef, GlobalVarId, IFuncId, InstId, MdId, Name, Value,
};
