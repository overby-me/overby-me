pub(crate) mod builtin_macros;
pub(crate) mod conditionals;
mod expr_eval;
mod includes;
pub(crate) mod macro_defs;
pub(crate) mod pipeline;
mod pragmas;
pub(crate) mod predefined_macros;
mod text_processing;
pub(crate) mod utils;

pub(crate) use pipeline::Preprocessor;
