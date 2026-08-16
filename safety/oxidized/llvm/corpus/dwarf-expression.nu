#!/usr/bin/env nu
# Generates what a `!DIExpression` may hold.
#
#   nu dwarf-expression.nu <opt> <out.rs> [<llvm-test-dir>]
#
# A standalone expression reached from a named list is verified, so
# `opt -S -passes=verify` on a two-line module is the whole oracle and every
# question is one module. An expression is a flat list of numbers: an opcode,
# its operands, the next opcode, and so on.
#
# This was recorded once as measured-and-not-a-table: three probe designs had
# each measured something other than arity, and the pair that settled it was
# `DIExpression(DW_OP_swap)` refused against `DIExpression(DW_OP_swap,
# DW_OP_deref)` accepted, which no arity explains. The conclusion drawn from
# that was wrong. It is a table, and what no arity explains is one rule beside
# it: `DW_OP_swap` may not be the only element. Asking every opcode the same
# questions is what shows that, where asking one opcode many questions did not.
#
# Each opcode is asked two kinds of question.
#
# The first is what upstream calls it and how many elements it swallows, and
# both come out of one module. Upstream writes an opcode as a `DW_OP_*` word
# only for an expression it finds valid, and a register operation ends the
# checking, so `DW_OP_reg0` in front of any code at all makes it answer: the
# word comes back where upstream has one, and the `DW_OP_addr`s written after
# it come back as the number three where the opcode takes them as operands and
# as their own word where it does not. That reaches every code upstream can
# write, including the ones it refuses to verify, which the printer needs.
#
# The second is validity, three modules given the arity the first found:
#
#   * `(N)`, or `(N, <zeros>)`. Accepted means upstream reads it as an
#     operation at all, and for an opcode taking nothing it also says whether
#     it may stand alone.
#   * `(N, <zeros>, DW_OP_deref)`. Refused means it has to be last.
#   * `(N, <zeros + three>)`. Accepted means nothing after it is checked.
#
# Two opcodes answer in a way none of that covers, and both are recorded as
# their own rule rather than bent into the table. `DW_OP_reg0` through
# `DW_OP_breg31` accept anything at all after them, three trailing zeros
# included, so a register operation ends the checking; `DW_OP_regx` and
# `DW_OP_bregx` do not, which is why the rule is a range rather than a notion.
# And `DW_OP_swap` is the one that may not stand alone.
#
# `DW_OP_LLVM_entry_value` is a third, and its rule is written in
# `crates/llvm-ir/src/metadata/expression.rs` beside the walk, because none of
# these questions can express it: its operand has to be exactly one, and it
# has to be the first operation or the one directly after a leading
# `DW_OP_LLVM_arg 0`. Every shape here is refused, so the table records it as
# an operation upstream does not read and the rule beside it says when it does.

const DEREF = 6
const REG0 = 80
const MARKER = 3
const MARKER_WORD = "DW_OP_addr"

def accepts [opt: path, work: path, body: string]: nothing -> bool {
  let source = ([$work "e.ll"] | path join)
  $"!named = !{!0}\n!0 = !DIExpression\(($body)\)\n" | save --force $source
  let run = (do { ^$opt -S -passes=verify $source -o /dev/null } | complete)
  $run.exit_code == 0
}

# What upstream calls an opcode and how many elements it takes as operands,
# read off its own output behind a register operation.
#
# The filler written after it is `DW_OP_addr`, which takes nothing and has a
# word: an element upstream reads as an operand comes back as the number, and
# one it reads as the next opcode comes back as the word, so the first word in
# the run is where the operands end. Filling with nought would not tell the
# two apart, nought being a number upstream has no word for either way, and
# that is what made `DW_OP_LLVM_convert` look like it takes one operand.
#
# The operands come back as well as their count, because upstream writes some
# of them as words of their own.
def readback [opt: path, work: path, code: string]: nothing -> record {
  let source = ([$work "r.ll"] | path join)
  let filler = (1..5 | each {|_| $"($MARKER)"} | str join ", ")
  $"!named = !{!0}\n!0 = !DIExpression\(($REG0), ($code), ($filler)\)\n" | save --force $source
  let run = (do { ^$opt -S -disable-verify $source -o - } | complete)
  if $run.exit_code != 0 {
    return {word: "", operands: 0, rendered: [], read: false}
  }
  let found = ($run.stdout | parse --regex 'DIExpression\((?P<body>[^)]*)\)' | get body)
  if ($found | is-empty) {
    return {word: "", operands: 0, rendered: [], read: false}
  }
  let parts = ($found | first | split row "," | each {|part| $part | str trim})
  let rendered = ($parts | skip 2 | take while {|part| $part != $MARKER_WORD})
  if ($rendered | length) == ($parts | length) - 2 {
    error make {msg: $"($code) swallowed every element written after it"}
  }
  {word: ($parts | get 1), operands: ($rendered | length), rendered: $rendered, read: true}
}

def zeros [count: int]: nothing -> string {
  if $count == 0 { return "" }
  (1..$count | each {|_| "0"} | str join ", ")
}

def main [opt: path, out: path, tests?: path] {
  let work = (mktemp -d)
  # The DWARF range, and the block LLVM adds above it for its own operations.
  let codes = ((0..255 | each {|c| $c}) ++ (4096..4200 | each {|c| $c}))
  mut rows = []
  for code in $codes {
    let read = (readback $opt $work $"($code)")
    if not $read.read {
      print $"($code): no readback, upstream did not answer"
      continue
    }
    let operands = $read.operands
    let body = (if $operands == 0 { $"($code)" } else { $"($code), (zeros $operands)" })
    let alone = ($operands == 0) and (accepts $opt $work $body)
    let accepted = (if $operands == 0 {
      $alone or (accepts $opt $work $"($code), ($DEREF)")
    } else {
      accepts $opt $work $body
    })
    if not $accepted and ($read.word | is-empty) {
      continue
    }
    # Anything after it, three zeros deep, is a register operation ending the
    # check rather than an opcode with a great many operands.
    let ends_check = $accepted and (accepts $opt $work $"($code), (zeros ($operands + 3))")
    let must_be_last = $accepted and (not $ends_check) and (not (accepts $opt $work $"($body), ($DEREF)"))
    # The arity came off the printer; where upstream reads the opcode at all,
    # the verifier has to agree about it, or one of the two was misread.
    if $accepted and $operands > 0 and (accepts $opt $work $"($code), (zeros ($operands - 1))") {
      error make {msg: $"($code) verifies with fewer operands than it prints"}
    }
    $rows = ($rows | append {
      code: $code
      word: $read.word
      operands: $operands
      rendered: $read.rendered
      accepted: $accepted
      alone: $alone
      ends_check: $ends_check
      must_be_last: $must_be_last
    })
  }

  # An operand upstream writes as a word of its own rather than as the number
  # it was given. Reported rather than tabled: it is a rule about one opcode
  # and it lives beside the walk.
  let worded = ($rows | where {|row| ($row.rendered | any {|part| $part != $"($MARKER)"})})
  for row in $worded {
    print $"($row.word) writes its operands as ($row.rendered | str join ', ')"
  }

  # A word stands for one code and a code for one word, or neither direction
  # is a lookup. The forward half is read back from upstream's own parser.
  let spelled = ($rows | where {|row| not ($row.word | is-empty)})
  let clashes = ($spelled | group-by word | items {|word, group| {word: $word, count: ($group | length)}} | where count > 1)
  if not ($clashes | is-empty) {
    error make {msg: $"two codes share a word: ($clashes | to json)"}
  }
  for row in $spelled {
    let back = (readback $opt $work $row.word)
    if (not $back.read) or $back.word != $row.word or $back.operands != $row.operands {
      error make {msg: $"($row.word) does not read back as itself: ($back | to json)"}
    }
  }

  # Every word upstream's own tests write is one this knows, or the sweep did
  # not reach far enough. Most of what the harvest turns up is prose in a
  # comment or half a word out of a FileCheck line, so each one it does not
  # know is put to upstream: a word upstream refuses is not a word.
  if $tests != null {
    let written = (
      ^grep -rhoE 'DW_OP_[A-Za-z0-9_]+' $tests
      | lines
      | uniq
      | where {|word| ($spelled | where word == $word | is-empty)}
      | where {|word| (readback $opt $work $word).read}
    )
    if not ($written | is-empty) {
      error make {msg: $"upstream reads words this does not know: ($written | str join ' ')"}
    }
    print "every DW_OP_ word upstream reads in its own tests is in the table"
  }

  rm -rf $work
  print $"($rows | length) codes upstream writes or reads"
  print $"($rows | where accepted | length) it reads as an operation"
  print $"($rows | where ends_check | length) end the checking"
  print $"($rows | where must_be_last | length) have to be last"
  print $"($rows | where {|r| $r.accepted and not $r.alone and $r.operands == 0} | length) may not stand alone"

  let body = (
    $rows
    | sort-by code
    | each {|row|
      $"    \(($row.code), \"($row.word)\", ($row.operands), ($row.accepted), ($row.alone), ($row.ends_check), ($row.must_be_last)\),"
    }
    | str join "\n"
  )
  let header = "//! What a `!DIExpression` may hold.
//!
//! Generated by `corpus/dwarf-expression.nu`, which explains the derivation
//! and the questions each opcode was asked. The walk that reads this table,
//! and the one rule no column of it can express, are in the module above.

/// One opcode: its number, the `DW_OP_*` word upstream writes it as, how many
/// elements follow it as operands, whether upstream reads it as an operation
/// at all, whether it may be the only element, whether nothing after it is
/// checked, and whether it has to be last.
///
/// A code upstream has a word for but reads as no operation is here too, with
/// `accepted` false: the printer needs its word and its arity to write back
/// what follows a register operation.
type Entry = (u64, &'static str, u8, bool, bool, bool, bool);

/// What upstream knows about this opcode, or `None` when it knows none.
pub fn operation(code: u64) -> Option<Operation> {
    let index = row(code)?;
    let (_, _, operands, accepted, alone, ends_check, must_be_last) = OPERATIONS[index];
    Some(Operation { operands, accepted, alone, ends_check, must_be_last })
}

/// The `DW_OP_*` word upstream writes this opcode as, or `None` when it
/// writes the number instead.
pub fn word(code: u64) -> Option<&'static str> {
    let spelling = OPERATIONS[row(code)?].1;
    (!spelling.is_empty()).then_some(spelling)
}

/// The number behind a `DW_OP_*` word. An expression may be written either
/// way and the two are one element, so a word is read into its number as the
/// node is built.
pub fn code_for_word(word: &str) -> Option<u64> {
    OPERATIONS
        .iter()
        .find(|(_, spelling, _, _, _, _, _)| !spelling.is_empty() && *spelling == word)
        .map(|(code, _, _, _, _, _, _)| *code)
}

fn row(code: u64) -> Option<usize> {
    OPERATIONS.binary_search_by_key(&code, |(code, _, _, _, _, _, _)| *code).ok()
}

/// What one opcode is allowed to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Operation {
    pub operands: u8,
    pub accepted: bool,
    pub alone: bool,
    pub ends_check: bool,
    pub must_be_last: bool,
}

/// Sorted by opcode, so the lookup can be a binary search.
static OPERATIONS: &[Entry] = &["

  [$header, $body, "];", ""] | str join "\n" | save --force $out
  ^rustfmt --edition 2021 $out
  print $"written to ($out)"
}
