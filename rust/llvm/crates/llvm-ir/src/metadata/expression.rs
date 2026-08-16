//! What a `!DIExpression` holds, and what upstream reads back out of it.
//!
//! An expression is a flat list of numbers: an opcode, its operands, the next
//! opcode, and so on. `table.rs` beside this is the sweep of every code
//! upstream writes or reads, and `corpus/dwarf-expression.nu` explains how
//! each column was measured. The walk over that table is here, along with the
//! rules no column of it can express.
//!
//! This was recorded once as measured-and-not-a-table, wanting "the stack
//! discipline of a DWARF expression". It is a table, and what looked like a
//! whole-sequence property is four rules beside it: an opcode that may not
//! stand alone, an opcode that has to be last, an opcode after which nothing
//! is checked, and the entry value.

mod table;

use super::{MdField, dwarf};

pub use table::{Operation, code_for_word, operation, word};

/// `DW_OP_LLVM_entry_value`, whose rule is the one below rather than a row.
const ENTRY_VALUE: u64 = 4099;

/// `DW_OP_LLVM_arg`, which is what may come before an entry value.
const ARG: u64 = 4101;

/// `DW_OP_LLVM_convert`, whose second operand is written as an encoding.
const CONVERT: u64 = 4097;

/// The number an element written as a word stands for.
///
/// Upstream reads an operation's word and an encoding's, and nothing else: a
/// `DW_TAG_*` in an expression is refused where `DW_ATE_signed` is read as
/// five wherever it is written, not only where an encoding is expected.
pub fn code_for_element_word(word: &str) -> Option<u64> {
    if let Some(code) = code_for_word(word) {
        return Some(code);
    }
    for (code, spelling) in dwarf::ENCODING {
        if *spelling == word {
            return Some(*code);
        }
    }
    None
}

/// Whether a whole expression is one upstream reads.
pub fn is_valid(elements: &[u64]) -> bool {
    let mut index = 0;
    while index < elements.len() {
        let code = elements[index];
        if code == ENTRY_VALUE {
            return entry_value_is_valid(elements, index);
        }
        let Some(operation) = operation(code).filter(|operation| operation.accepted) else {
            return false;
        };
        if !operation.alone && elements.len() == 1 {
            return false;
        }
        let next = index + 1 + usize::from(operation.operands);
        if next > elements.len() {
            return false;
        }
        // A register operation says where the value is and upstream stops
        // reading there, so what follows it is not checked at all. What is
        // written after one that does not read as an operation is where
        // upstream crashes rather than answering, so there is no verdict to
        // copy and this stops too.
        if operation.ends_check {
            return true;
        }
        if operation.must_be_last && next != elements.len() {
            return false;
        }
        index = next;
    }
    true
}

/// The entry value, which none of the sweep's questions can express.
///
/// It says the value is the one the variable had when the function was
/// entered, and upstream will only have that of a single operation: the
/// operand is exactly one, whatever follows. It has to be the first operation,
/// or the one directly after a leading `DW_OP_LLVM_arg 0`; anywhere else is
/// refused, `DW_OP_deref, DW_OP_LLVM_entry_value, 1, DW_OP_deref` among them.
/// Like a register operation it then ends the checking, which is why a second
/// entry value further along is never asked about and may say anything at all.
fn entry_value_is_valid(elements: &[u64], index: usize) -> bool {
    let start = usize::from(elements.starts_with(&[ARG, 0])) * 2;
    if index != start {
        return false;
    }
    elements.get(index + 1) == Some(&1)
}

/// The elements of a `!DIExpression` as numbers, or `None` for anything
/// upstream would not have built one out of.
///
/// A word is read into its number as the node is built, so everything here is
/// a number by the time the verifier or the printer asks. An element that is
/// not one, a signed number included, is no expression at all: upstream reads
/// the list as unsigned and refuses the text where a sign is written.
pub fn elements(fields: &[MdField]) -> Option<Vec<u64>> {
    let mut elements = Vec::with_capacity(fields.len());
    for field in fields {
        match field {
            MdField::Unsigned(number) => elements.push(u64::try_from(*number).ok()?),
            _ => return None,
        }
    }
    Some(elements)
}

/// How upstream writes each element back: the word to write in place of the
/// number, or `None` where the element is written as it stands.
///
/// An expression upstream does not read is written out as numbers, which is
/// what `None` for the whole list says. Otherwise an element in opcode
/// position is written as its word, and as nothing at all where upstream has
/// no word for it, which is how `DW_OP_reg0, 1` comes back `DW_OP_reg0, `.
pub fn written_words(elements: &[u64]) -> Option<Vec<Option<&'static str>>> {
    if !is_valid(elements) {
        return None;
    }
    let mut written = vec![None; elements.len()];
    let mut index = 0;
    while index < elements.len() {
        let code = elements[index];
        written[index] = Some(word(code).unwrap_or(""));
        let operands = usize::from(operation(code).map_or(0, |operation| operation.operands));
        // The one operand upstream writes as a word of its own: what a
        // conversion converts to is an encoding, so `DW_OP_LLVM_convert, 8, 5`
        // comes back `DW_OP_LLVM_convert, 8, DW_ATE_signed`.
        if code == CONVERT && index + 2 < elements.len() {
            written[index + 2] =
                Some(dwarf::word(dwarf::ENCODING, elements[index + 2]).unwrap_or(""));
        }
        index += 1 + operands;
    }
    Some(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of these is a module `opt -S -passes=verify` was asked about.
    #[test]
    fn matches_what_upstream_verifies() {
        for (elements, valid) in [
            (&[6][..], true),
            (&[22][..], false),
            (&[22, 6][..], true),
            (&[16][..], false),
            (&[16, 0][..], true),
            (&[80][..], true),
            (&[80, 0, 0, 0][..], true),
            (&[144, 0, 0][..], false),
            (&[146, 0, 0][..], true),
            (&[159][..], true),
            (&[159, 6][..], false),
            (&[4096, 0, 8][..], true),
            (&[4096, 0, 8, 6][..], false),
            (&[0, 1, 9, 7, 2][..], false),
            (&[163, 1][..], false),
            (&[6, 999][..], false),
        ] {
            assert_eq!(is_valid(elements), valid, "{elements:?}");
        }
    }

    /// The entry value, one module each.
    #[test]
    fn matches_what_upstream_verifies_of_an_entry_value() {
        for (elements, valid) in [
            (&[4099, 1, 6][..], true),
            (&[4099, 1][..], true),
            (&[4099, 0][..], false),
            (&[4099, 2, 6, 6][..], false),
            (&[4099, 100, 16, 0][..], false),
            (&[4099, 4, 16, 0, 159][..], false),
            (&[16, 0, 4099, 1, 16, 0][..], false),
            (&[6, 4099, 1, 6][..], false),
            (&[4101, 0, 4099, 1, 6][..], true),
            (&[4101, 7, 4099, 1, 6][..], false),
            (&[4101, 0, 6, 4099, 1, 6][..], false),
            (&[4101, 0, 4101, 0, 4099, 1, 6][..], false),
            // Ends the checking, so the second is never asked about.
            (&[4099, 1, 6, 4099, 2, 6][..], true),
            (&[4099, 1, 999][..], true),
            (&[4099, 1, 159, 6][..], true),
        ] {
            assert_eq!(is_valid(elements), valid, "{elements:?}");
        }
    }

    /// A word and the number behind it are one element.
    #[test]
    fn reads_a_word_as_its_number() {
        assert_eq!(code_for_element_word("DW_OP_deref"), Some(6));
        assert_eq!(code_for_element_word("DW_OP_LLVM_entry_value"), Some(4099));
        assert_eq!(code_for_element_word("DW_ATE_signed"), Some(5));
        assert_eq!(code_for_element_word("DW_TAG_member"), None);
        assert_eq!(code_for_element_word("DW_OP_bogus"), None);
    }

    /// What upstream writes back, read off its own output.
    #[test]
    fn writes_what_upstream_writes() {
        assert_eq!(written_words(&[6]), Some(vec![Some("DW_OP_deref")]));
        assert_eq!(
            written_words(&[16, 3]),
            Some(vec![Some("DW_OP_constu"), None])
        );
        assert_eq!(written_words(&[6, 999]), None);
        assert_eq!(
            written_words(&[80, 1, 2, 3]),
            Some(vec![
                Some("DW_OP_reg0"),
                Some(""),
                Some(""),
                Some("DW_OP_addr"),
            ])
        );
        assert_eq!(
            written_words(&[4097, 8, 5]),
            Some(vec![
                Some("DW_OP_LLVM_convert"),
                None,
                Some("DW_ATE_signed"),
            ])
        );
        assert_eq!(
            written_words(&[4097, 0, 0]),
            Some(vec![Some("DW_OP_LLVM_convert"), None, Some("")])
        );
    }
}
