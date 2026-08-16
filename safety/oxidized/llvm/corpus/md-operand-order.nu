#!/usr/bin/env nu
# Derives the order upstream numbers a specialized metadata node's references
# in.
#
#   nu md-operand-order.nu [llvm-as] [llvm-dis]
#
# A specialized node holds its operands in a fixed order that the printer does
# not follow. `DISubprogram` writes `scope` before `file` and stores `file`
# first, so a subprogram whose file and scope are both new gives the file the
# lower number. A printer that numbered in written order would swap the two,
# and every node numbered after them.
#
# That order is not in LangRef, which documents the syntax rather than the
# layout, and this project does not read upstream's C++. So it is measured:
# one probe module per node kind, every reference field pointing at a node
# with a name no other node has, run through `llvm-as | llvm-dis`, and the
# numbering that comes back read off in order.
#
# What it prints is the observed order for each kind, next to the order the
# fields were written in. A kind whose two orders agree needs no entry in
# `crates/llvm-ir-print/src/md_slots.rs`; a kind whose orders differ needs
# one, and the printed order is what it should say.
#
# The probes below are the ones that pin every reference field of the kinds
# real debug info uses. A field that upstream refuses in isolation (a
# `dataLocation` outside an array, a `discriminator` outside a variant part)
# gets its own probe, which is why some kinds appear more than once.

# One probe: a name for the report, the node under test as `!0`, and the
# supporting nodes it needs. Every node worth tracking carries a SHOUTED name
# so it can be picked out of the output.
const PROBES = [
  [name, cu, body];

  ["DISubprogram" "!9" '!0 = distinct !DISubprogram(name: "SELF", scope: !1, file: !2, line: 1, type: !3, scopeLine: 1, spFlags: DISPFlagDefinition, unit: !9, templateParams: !4, retainedNodes: !5, declaration: !6, containingType: !7, annotations: !8)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DISubroutineType(types: !10)
!4 = !{!11}
!5 = !{!12}
!6 = !DISubprogram(name: "DECLARATION", scope: !1, file: !2, line: 1, type: !3, spFlags: DISPFlagOptimized)
!7 = !DICompositeType(tag: DW_TAG_structure_type, name: "CONTAININGTYPE", size: 8)
!8 = !{!13}
!9 = distinct !DICompileUnit(language: DW_LANG_C99, file: !20, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!10 = !{null}
!11 = !DITemplateTypeParameter(name: "TEMPLATEPARAMS", type: !14)
!12 = !DILocalVariable(name: "RETAINEDNODES", scope: !0, type: !15)
!13 = !{!"ANNOTATIONS"}
!14 = !DIBasicType(name: "TEMPLATEPARAMSTYPE", size: 8, encoding: DW_ATE_signed)
!15 = !DIBasicType(name: "RETAINEDNODESTYPE", size: 8, encoding: DW_ATE_signed)
!20 = !DIFile(filename: "CUFILE", directory: "d")']

  ["DICompositeType" "" '!0 = !DICompositeType(tag: DW_TAG_structure_type, name: "SELF", scope: !1, file: !2, line: 1, size: 64, baseType: !3, elements: !4, templateParams: !5, vtableHolder: !6, annotations: !7, specification: !8)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DIBasicType(name: "BASETYPE", size: 8, encoding: DW_ATE_signed)
!4 = !{!9}
!5 = !{!10}
!6 = !DICompositeType(tag: DW_TAG_structure_type, name: "VTABLEHOLDER", size: 8)
!7 = !{!11}
!8 = !DICompositeType(tag: DW_TAG_structure_type, name: "SPECIFICATION", size: 8)
!9 = !DIDerivedType(tag: DW_TAG_member, name: "ELEMENTS", baseType: !3, size: 8)
!10 = !DITemplateTypeParameter(name: "TEMPLATEPARAMS", type: !3)
!11 = !{!"ANNOTATIONS"}']

  ["DICompositeType (array)" "" '!0 = !DICompositeType(tag: DW_TAG_array_type, name: "SELF", scope: !1, file: !2, line: 1, size: 64, baseType: !3, elements: !4, dataLocation: !5, associated: !6, allocated: !7, rank: !8, annotations: !9)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DIBasicType(name: "BASETYPE", size: 8, encoding: DW_ATE_signed)
!4 = !{!10}
!5 = !DIGlobalVariable(name: "DATALOCATION", scope: null, type: !3, isLocal: false, isDefinition: true)
!6 = !DIGlobalVariable(name: "ASSOCIATED", scope: null, type: !3, isLocal: false, isDefinition: true)
!7 = !DIGlobalVariable(name: "ALLOCATED", scope: null, type: !3, isLocal: false, isDefinition: true)
!8 = !DIGlobalVariable(name: "RANK", scope: null, type: !3, isLocal: false, isDefinition: true)
!9 = !{!11}
!10 = !DISubrange(count: 2)
!11 = !{!"ANNOTATIONS"}']

  ["DICompositeType (variant)" "" '!0 = !DICompositeType(tag: DW_TAG_variant_part, name: "SELF", scope: !1, file: !2, size: 64, baseType: !3, elements: !4, discriminator: !5, annotations: !6, templateParams: !7)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DIBasicType(name: "BASETYPE", size: 8, encoding: DW_ATE_signed)
!4 = !{!8}
!5 = !DIDerivedType(tag: DW_TAG_member, name: "DISCRIMINATOR", baseType: !3, size: 8)
!6 = !{!9}
!7 = !{!10}
!8 = !DIDerivedType(tag: DW_TAG_member, name: "ELEMENTS", baseType: !3, size: 8)
!9 = !{!"ANNOTATIONS"}
!10 = !DITemplateTypeParameter(name: "TEMPLATEPARAMS", type: !3)']

  ["DIDerivedType" "" '!0 = !DIDerivedType(tag: DW_TAG_member, name: "SELF", scope: !1, file: !2, line: 1, baseType: !3, size: 8, extraData: !4, annotations: !5)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DIBasicType(name: "BASETYPE", size: 8, encoding: DW_ATE_signed)
!4 = !DIBasicType(name: "EXTRADATA", size: 8, encoding: DW_ATE_signed)
!5 = !{!6}
!6 = !{!"ANNOTATIONS"}']

  ["DILexicalBlock" "!5" '!0 = distinct !DILexicalBlock(scope: !1, file: !2, line: 1, column: 1)
!1 = distinct !DISubprogram(name: "SCOPE", scope: !2, file: !2, line: 1, type: !3, spFlags: DISPFlagDefinition, unit: !5)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DISubroutineType(types: !4)
!4 = !{null}
!5 = distinct !DICompileUnit(language: DW_LANG_C99, file: !6, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!6 = !DIFile(filename: "CUFILE", directory: "d")']

  ["DILexicalBlockFile" "!5" '!0 = distinct !DILexicalBlockFile(scope: !1, file: !2, discriminator: 0)
!1 = distinct !DISubprogram(name: "SCOPE", scope: !2, file: !2, line: 1, type: !3, spFlags: DISPFlagDefinition, unit: !5)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DISubroutineType(types: !4)
!4 = !{null}
!5 = distinct !DICompileUnit(language: DW_LANG_C99, file: !6, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!6 = !DIFile(filename: "CUFILE", directory: "d")']

  ["DIModule" "" '!0 = !DIModule(scope: !1, name: "SELF", configMacros: "c", includePath: "i", apinotes: "a", file: !2, line: 1)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")']

  ["DILocalVariable" "!6" '!0 = !DILocalVariable(name: "SELF", scope: !1, file: !2, line: 1, type: !3, annotations: !4)
!1 = distinct !DISubprogram(name: "SCOPE", scope: !2, file: !2, line: 1, type: !5, spFlags: DISPFlagDefinition, unit: !6)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DIBasicType(name: "TYPE", size: 8, encoding: DW_ATE_signed)
!4 = !{!7}
!5 = !DISubroutineType(types: !8)
!6 = distinct !DICompileUnit(language: DW_LANG_C99, file: !9, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!7 = !{!"ANNOTATIONS"}
!8 = !{null}
!9 = !DIFile(filename: "CUFILE", directory: "d")']

  ["DIGlobalVariable" "!5" '!0 = distinct !DIGlobalVariable(name: "SELF", linkageName: "l", scope: !1, file: !2, line: 1, type: !3, isLocal: false, isDefinition: true, declaration: !4, annotations: !7)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DIFile(filename: "FILE", directory: "d")
!3 = !DIBasicType(name: "TYPE", size: 8, encoding: DW_ATE_signed)
!4 = !DIDerivedType(tag: DW_TAG_member, name: "DECLARATION", baseType: !3, size: 8)
!5 = distinct !DICompileUnit(language: DW_LANG_C99, file: !6, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug, globals: !8)
!6 = !DIFile(filename: "CUFILE", directory: "d")
!7 = !{!9}
!8 = !{!10}
!9 = !{!"ANNOTATIONS"}
!10 = !DIGlobalVariableExpression(var: !0, expr: !DIExpression())']

  ["DICompileUnit" "!0" '!0 = distinct !DICompileUnit(language: DW_LANG_C99, file: !1, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug, enums: !2, retainedTypes: !3, globals: !4, imports: !5, macros: !6)
!1 = !DIFile(filename: "FILE", directory: "d")
!2 = !{!7}
!3 = !{!8}
!4 = !{!9}
!5 = !{!10}
!6 = !{!11}
!7 = !DICompositeType(tag: DW_TAG_enumeration_type, name: "ENUMS", size: 8, elements: !12, baseType: !8)
!8 = !DIBasicType(name: "RETAINEDTYPES", size: 8, encoding: DW_ATE_signed)
!9 = !DIGlobalVariableExpression(var: !13, expr: !DIExpression())
!10 = !DIImportedEntity(tag: DW_TAG_imported_module, name: "IMPORTS", scope: !0, entity: !14)
!11 = !DIMacro(type: DW_MACINFO_define, line: 1, name: "MACROS", value: "v")
!12 = !{!15}
!13 = distinct !DIGlobalVariable(name: "GLOBALS", scope: !0, file: !1, line: 1, type: !8, isLocal: false, isDefinition: true)
!14 = !DINamespace(name: "IMPORTSENTITY", scope: null)
!15 = !DIEnumerator(name: "ENUMSELEMENTS", value: 1)']

  ["DIImportedEntity" "" '!0 = !DIImportedEntity(tag: DW_TAG_imported_module, name: "SELF", scope: !1, entity: !2, file: !3, line: 1, elements: !4)
!1 = !DINamespace(name: "SCOPE", scope: null)
!2 = !DINamespace(name: "ENTITY", scope: null)
!3 = !DIFile(filename: "FILE", directory: "d")
!4 = !{!5}
!5 = !DIBasicType(name: "ELEMENTS", size: 8, encoding: DW_ATE_signed)']

  ["DICommonBlock" "!5" '!0 = distinct !DICommonBlock(scope: !1, declaration: !7, name: "SELF", file: !9, line: 1)
!1 = distinct !DISubprogram(name: "SCOPE", scope: !2, file: !2, line: 1, type: !3, spFlags: DISPFlagDefinition, unit: !5)
!2 = !DIFile(filename: "SCOPEFILE", directory: "d")
!3 = !DISubroutineType(types: !4)
!4 = !{null}
!5 = distinct !DICompileUnit(language: DW_LANG_C99, file: !6, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!6 = !DIFile(filename: "CUFILE", directory: "d")
!7 = distinct !DIGlobalVariable(name: "DECLARATION", scope: !1, file: !2, line: 1, type: !8, isLocal: false, isDefinition: true)
!8 = !DIBasicType(name: "DECLARATIONTYPE", size: 8, encoding: DW_ATE_signed)
!9 = !DIFile(filename: "FILE", directory: "d")']

  ["DILabel" "!5" '!0 = !DILabel(scope: !1, name: "SELF", file: !7, line: 1)
!1 = distinct !DISubprogram(name: "SCOPE", scope: !2, file: !2, line: 1, type: !3, spFlags: DISPFlagDefinition, unit: !5)
!2 = !DIFile(filename: "SCOPEFILE", directory: "d")
!3 = !DISubroutineType(types: !4)
!4 = !{null}
!5 = distinct !DICompileUnit(language: DW_LANG_C99, file: !6, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)
!6 = !DIFile(filename: "CUFILE", directory: "d")
!7 = !DIFile(filename: "FILE", directory: "d")']

  ["DIObjCProperty" "" '!0 = !DIObjCProperty(name: "SELF", file: !1, line: 1, setter: "s", getter: "g", attributes: 0, type: !2)
!1 = !DIFile(filename: "FILE", directory: "d")
!2 = !DIBasicType(name: "TYPE", size: 8, encoding: DW_ATE_signed)']

  ["DITemplateValueParameter" "" '!0 = !DITemplateValueParameter(tag: DW_TAG_template_value_parameter, name: "SELF", type: !1, value: !2)
!1 = !DIBasicType(name: "TYPE", size: 8, encoding: DW_ATE_signed)
!2 = !{!3}
!3 = !DIBasicType(name: "VALUE", size: 8, encoding: DW_ATE_signed)']

  ["DIMacroFile" "" '!0 = !DIMacroFile(line: 1, file: !1, nodes: !2)
!1 = !DIFile(filename: "FILE", directory: "d")
!2 = !{!3}
!3 = !DIMacro(type: DW_MACINFO_define, line: 1, name: "NODES", value: "v")']

  ["DIStringType" "" '!0 = !DIStringType(name: "SELF", size: 8, stringLength: !1, stringLengthExpression: !2, stringLocationExpression: !3)
!1 = !DIGlobalVariable(name: "STRINGLENGTH", scope: null, type: !4, isLocal: false, isDefinition: true)
!2 = !DIExpression()
!3 = !DIExpression()
!4 = !DIBasicType(name: "STRINGLENGTHTYPE", size: 8, encoding: DW_ATE_signed)']
]

# Each supporting node is named after the field it fills, shouted, so a name
# in the output says which field put it there. A node that exists only to make
# a probe legal (`CUFILE`, `SCOPEFILE`, the `*TYPE` helpers) is named after no
# field and drops out of both lists.

# The reference-valued fields of the node under test, in the order written.
def written-order [body: string]: nothing -> list<string> {
  $body
  | lines
  | first
  | parse --regex '(?P<field>[A-Za-z]+): !'
  | get field
}

# The fields llvm-dis numbered, in the order it numbered them.
def measured-order [text: string, fields: list<string>]: nothing -> list<string> {
  let wanted = ($fields | each {|field| {key: ($field | str downcase), field: $field}})
  $text
  | lines
  | where ($it | str starts-with "!")
  | parse --regex '"(?P<name>[A-Z]+)"'
  | get name
  | each {|name| $wanted | where key == ($name | str downcase) | get field}
  | flatten
}

def main [llvm_as: path = "llvm-as", llvm_dis: path = "llvm-dis"] {
  let work = (mktemp -d)
  for probe in $PROBES {
    let source = ([$work "probe.ll"] | path join)
    let bitcode = ([$work "probe.bc"] | path join)
    let header = if ($probe.cu | is-empty) { [] } else { [$"!llvm.dbg.cu = !{($probe.cu)}"] }
    (["!llvm.module.flags = !{!100}"
      ...$header
      "!llvm.probe = !{!0}"
      '!100 = !{i32 2, !"Debug Info Version", i32 3}'
      $probe.body]
      | str join "\n"
      | save -f $source)
    let assembled = (do { ^$llvm_as $source -o $bitcode } | complete)
    if $assembled.exit_code != 0 {
      print $"($probe.name): llvm-as refused the probe"
      print ($assembled.stderr | lines | first)
      continue
    }
    let printed = (^$llvm_dis $bitcode -o -)
    let written = (written-order $probe.body)
    let measured = (measured-order $printed $written)
    let agrees = ($measured == ($written | where $it in $measured))
    print $"($probe.name)"
    print $"  written:  ($written | str join ', ')"
    print $"  measured: ($measured | str join ', ')"
    print $"  ((if $agrees { 'agrees, so no table entry' } else { 'DIFFERS, so it needs a table entry' }))"
  }
  rm -rf $work
}
