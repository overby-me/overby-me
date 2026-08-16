#!/usr/bin/env nu
# Derives which specialized-metadata fields a node cannot be written without.
#
#   nu md-required-fields.nu [upstream-opt]
#
# `!DICompileUnit` without a `file:` is "missing required field 'file'", and a
# parser that reads it anyway accepts a module upstream refuses. Which fields
# are required is in LangRef for some nodes and not for others, so it is
# measured: each node is written with a set of fields that parses, then once
# per field with that field left out, and a field whose absence upstream
# refuses is required.
#
# What comes out is the list of `(node, field)` pairs that are required.
# Compare it against the `required: true` marks in
# `crates/llvm-ir-parse/src/md_schema.rs`.

const PROBES = [
  [node, fields];

  ["DICompileUnit" [[field, value]; [language 'DW_LANG_C99'] [file '!10'] [producer '"p"'] [isOptimized 'false'] [runtimeVersion '0'] [emissionKind 'FullDebug']]]
  ["DIFile" [[field, value]; [filename '"f"'] [directory '"d"']]]
  ["DIBasicType" [[field, value]; [name '"n"'] [size '8'] [encoding 'DW_ATE_signed']]]
  ["DIDerivedType" [[field, value]; [tag 'DW_TAG_member'] [baseType '!11'] [size '8']]]
  ["DICompositeType" [[field, value]; [tag 'DW_TAG_structure_type'] [name '"c"'] [size '8']]]
  ["DISubroutineType" [[field, value]; [types '!13']]]
  ["DISubrange" [[field, value]; [count '2']]]
  ["DIEnumerator" [[field, value]; [name '"e"'] [value '1']]]
  ["DITemplateTypeParameter" [[field, value]; [name '"T"'] [type '!11']]]
  ["DITemplateValueParameter" [[field, value]; [name '"V"'] [type '!11'] [value 'i32 1']]]
  ["DINamespace" [[field, value]; [name '"n"'] [scope 'null']]]
  ["DIModule" [[field, value]; [scope 'null'] [name '"M"']]]
  ["DILocalVariable" [[field, value]; [name '"v"'] [scope '!9'] [line '1']]]
  ["DILabel" [[field, value]; [scope '!9'] [name '"l"'] [file '!10'] [line '1']]]
  ["DILocation" [[field, value]; [line '1'] [column '1'] [scope '!9']]]
  ["DIObjCProperty" [[field, value]; [name '"p"'] [setter '"s"'] [getter '"g"']]]
  ["DIImportedEntity" [[field, value]; [tag 'DW_TAG_imported_module'] [scope 'null'] [entity '!11']]]
  ["DIMacro" [[field, value]; [type 'DW_MACINFO_define'] [name '"m"'] [value '"v"']]]
  ["DIMacroFile" [[field, value]; [file '!10'] [nodes '!13']]]
  ["DIGlobalVariableExpression" [[field, value]; [var '!15'] [expr '!DIExpression()']]]
  ["DIStringType" [[field, value]; [name '"s"'] [size '8']]]
]

def probe-text [node: string, body: string]: nothing -> string {
  let distinct = if $node in ["DICompileUnit" "DIAssignID" "DIGlobalVariable"] { "distinct " } else { "" }
  let units = if $node == "DICompileUnit" { '!llvm.dbg.cu = !{!12, !0}' } else { '!llvm.dbg.cu = !{!12}' }
  ([
    '!named = !{!0}'
    $"!0 = ($distinct)!($node)\(($body)\)"
    '!9 = distinct !DISubprogram(name: "s", scope: !10, file: !10, line: 1, type: !11, spFlags: DISPFlagDefinition, unit: !12)'
    '!10 = !DIFile(filename: "a", directory: "d")'
    '!11 = !DIBasicType(name: "int", size: 32, encoding: DW_ATE_signed)'
    '!12 = distinct !DICompileUnit(language: DW_LANG_C99, file: !10, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)'
    '!13 = !{null}'
    '!15 = distinct !DIGlobalVariable(name: "g", scope: null, type: !11, isLocal: false, isDefinition: true)'
    $units
    '!llvm.module.flags = !{!14}'
    '!14 = !{i32 2, !"Debug Info Version", i32 3}'
  ] | str join "\n")
}

def main [upstream_opt: path = "opt"] {
  let work = (mktemp -d)
  let source = ([$work "probe.ll"] | path join)
  mut required = []
  for probe in $PROBES {
    let whole = ($probe.fields | each {|f| $"($f.field): ($f.value)"} | str join ", ")
    probe-text $probe.node $whole | save -f $source
    let baseline = (do { ^$upstream_opt -S $source -o - } | complete)
    if $baseline.exit_code != 0 {
      print $"($probe.node): the whole probe is refused, so nothing can be told from it"
      print ($baseline.stderr | lines | where ($it | str contains "error") | first)
      continue
    }
    for entry in $probe.fields {
      let without = (
        $probe.fields
        | where field != $entry.field
        | each {|f| $"($f.field): ($f.value)"}
        | str join ", "
      )
      probe-text $probe.node $without | save -f $source
      let attempt = (do { ^$upstream_opt -S $source -o - } | complete)
      if $attempt.exit_code != 0 {
        $required = ($required | append $"($probe.node).($entry.field)")
        print $"($probe.node).($entry.field): REQUIRED"
      } else {
        print $"($probe.node).($entry.field): optional"
      }
    }
  }
  rm -rf $work
  print ""
  print "required:"
  print ($required | str join "\n")
}
