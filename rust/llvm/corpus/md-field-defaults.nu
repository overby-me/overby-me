#!/usr/bin/env nu
# Derives which specialized-metadata fields upstream drops at their default.
#
#   nu md-field-defaults.nu [upstream-opt]
#
# A field written with the value it would have had anyway is not written back:
# `!DIBasicType(tag: DW_TAG_base_type, name: "n")` comes back without the tag,
# and `!DIBasicType(name: "n", align: 0)` without the align. That matters for
# more than tidiness, because two nodes that differ only in such a field are
# the same node once it is gone, and upstream uniques them: a `!named` list
# holding both comes back naming one twice.
#
# Which fields do this is not a rule with a shape. `!DIEnumerator`'s
# `isUnsigned: false` goes and `!DIGlobalVariable`'s `isLocal: false` stays;
# `!DIDerivedType`'s `offset: 0` goes and `!DISubrange`'s `lowerBound: 0`
# stays, that one being a metadata operand where a bare number is a constant
# rather than an absence. So it is measured, one field at a time, by writing
# the field at its default and seeing whether it survives.
#
# What comes out is the list of `(node, field)` pairs that drop. Everything
# not listed is written as given.

# The nodes to probe, each with the fields it needs to be legal at all and the
# optional fields to test. A field is tested by adding it at `default` to the
# legal skeleton.
const PROBES = [
  [node, skeleton, fields];

  ["DICompileUnit" 'language: DW_LANG_C99, file: !10, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug' [[field, default]; [splitDebugFilename '""'] [dwoId '0'] [splitDebugInlining 'false'] [debugInfoForProfiling 'false'] [rangesBaseAddress 'false'] [sysroot '""'] [sdk '""'] [enums 'null'] [retainedTypes 'null'] [globals 'null'] [imports 'null'] [macros 'null']]]
  ["DIBasicType" 'name: "n"' [[field, default]; [tag 'DW_TAG_base_type'] [size '0'] [align '0'] [encoding '0'] [flags '0'] [num_extra_inhabitants '0'] [name '""']]]
  ["DIDerivedType" 'tag: DW_TAG_member, baseType: null' [[field, default]; [name '""'] [line '0'] [size '0'] [align '0'] [offset '0'] [flags '0'] [dwarfAddressSpace '0'] [baseType 'null'] [scope 'null'] [file 'null'] [extraData 'null'] [annotations 'null']]]
  ["DICompositeType" 'tag: DW_TAG_structure_type' [[field, default]; [name '""'] [line '0'] [size '0'] [align '0'] [offset '0'] [flags '0'] [runtimeLang '0'] [identifier '""'] [scope 'null'] [file 'null'] [baseType 'null'] [elements 'null'] [vtableHolder 'null'] [templateParams 'null'] [annotations 'null']]]
  ["DISubroutineType" 'types: null' [[field, default]; [flags '0'] [cc '0'] [types 'null']]]
  ["DIFile" 'filename: "f", directory: "d"' [[field, default]; [source '""'] [checksum 'null']]]
  ["DISubprogram" 'name: "n", scope: null, type: null, spFlags: DISPFlagDefinition, unit: !12' [[field, default]; [linkageName '""'] [line '0'] [scopeLine '0'] [flags '0'] [virtualIndex '0'] [thisAdjustment '0'] [name '""'] [scope 'null'] [type 'null'] [file 'null'] [containingType 'null'] [templateParams 'null'] [declaration 'null'] [retainedNodes 'null'] [thrownTypes 'null'] [annotations 'null']]]
  ["DILexicalBlock" 'scope: !9' [[field, default]; [line '0'] [column '0'] [file 'null']]]
  ["DILocation" 'line: 1, scope: !9' [[field, default]; [column '0'] [isImplicitCode 'false'] [inlinedAt 'null']]]
  ["DILocalVariable" 'name: "v", scope: !9' [[field, default]; [line '0'] [arg '0'] [flags '0'] [align '0'] [name '""'] [file 'null'] [type 'null'] [annotations 'null']]]
  ["DIGlobalVariable" 'name: "g", scope: null, isLocal: false, isDefinition: false' [[field, default]; [linkageName '""'] [line '0'] [align '0'] [name '""'] [scope 'null'] [isLocal 'false'] [isDefinition 'false'] [file 'null'] [type 'null'] [declaration 'null'] [templateParams 'null'] [annotations 'null']]]
  ["DIEnumerator" 'name: "e", value: 1' [[field, default]; [isUnsigned 'false']]]
  ["DINamespace" 'name: "n", scope: null' [[field, default]; [exportSymbols 'false'] [name '""'] [scope 'null']]]
  ["DIModule" 'scope: null, name: "M"' [[field, default]; [configMacros '""'] [includePath '""'] [apinotes '""'] [line '0'] [isDecl 'false'] [scope 'null'] [name '""'] [file 'null']]]
  ["DISubrange" 'count: 1' [[field, default]; [lowerBound '0'] [upperBound '0'] [stride '0']]]
  ["DITemplateTypeParameter" 'name: "T", type: null' [[field, default]; [defaulted 'false'] [name '""'] [type 'null']]]
  ["DITemplateValueParameter" 'name: "V", type: null, value: null' [[field, default]; [defaulted 'false'] [name '""'] [type 'null'] [value 'null']]]
  ["DIObjCProperty" 'name: "p"' [[field, default]; [line '0'] [setter '""'] [getter '""'] [attributes '0'] [name '""'] [file 'null'] [type 'null']]]
  ["DIImportedEntity" 'tag: DW_TAG_imported_module, scope: null' [[field, default]; [name '""'] [line '0'] [scope 'null'] [entity 'null'] [file 'null'] [elements 'null']]]
  ["DILabel" 'scope: !9, name: "l", file: null, line: 1' [[field, default]; [column '0'] [column '0'] [file 'null']]]
  ["DIMacro" 'type: DW_MACINFO_define, name: "m"' [[field, default]; [line '0'] [value '""']]]
  ["DIMacroFile" 'file: null' [[field, default]; [line '0'] [nodes 'null']]]
]

# A module holding one node, plus the scope a function-local node needs.
def probe-text [node: string, body: string]: nothing -> string {
  # A few kinds have to be written `distinct`, and a compile unit has to be
  # listed in `llvm.dbg.cu` besides.
  let distinct = if $node in ["DICompileUnit" "DISubprogram" "DILexicalBlock" "DIGlobalVariable"] { "distinct " } else { "" }
  let units = if $node == "DICompileUnit" { '!llvm.dbg.cu = !{!12, !0}' } else { '!llvm.dbg.cu = !{!12}' }
  ([
    '!named = !{!0}'
    $"!0 = ($distinct)!($node)\(($body)\)"
    '!9 = distinct !DISubprogram(name: "s", scope: !10, file: !10, line: 1, type: !11, spFlags: DISPFlagDefinition, unit: !12)'
    '!10 = !DIFile(filename: "a", directory: "d")'
    '!11 = !DISubroutineType(types: !13)'
    '!12 = distinct !DICompileUnit(language: DW_LANG_C99, file: !10, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)'
    '!13 = !{null}'
    $units
    '!llvm.module.flags = !{!14}'
    '!14 = !{i32 2, !"Debug Info Version", i32 3}'
  ] | str join "\n")
}

def main [upstream_opt: path = "opt"] {
  let work = (mktemp -d)
  let source = ([$work "probe.ll"] | path join)
  mut dropped = []
  for probe in $PROBES {
    for entry in $probe.fields {
      # The skeleton may already set this field; the probe replaces it
      # rather than writing it twice, which upstream refuses.
      let kept = (
        $probe.skeleton
        | split row ", "
        | where not ($it | str starts-with $"($entry.field): ")
        | str join ", "
      )
      let body = if ($kept | is-empty) {
        $"($entry.field): ($entry.default)"
      } else {
        $"($kept), ($entry.field): ($entry.default)"
      }
      probe-text $probe.node $body | save -f $source
      let printed = (do { ^$upstream_opt -S $source -o - } | complete)
      if $printed.exit_code != 0 {
        print $"($probe.node).($entry.field): upstream refused the probe"
        continue
      }
      let line = ($printed.stdout | lines | where ($it | str starts-with "!0 = ") | first)
      let kept = ($line | str contains $"($entry.field):")
      if not $kept {
        $dropped = ($dropped | append $"($probe.node).($entry.field)")
      }
      print $"($probe.node).($entry.field): (if $kept { 'kept' } else { 'DROPPED' })"
    }
  }
  rm -rf $work
  print ""
  print "dropped at their default:"
  print ($dropped | str join "\n")
}
