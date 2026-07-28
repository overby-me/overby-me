#!/usr/bin/env nu
# Derives the DWARF vocabulary by asking upstream what each number is called.
#
#   nu dwarf-vocabulary.nu [llvm-as] [llvm-dis] [out.rs]
#
# A field like `encoding:` takes a word, and a module may write the number
# behind it instead: `encoding: 5` and `encoding: DW_ATE_signed` are the same
# field. Upstream prints the word back, so a printer that keeps the number
# diverges, and a verifier with no list of words cannot say that
# `DW_TAG_badtag` is not one.
#
# The list has been recorded as unobtainable since the sixty-third pass,
# because no specification this project may read enumerates every `DW_TAG_*`.
# But the oracle does: writing `tag: 5` and reading back `DW_TAG_variable` is
# a question upstream answers, the same way every other rule here is asked.
# So the vocabulary is swept out rather than looked up.
#
# The sweep covers each field's whole range rather than a sample, which is
# what makes the tables an answer to "is this a word" as well as to "what is
# this number called": a word none of them has is one upstream does not know.
# Getting there needs the values asked in batches, one module holding a node
# per value, because sixty-five thousand separate runs is an hour and one
# module is a second.
#
# It reads back through `llvm-as | llvm-dis` rather than `opt -S`, because
# some values are legal to write and refused by the verifier, and a verifier
# complaint would take the whole batch with it.

# Each field, where it is written, the skeleton the node needs, and the range
# of values worth asking about. `tag` is per-node, each kind taking the tags
# that make sense for it, so it is swept on the kind that takes the most and
# the answers are the union.
const FIELDS = [
  [name, node, skeleton, first, last];

  ["tag" "GenericDINode" 'header: "h"' 0 65536]
  ["encoding" "DIBasicType" 'name: "n", size: 8' 0 256]
  ["language" "DICompileUnit" 'file: !2' 0 65536]
  ["emissionKind" "DICompileUnit" 'language: DW_LANG_C99, file: !2' 0 16]
  ["nameTableKind" "DICompileUnit" 'language: DW_LANG_C99, file: !2' 0 16]
  ["virtuality" "DISubprogram" 'name: "s", scope: null, type: null, spFlags: 0' 0 16]
  ["cc" "DISubroutineType" 'types: null' 0 256]
  ["checksumkind" "DIFile" 'filename: "f", directory: "d", checksum: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"' 0 16]
  ["type" "DIMacro" 'name: "m", value: "v"' 0 16]
]

# One module holding a node per value, so a whole range is one question.
def batch-text [field: string, node: string, skeleton: string, values: list<int>]: nothing -> string {
  let distinct = if $node in ["DICompileUnit" "DISubprogram"] { "distinct " } else { "" }
  let nodes = ($values | enumerate | each {|pair|
    $"!($pair.index + 100) = ($distinct)!($node)\(($skeleton), ($field): ($pair.item)\)"
  })
  let names = ($values | enumerate | each {|pair| $"!($pair.index + 100)" } | str join ", ")
  let units = if $node == "DICompileUnit" {
    # A compile unit has to be listed, and every one of these is a unit.
    $"!llvm.dbg.cu = !{($names)}"
  } else { '' }
  ([
    $"!named = !{($names)}"
    ...$nodes
    '!2 = !DIFile(filename: "a", directory: "d")'
    $units
    '!llvm.module.flags = !{!9}'
    '!9 = !{i32 2, !"Debug Info Version", i32 3}'
  ] | where $it != "" | str join "\n")
}

# The words a batch comes back with, in the order the values went in, or
# nothing at all when upstream refuses the batch.
def ask [as: path, dis: path, source: path, text: string]: nothing -> any {
  $text | save -f $source
  let assembled = (do { ^$as $source -o - } | complete)
  if $assembled.exit_code != 0 { return null }
  let printed = (do { $assembled.stdout | ^$dis - -o - } | complete)
  if $printed.exit_code != 0 { return null }
  let text = (try { $printed.stdout | decode utf-8 } catch { $printed.stdout })
  let lines = ($text | lines)
  # The named list gives the printed number of each node in the order they
  # were written, which is what maps an answer back to the value it came from.
  let order = (
    $lines
    | where ($it | str starts-with "!named = ")
    | get 0?
    | default ""
    | parse --regex '\{(?P<body>.*)\}'
    | get body?
    | default []
  )
  if ($order | is-empty) { return null }
  let ids = ($order | first | split row ", ")
  let by_id = (
    $lines
    | where ($it | str starts-with "!") and ($it | str contains " = ")
    | reduce --fold {} {|line, acc|
      let id = ($line | split row " = " | first)
      $acc | upsert $id $line
    }
  )
  $ids | each {|id| $by_id | get -o $id | default "" }
}

def main [
  llvm_as: path = "llvm-as"
  llvm_dis: path = "llvm-dis"
  out: path = "dwarf.rs"
] {
  let work = (mktemp -d)
  let source = ([$work "probe.ll"] | path join)
  mut tables = []
  for field in $FIELDS {
    mut pairs = []
    # Four thousand at a time: large enough that the whole tag range is
    # sixteen questions, small enough that one refused value costs little.
    let values = (($field.first)..<($field.last) | each {|v| $v} | chunks 4096)
    for chunk in $values {
      mut answers = (ask $llvm_as $llvm_dis $source (batch-text $field.name $field.node $field.skeleton $chunk))
      if $answers == null {
        # Something in the chunk is not a value this field takes, so it is
        # asked one at a time and the refusals are the ones with no word.
        $answers = ($chunk | each {|value|
          let one = (ask $llvm_as $llvm_dis $source (batch-text $field.name $field.node $field.skeleton [$value]))
          if $one == null { "" } else { $one | first }
        })
      }
      for pair in ($chunk | zip $answers) {
        let word = (
          $pair.1
          | parse --regex $"($field.name): \(?P<word>[A-Za-z_][A-Za-z0-9_]*\)"
          | get word?
          | default []
        )
        if ($word | is-empty) { continue }
        $pairs = ($pairs | append {value: $pair.0, word: ($word | first)})
      }
    }
    print $"($field.name) via ($field.node): ($pairs | length) words"
    $tables = ($tables | append {field: $field.name, pairs: $pairs})
  }
  rm -rf $work

  # A vocabulary swept on more than one node kind is the union: each kind
  # takes the tags that make sense for it, and the words are the same words.
  let tables = (
    $tables
    | group-by field
    | transpose field rows
    | each {|group|
      {
        field: $group.field
        pairs: ($group.rows | get pairs | flatten | uniq-by value | sort-by value)
      }
    }
  )
  let body = (
    $tables
    | each {|table|
      let rows = (
        $table.pairs
        | each {|pair| $"    \(($pair.value), \"($pair.word)\"\)," }
        | str join "\n"
      )
      $"/// The words `($table.field):` takes, and the number behind each.\npub static ($table.field | str upcase): &[\(u64, &str\)] = &[\n($rows)\n];\n"
    }
    | str join "\n"
  )
  let header = "//! The DWARF vocabulary, as upstream spells it.
//!
//! Generated by `corpus/dwarf-vocabulary.nu`, which explains the derivation.
//! In short: a field that takes a word also takes the number behind it, and
//! upstream prints the word back, so the map from one to the other is a
//! question the assembler answers rather than a list to be copied from a
//! specification this project may not read.
//!
//! Each field is swept over its whole range rather than a sample, so a word
//! none of these tables has is a word upstream does not know. That makes them
//! the answer to \"is this a word\" as well as to \"what is this number
//! called\".

/// The word a number is spelled as, in the vocabulary a field takes.
pub fn word(vocabulary: &[(u64, &'static str)], value: u64) -> Option<&'static str> {
    vocabulary
        .iter()
        .find(|(number, _)| *number == value)
        .map(|(_, word)| *word)
}

/// Whether a word is one this vocabulary has.
pub fn has(vocabulary: &[(u64, &'static str)], word: &str) -> bool {
    vocabulary.iter().any(|(_, known)| *known == word)
}
"
  $"($header)\n($body)" | save -f $out
  print $"wrote ($out)"
}
