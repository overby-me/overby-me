#!/usr/bin/env nu
# Derives the order upstream prints a specialized node's fields in.
#
#   nu md-field-order.nu [upstream-opt]
#
# A node is written with its fields in whatever order the author chose, and
# upstream prints them back in an order of its own: `!DIBasicType(size: 32,
# name: "int")` comes back as `(name: "int", size: 32)`. So a printer that
# writes them in the order they were read diverges from upstream on any module
# that did not already write them in that order.
#
# The order is not documented, so it is measured: each node kind is written
# once with its fields in reverse, and the order they come back in is the
# answer. What comes out is one line per kind, which is compared against the
# order `crates/llvm-ir-parse/src/md_schema.rs` lists them in; the printer
# sorts by that, so a kind whose two orders differ is a bug to fix.

const PROBES = [
  [node, fields];

  ["DICompileUnit" [[field, value]; [language 'DW_LANG_C99'] [file '!10'] [producer '"p"'] [isOptimized 'false'] [runtimeVersion '0'] [emissionKind 'FullDebug'] [flags '"-O2"'] [splitDebugFilename '"a.dwo"'] [dwoId '7'] [sysroot '"/"'] [sdk '"s"'] [enums '!18'] [retainedTypes '!18'] [globals '!18'] [imports '!18'] [macros '!16'] [splitDebugInlining 'false'] [debugInfoForProfiling 'true'] [nameTableKind 'GNU'] [rangesBaseAddress 'true']]]
  ["DIFile" [[field, value]; [filename '"f"'] [directory '"d"'] [checksumkind 'CSK_MD5'] [checksum '"0123456789abcdef0123456789abcdef"'] [source '"x"']]]
  ["DIBasicType" [[field, value]; [tag 'DW_TAG_unspecified_type'] [name '"n"'] [size '8'] [align '8'] [encoding 'DW_ATE_signed'] [flags 'DIFlagPublic'] [num_extra_inhabitants '1']]]
  ["DIDerivedType" [[field, value]; [tag 'DW_TAG_member'] [name '"n"'] [scope '!14'] [file '!10'] [line '1'] [baseType '!11'] [size '8'] [align '8'] [offset '8'] [flags 'DIFlagPublic'] [extraData '!11'] [annotations '!18']]]
  ["DICompositeType" [[field, value]; [tag 'DW_TAG_structure_type'] [name '"c"'] [scope '!14'] [file '!10'] [line '1'] [size '8'] [align '8'] [offset '8'] [num_extra_inhabitants '2'] [flags 'DIFlagPublic'] [elements '!18'] [templateParams '!18'] [vtableHolder '!11'] [annotations '!18'] [runtimeLang 'DW_LANG_ObjC'] [identifier '"id"']]]
  # An enumeration is the composite type that carries a `baseType` beside a
  # `scope`, a `file` and a `line`. Without it nothing said where `baseType`
  # goes: the structure probe above has no `baseType` and the array probe
  # below has no `file`, so the table put it next to `name`, where the array
  # probe alone suggested, and upstream writes it after `line`.
  ["DICompositeType/enum" [[field, value]; [tag 'DW_TAG_enumeration_type'] [name '"e"'] [scope '!14'] [file '!10'] [line '1'] [baseType '!11'] [size '8'] [align '8'] [elements '!18'] [identifier '"eid"']]]
  ["DISubroutineType" [[field, value]; [flags 'DIFlagPublic'] [cc 'DW_CC_normal'] [types '!13']]]
  ["DISubrange" [[field, value]; [count '2'] [lowerBound '1'] [stride '2']]]
  ["DIEnumerator" [[field, value]; [name '"e"'] [value '1'] [isUnsigned 'true']]]
  ["DITemplateTypeParameter" [[field, value]; [name '"T"'] [type '!11'] [defaulted 'true']]]
  ["DITemplateValueParameter" [[field, value]; [tag 'DW_TAG_GNU_template_template_param'] [name '"V"'] [type '!11'] [defaulted 'true'] [value 'i32 1']]]
  ["DINamespace" [[field, value]; [scope 'null'] [name '"n"'] [exportSymbols 'true']]]
  ["DIModule" [[field, value]; [scope 'null'] [name '"M"'] [configMacros '"-DM"'] [includePath '"/i"'] [apinotes '"a"'] [file '!10'] [line '1'] [isDecl 'true']]]
  ["DISubprogram" [[field, value]; [name '"n"'] [linkageName '"l"'] [scope '!10'] [file '!10'] [line '1'] [type '!11'] [scopeLine '2'] [flags 'DIFlagPublic'] [spFlags 'DISPFlagDefinition'] [unit '!12'] [virtualIndex '3'] [thisAdjustment '4'] [containingType '!11'] [templateParams '!18'] [declaration '!23'] [retainedNodes '!18'] [thrownTypes '!18'] [annotations '!18']]]
  ["DILexicalBlock" [[field, value]; [scope '!9'] [file '!10'] [line '1'] [column '2']]]
  ["DILexicalBlockFile" [[field, value]; [scope '!9'] [file '!10'] [discriminator '3']]]
  ["DILocalVariable" [[field, value]; [name '"v"'] [arg '1'] [scope '!9'] [file '!10'] [line '1'] [type '!19'] [flags 'DIFlagPublic'] [align '8'] [annotations '!18']]]
  ["DILabel" [[field, value]; [scope '!9'] [name '"l"'] [file '!10'] [line '1'] [column '2']]]
  ["DILocation" [[field, value]; [line '1'] [column '2'] [scope '!9'] [isImplicitCode 'true']]]
  ["DIObjCProperty" [[field, value]; [name '"p"'] [file '!10'] [line '1'] [setter '"s"'] [getter '"g"'] [attributes '1'] [type '!11']]]
  ["DIImportedEntity" [[field, value]; [tag 'DW_TAG_imported_module'] [name '"i"'] [scope 'null'] [entity '!11'] [file '!10'] [line '1'] [elements '!18']]]
  ["DIMacro" [[field, value]; [type 'DW_MACINFO_define'] [line '1'] [name '"m"'] [value '"v"']]]
  ["DIMacroFile" [[field, value]; [line '1'] [file '!10'] [nodes '!16']]]
  ["DIGlobalVariable" [[field, value]; [name '"g"'] [linkageName '"l"'] [scope 'null'] [file '!10'] [line '1'] [type '!11'] [isLocal 'true'] [isDefinition 'true'] [align '8'] [templateParams '!18'] [annotations '!18']]]
  ["DICompositeType/array" [[field, value]; [tag 'DW_TAG_array_type'] [name '"a"'] [baseType '!19'] [size '8'] [elements '!18'] [dataLocation '!20'] [associated '!20'] [allocated '!20'] [rank '!20'] [identifier '"aid"'] [annotations '!18'] [templateParams '!18']]]
  # The pointer authentication fields, which only a `DW_TAG_LLVM_ptrauth_type`
  # writes back: they share the slot `align` uses and the tag is what decides
  # which name the slot prints under. The key has to be non-zero or the whole
  # payload is dropped, so this probe carries one.
  ["DIDerivedType/ptrauth" [[field, value]; [tag 'DW_TAG_LLVM_ptrauth_type'] [name '"p"'] [baseType '!11'] [size '8'] [offset '8'] [flags 'DIFlagPublic'] [annotations '!18'] [ptrAuthKey '1'] [ptrAuthIsAddressDiscriminated 'true'] [ptrAuthExtraDiscriminator '7'] [ptrAuthIsaPointer 'true'] [ptrAuthAuthenticatesNullValues 'true']]]
  ["DIStringType" [[field, value]; [tag 'DW_TAG_string_type'] [name '"s"'] [stringLength '!20'] [stringLengthExpression '!DIExpression()'] [stringLocationExpression '!DIExpression()'] [size '8'] [align '8'] [encoding 'DW_ATE_ASCII']]]
]

def probe-text [node: string, body: string]: nothing -> string {
  let distinct = if $node in ["DICompileUnit" "DISubprogram" "DILexicalBlock" "DIGlobalVariable"] { "distinct " } else { "" }
  let units = if $node == "DICompileUnit" { '!llvm.dbg.cu = !{!12, !0}' } else { '!llvm.dbg.cu = !{!12}' }
  ([
    '!named = !{!0}'
    $"!0 = ($distinct)!($node | split row '/' | first)\(($body)\)"
    '!9 = distinct !DISubprogram(name: "s", scope: !10, file: !10, line: 1, type: !11, spFlags: DISPFlagDefinition, unit: !12)'
    '!10 = !DIFile(filename: "a", directory: "d")'
    '!11 = !DISubroutineType(types: !13)'
    '!12 = distinct !DICompileUnit(language: DW_LANG_C99, file: !10, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug)'
    '!13 = !{null}'
    '!14 = !DINamespace(name: "ns", scope: null)'
    '!16 = !{!17}'
    '!17 = !DIMacro(type: DW_MACINFO_define, name: "m", value: "v")'
    '!18 = !{}'
    '!19 = !DIBasicType(name: "int", size: 32, encoding: DW_ATE_signed)'
    '!20 = !DILocalVariable(name: "len", scope: !9)'
    '!22 = !DIDerivedType(tag: DW_TAG_member, name: "d", scope: !14, baseType: !19, size: 8)'
    '!23 = !DISubprogram(name: "decl", scope: !10, file: !10, line: 1, type: !11, spFlags: DISPFlagOptimized)'
    '!24 = !DIGlobalVariable(name: "gdecl", scope: null, isLocal: false, isDefinition: false)'
    $units
    '!llvm.module.flags = !{!15}'
    '!15 = !{i32 2, !"Debug Info Version", i32 3}'
  ] | str join "\n")
}

def main [upstream_opt: path = "opt"] {
  let work = (mktemp -d)
  let source = ([$work "probe.ll"] | path join)
  for probe in $PROBES {
    # Written backwards, so an order that comes back forwards is upstream's
    # rather than the one the probe happened to use.
    let body = ($probe.fields | reverse | each {|f| $"($f.field): ($f.value)"} | str join ", ")
    probe-text $probe.node $body | save -f $source
    let printed = (do { ^$upstream_opt -S $source -o - } | complete)
    if $printed.exit_code != 0 {
      print $"($probe.node): refused, so nothing can be told from it"
      print ($printed.stderr | lines | where ($it =~ "error") | first)
      continue
    }
    let line = ($printed.stdout | lines | where ($it | str starts-with "!0 = ") | first)
    let order = (
      $line
      | parse --regex '\((?P<body>.*)\)$'
      | get body.0
      | split row ", "
      | each {|pair| $pair | split row ": " | first}
      | str join " "
    )
    # A field written at its default is not written back, so it would be
    # missing from the order. Saying so is what keeps the table complete.
    let asked = ($probe.fields | get field)
    let got = ($order | split row " ")
    let missing = ($asked | where {|f| $f not-in $got})
    if ($missing | is-not-empty) {
      print $"($probe.node): ($order)   [dropped: ($missing | str join ' ')]"
    } else {
      print $"($probe.node): ($order)"
    }
  }
  rm -rf $work
}
