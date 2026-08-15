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

use std::collections::{HashMap, HashSet};

use lexer::{Lexer, Position, Spanned};
use llvm_ir::value::{BlockId, FunctionId, GlobalRef, InstId, Name};
use llvm_ir::{Module, TypeId};

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
        implied_intrinsics: Vec::new(),
        implied_signatures: HashMap::new(),
        first_implied_id: FunctionId(0),
        extra_implied_ids: FunctionId(0),
        extra_implied_room: 0,
        extra_implied: Vec::new(),
        next_inline_metadata: 0,
        wrote_debug_record: false,
        wrote_debug_intrinsic: false,
        use_list_orders: Vec::new(),
        block_use_list_orders: Vec::new(),
    };
    parser.next_inline_metadata = parser
        .tokens
        .iter()
        .filter_map(|spanned| match spanned.token {
            lexer::Token::MetadataNumber(number) => Some(number + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    parser.module.module_id = module_id;
    parser.prescan_symbols()?;
    parser.parse_top_level()?;
    if parser.wrote_debug_record && parser.wrote_debug_intrinsic {
        return parser.error(
            "the module writes debug info as records in one place and as intrinsic calls in another",
        );
    }
    parser.add_implied_intrinsics();
    // After the declarations the calls implied, so that a name a call wrote
    // without its components is filled in the same way one a declaration
    // wrote is.
    parser.remangle_intrinsics();
    // After the renaming, which moves what it renames to the end: the two
    // sets never overlap, so the implied ones sort where they already sit.
    parser.sort_implied_declarations();
    // After the names are settled, so that a rewrite asks about the name the
    // intrinsic ended up with.
    parser.rewrite_intrinsic_calls();
    // After the declarations the calls implied, so that those get the
    // attributes too: upstream materialises one with them already on.
    parser.apply_intrinsic_attributes()?;
    // Before the printer reads a default alignment out of the layout, and
    // after the module has had every chance to write one of its own.
    parser.fill_data_layout_from_triple();
    parser.upgrade_module_flags();
    parser.upgrade_objc_module_flags();
    parser.drop_invalid_debug_info();
    parser.mark_self_referencing_distinct();
    parser.check_use_list_orders()?;
    Ok(parser.module)
}

pub(crate) struct Parser {
    pub(crate) tokens: Vec<Spanned>,
    pub(crate) index: usize,
    pub(crate) module: Module,
    /// Every global-scope name and the id it will get, worked out before
    /// parsing so that forward references resolve without placeholders.
    pub(crate) symbols: HashMap<Name, GlobalRef>,
    /// The `llvm.*` names nothing declares, sorted by the name the module
    /// wrote, which is the order upstream appends their declarations in.
    pub(crate) implied_intrinsics: Vec<Name>,
    /// What the first call to each of them says its signature is.
    pub(crate) implied_signatures: HashMap<Name, (TypeId, Vec<TypeId>)>,
    /// The id the first implied declaration took, the rest running on from
    /// it in the order `implied_intrinsics` holds.
    pub(crate) first_implied_id: FunctionId,
    /// The first id of the block reserved for the names that stand for more
    /// than one function, and how many ids that block holds.
    pub(crate) extra_implied_ids: FunctionId,
    pub(crate) extra_implied_room: usize,
    /// The further declarations those names asked for, in the order the calls
    /// asked, which is the order their ids run in. Each is the written name,
    /// the id it took, and the signature the call gave it.
    pub(crate) extra_implied: Vec<(Name, FunctionId, TypeId, Vec<TypeId>)>,
    /// The next number to give a node written in place. Upstream has no such
    /// thing: it numbers every node and refers to it, so a node written
    /// inside another is hoisted out and numbered. Starting past every
    /// number the text writes keeps the two from colliding.
    pub(crate) next_inline_metadata: u32,
    /// Whether the text wrote a `#dbg_` record, and whether it wrote a call
    /// to one of the intrinsics that are the older spelling of one. A module
    /// uses one spelling or the other, and upstream refuses a mix; by the
    /// time the module is built both look the same, so the two have to be
    /// noticed as they are read.
    pub(crate) wrote_debug_record: bool,
    pub(crate) wrote_debug_intrinsic: bool,
    /// Every `uselistorder` that named a constant, with where it was written
    /// and the indexes it gave. Checked once the module is whole, a global
    /// used by a later function not yet being used while the text is read.
    pub(crate) use_list_orders: Vec<(Position, llvm_ir::constant::ConstId, Vec<u64>)>,
    /// Every `uselistorder_bb`, likewise. A block's list holds the
    /// terminator slots that reach it, plus the `blockaddress` that names
    /// it, and that address can be written below the directive.
    pub(crate) block_use_list_orders:
        Vec<(Position, llvm_ir::value::FunctionId, BlockId, Vec<u64>)>,
}

/// Everything the parser has to remember while inside one function body.
#[derive(Default)]
pub(crate) struct FunctionState {
    pub(crate) named_values: HashMap<String, InstId>,
    /// The names that have been defined rather than only referred to, so a
    /// second definition is a redefinition rather than a forward reference
    /// arriving.
    pub(crate) defined_values: HashSet<String>,
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
