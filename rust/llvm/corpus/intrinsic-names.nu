#!/usr/bin/env nu
# Harvests the intrinsic base names LangRef documents.
#
#   nu intrinsic-names.nu <llvm-source-tree> [out-file]
#
# Upstream materialises a declaration for an undeclared `llvm.*` name when it
# recognises the name, and reports "use of undefined value" when it does not.
# Telling those apart needs the set of real intrinsics, and the only
# specification of that set we are allowed to read is LangRef, which documents
# each intrinsic with a `declare` line naming it.
#
# What comes out is base names, with the mangling suffix stripped: LangRef
# writes `@llvm.smax.i32` and `@llvm.smax.v4i32` for one intrinsic, and a call
# may mangle it any way its argument types require. So `llvm.smax` is what the
# table holds and a call's own suffix is not checked, which is looser than
# upstream and never looser than refusing the call outright.
#
# The list is deliberately incomplete: it has what LangRef documents and not
# the target-specific intrinsics that live in each backend's own tables, so
# `llvm.amdgcn.ds.append` is still refused when undeclared. That is a gap with
# a number attached rather than a guess.

def main [tree: path, out: path = "intrinsic-names.txt"] {
  let langref = ([$tree "llvm" "docs" "LangRef.rst"] | path join)
  if not ($langref | path exists) {
    error make {msg: $"no LangRef at ($langref)"}
  }

  # Anything a mangling suffix can look like: a vector, a pointer, an integer
  # or a float width, or an array.
  let mangled = '^(v[0-9].*|nxv[0-9].*|p[0-9]+|i[0-9]+|f[0-9]+|bf[0-9]+|f80|f128|ppcf128|isVoid|a[0-9].*)$'

  let names = (
    open --raw $langref
    | parse --regex '@(?P<name>llvm\.[A-Za-z0-9_.]*[A-Za-z0-9_])'
    | get name
    | uniq
  )

  let bases = (
    $names
    | each {|name|
      mut parts = ($name | split row ".")
      while (($parts | length) > 2 and (($parts | last) =~ $mangled)) {
        $parts = ($parts | drop 1)
      }
      $parts | str join "."
    }
    | uniq
    | sort
  )

  $bases | str join "\n" | $"($in)\n" | save --force $out
  print $"($names | length) names in LangRef, ($bases | length) base names into ($out)"
}
