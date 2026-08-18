//! Values and the names they carry.

use crate::constant::ConstId;

/// An instruction inside a function's arena.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct InstId(pub u32);

/// A basic block inside a function's arena.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u32);

/// A metadata node. The number is the one the module prints, so `!7` in the
/// text is `MdId(7)` here and stays `!7` on the way out.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MdId(pub u32);

/// Index of a function in a module.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FunctionId(pub u32);

/// Index of a global variable in a module.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GlobalVarId(pub u32);

/// Index of an alias in a module.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct AliasId(pub u32);

/// Index of an ifunc in a module.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct IFuncId(pub u32);

/// Anything spelled with a leading `@`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum GlobalRef {
    Function(FunctionId),
    Variable(GlobalVarId),
    Alias(AliasId),
    IFunc(IFuncId),
}

/// An operand.
///
/// A value is eight bytes: a tag and an index. Instructions and blocks are
/// function-scoped, constants and metadata are module-scoped, and an argument
/// is an index into the enclosing function's parameter list.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Value {
    Instruction(InstId),
    Argument(u32),
    Constant(ConstId),
    Block(BlockId),
    Metadata(MdId),
}

impl Value {
    pub fn as_constant(self) -> Option<ConstId> {
        match self {
            Value::Constant(id) => Some(id),
            _ => None,
        }
    }

    pub fn as_block(self) -> Option<BlockId> {
        match self {
            Value::Block(id) => Some(id),
            _ => None,
        }
    }

    pub fn as_instruction(self) -> Option<InstId> {
        match self {
            Value::Instruction(id) => Some(id),
            _ => None,
        }
    }
}

/// The name of a local value, a global, or a basic block.
///
/// Values with no name print as a slot number assigned in order. Parsing
/// `%3` for such a value checks that 3 is the slot it would get anyway and
/// then stores no name, which is upstream's rule and keeps printing and
/// parsing from disagreeing about numbering.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Name {
    Named(String),
    /// An explicitly written number that is not the natural slot, which only
    /// arises for globals and for blocks in hand-written input.
    Number(u32),
}

impl Name {
    pub fn named(text: impl Into<String>) -> Name {
        Name::Named(text.into())
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Name::Named(text) => Some(text),
            Name::Number(_) => None,
        }
    }
}

/// Whether an identifier can be printed bare or has to be quoted.
///
/// LLVM accepts `[-a-zA-Z$._][-a-zA-Z$._0-9]*` unquoted; anything else, and
/// anything that would read back as a number, needs quotes and escapes.
pub fn needs_quotes(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    let mut chars = name.chars();
    let first = chars.next().expect("name is not empty");
    if first.is_ascii_digit() {
        return true;
    }
    !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '$' | '.' | '_'))
}

/// Escapes a name for printing between double quotes.
///
/// A backslash doubles, a double quote becomes `\22`, and anything outside
/// printable ASCII becomes a backslash and two uppercase hex digits. The
/// asymmetry between the two escaped characters is upstream's.
pub fn escape_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_follows_upstreams_identifier_rule() {
        assert!(!needs_quotes("main"));
        assert!(!needs_quotes("_ZN4core3fmt9Arguments6new_v1E"));
        assert!(!needs_quotes("a.b-c$d"));
        assert!(needs_quotes(""));
        assert!(needs_quotes("0abc"));
        assert!(needs_quotes("has space"));
        assert!(needs_quotes("unicode\u{e9}"));
    }

    #[test]
    fn escaping_uses_two_hex_digits() {
        assert_eq!(escape_name("plain"), "plain");
        assert_eq!(escape_name("with\"quote"), "with\\22quote");
        assert_eq!(escape_name("back\\slash"), "back\\\\slash");
        assert_eq!(escape_name("tab\there"), "tab\\09here");
        assert_eq!(escape_name("\u{e9}"), "\\C3\\A9");
    }
}
