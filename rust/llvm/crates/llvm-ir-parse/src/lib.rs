//! The textual IR parser.
//!
//! A hand-written recursive descent over the token stream from [`lexer`].
//! Every error carries a line and a column, and anything the crate does not
//! model is an error rather than something quietly skipped: a dropped
//! attribute or a swallowed metadata node is a miscompilation waiting for a
//! later tier.
//!
//! Two deliberate departures from upstream's parser, both recorded in
//! `docs/dialect-notes.md`:
//!
//! * The `; ModuleID = '...'` comment is read rather than ignored, so that a
//!   module prints back the identity it came in with. Upstream regenerates it
//!   from the input path.
//! * Unnamed values and blocks may be numbered non-consecutively. Upstream
//!   renumbers them on output; we keep a map from the written number, which
//!   accepts strictly more input and changes nothing for canonical files.

mod attributes;
mod body;
mod constants;
mod globals;
mod lexer;
mod md_schema;
mod metadata;
mod parser;
mod toplevel;
mod types;

use std::collections::HashMap;

use lexer::{Lexer, Position, Spanned};
use llvm_ir::Module;
use llvm_ir::value::{BlockId, GlobalRef, InstId, Name};

pub use lexer::LexError;

/// Why the input is not a module.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    pub position: Position,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.position, self.message)
    }
}

impl std::error::Error for ParseError {}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> ParseError {
        ParseError {
            position: error.position,
            message: error.message,
        }
    }
}

/// Reads a module from textual IR.
pub fn parse_module(text: &str) -> Result<Module, ParseError> {
    let (tokens, module_id) = Lexer::new(text).tokenize()?;
    let mut parser = Parser {
        tokens,
        index: 0,
        module: Module::new(),
        symbols: HashMap::new(),
    };
    parser.module.module_id = module_id;
    parser.prescan_symbols()?;
    parser.parse_top_level()?;
    Ok(parser.module)
}

pub(crate) struct Parser {
    pub(crate) tokens: Vec<Spanned>,
    pub(crate) index: usize,
    pub(crate) module: Module,
    /// Every global-scope name and the id it will get, worked out before
    /// parsing so that forward references resolve without placeholders.
    pub(crate) symbols: HashMap<Name, GlobalRef>,
}

/// Everything the parser has to remember while inside one function body.
#[derive(Default)]
pub(crate) struct FunctionState {
    pub(crate) named_values: HashMap<String, InstId>,
    pub(crate) numbered_values: HashMap<u32, InstId>,
    pub(crate) named_blocks: HashMap<String, BlockId>,
    pub(crate) numbered_blocks: HashMap<u32, BlockId>,
    pub(crate) named_params: HashMap<String, u32>,
    pub(crate) numbered_params: HashMap<u32, u32>,
    /// The next number an unnamed value or block would take, shared between
    /// them the way upstream's slot tracker shares it.
    pub(crate) next_number: u32,
}

impl Parser {}
