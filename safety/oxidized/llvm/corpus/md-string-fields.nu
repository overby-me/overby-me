#!/usr/bin/env nu
# Derives which specialized-metadata fields may hold a metadata string.
#
#   nu md-string-fields.nu [upstream-opt]
#
# A field written `name: "text"` holds text, and a field written `scope: !2`
# holds a node. `!"text"` is neither: it is a reference to a metadata string,
# which is the thing a `!typerefs` list holds when debug info names a type it
# has not seen yet. Upstream refuses one almost everywhere, which is what
# `llvm/test/Verifier/dbg-typerefs.ll` checks, and accepts it in the one place
# a template argument can be a name rather than a type.
#
# Where the exceptions are is not something to guess, so it is measured: every
# field of every node kind is written once as `!"probe"`, and the ones upstream
# still reads are the exceptions. What comes out is that list.

const PROBES = [
  [node, skeleton, fields];

  ["DICompileUnit" 'language: DW_LANG_C99, file: !10, producer: "p", isOptimized: false, runtimeVersion: 0, emissionKind: FullDebug' [file producer splitDebugFilename enums retainedTypes globals imports macros sysroot sdk]]
  ["DIFile" 'filename: "f", directory: "d"' [filename directory source checksumkind]]
  ["DIBasicType" 'name: "n"' [tag name size align encoding flags]]
  ["DIDerivedType" 'tag: DW_TAG_member, baseType: null' [tag name scope file baseType line size align offset flags extraData annotations]]
  ["DICompositeType" 'tag: DW_TAG_structure_type' [tag name scope file baseType line size align offset flags elements runtimeLang vtableHolder templateParams identifier discriminator dataLocation associated allocated rank annotations]]
  ["DISubroutineType" 'types: null' [types flags cc]]
  ["DISubrange" 'count: 1' [count lowerBound upperBound stride]]
  ["DIEnumerator" 'name: "e", value: 1' [name value isUnsigned]]
  ["DITemplateTypeParameter" 'name: "T", type: null' [name type defaulted]]
  ["DITemplateValueParameter" 'name: "V", type: null, value: null' [name type value defaulted]]
  ["DINamespace" 'name: "n", scope: null' [name scope exportSymbols]]
  ["DIModule" 'scope: null, name: "M"' [scope name configMacros includePath apinotes file line]]
  ["DISubprogram" 'name: "n", scope: null, type: null' [name scope file type linkageName line scopeLine containingType declaration retainedNodes templateParams thrownTypes annotations]]
  ["DILexicalBlock" 'scope: !9' [scope file line column]]
  ["DILocalVariable" 'name: "v", scope: !9' [name scope file type line arg flags align annotations]]
  ["DILabel" 'scope: !9, name: "l", file: !10, line: 1' [scope name file line column]]
  ["DILocation" 'line: 1, scope: !9' [scope inlinedAt line column]]
  ["DIObjCProperty" 'name: "p"' [name file line setter getter attributes type]]
  ["DIImportedEntity" 'tag: DW_TAG_imported_module, scope: null' [tag scope entity file line name elements]]
  ["DIMacro" 'type: DW_MACINFO_define, name: "m"' [type name value line]]
  ["DIMacroFile" 'file: !10' [file nodes line]]
  ["DIGlobalVariable" 'name: "g", scope: null, isLocal: false, isDefinition: true' [name scope file type linkageName line declaration templateParams annotations]]
  ["DIGlobalVariableExpression" 'var: !15, expr: !DIExpression()' [var expr]]
  ["DIStringType" 'name: "s", size: 8' [name size align stringLength stringLengthExpression stringLocationExpression]]
]

# A module holding one node, plus the scope a function-local node needs.
def probe-text [node: string, body: string]: nothing -> string {
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
    '!15 = distinct !DIGlobalVariable(name: "g", scope: null, type: !11, isLocal: false, isDefinition: true)'
    $units
    '!llvm.module.flags = !{!14}'
    '!14 = !{i32 2, !"Debug Info Version", i32 3}'
  ] | str join "\n")
}

def main [upstream_opt: path = "opt"] {
  let work = (mktemp -d)
  let source = ([$work "probe.ll"] | path join)
  mut accepted = []
  for probe in $PROBES {
    probe-text $probe.node $probe.skeleton | save -f $source
    let baseline = (do { ^$upstream_opt -S $source -o - } | complete)
    if $baseline.exit_code != 0 {
      print $"($probe.node): the skeleton alone is refused, so nothing can be told from it"
      continue
    }
    for field in $probe.fields {
      # The skeleton may already set this field; the probe replaces it.
      let kept = (
        $probe.skeleton
        | split row ", "
        | where not ($it | str starts-with $"($field): ")
        | str join ", "
      )
      let body = if ($kept | is-empty) { $"($field): !\"probe\"" } else { $"($kept), ($field): !\"probe\"" }
      probe-text $probe.node $body | save -f $source
      # A field upstream crashes on gives no verdict to copy, so it is
      # reported and skipped rather than read as a refusal.
      let attempt = (try { do { ^$upstream_opt -S $source -o - } | complete } catch { {exit_code: 139} })
      if $attempt.exit_code >= 128 {
        print $"($probe.node).($field): upstream crashes, so there is no verdict"
      } else if $attempt.exit_code == 0 {
        $accepted = ($accepted | append $"($probe.node).($field)")
        print $"($probe.node).($field): ACCEPTS a metadata string"
      } else {
        print $"($probe.node).($field): refuses one"
      }
    }
  }
  rm -rf $work
  print ""
  print "fields that accept a metadata string:"
  print ($accepted | str join "\n")
}
