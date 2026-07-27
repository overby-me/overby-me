#!/usr/bin/env nu
# Measures the surface rust/llvm has to implement, from the backend that will
# consume it.
#
#   nu surface-inventory.nu <path-to-rustc_codegen_llvm>
#
# The crate's ffi.rs declares every LLVM entry point the backend can reach.
# That list, grouped by the areas in PLAN.md section 3, is the scope of this
# project stated as a number rather than as a guess. Each area gets three:
# how many entry points it declares, how many times the rest of the crate
# mentions them, and how many are LLVMRust shims rather than the stable C
# API, because PLAN.md section 2.2 counts those separately: they are rustc's
# own C++ wrappers and they churn every release.
#
# Get the source without cloning anything:
#
#   nix build nixpkgs#rustc.src
#   tar -xzf result --wildcards '*/compiler/rustc_codegen_llvm/*'
#
# enzyme_ffi.rs is excluded: it is an optional autodiff integration rather
# than part of codegen.

# Which area an entry point belongs to. First match wins, so the order is the
# classification: the specific buckets come before the general ones.
const AREAS = [
  [pattern, area];
  ["Coverage", "LTO, PGO and coverage"]
  ["Profile", "LTO, PGO and coverage"]
  ["PGO", "LTO, PGO and coverage"]
  ["ThinLTO", "LTO, PGO and coverage"]
  ["LTO", "LTO, PGO and coverage"]
  ["Bitcode", "Bitcode"]
  ["DIBuilder", "Debug info"]
  ["DebugLoc", "Debug info"]
  ["Debug", "Debug info"]
  ["DI", "Debug info"]
  ["Attribute", "Attributes and metadata"]
  ["Attr", "Attributes and metadata"]
  ["Metadata", "Attributes and metadata"]
  ["MDNode", "Attributes and metadata"]
  ["MDString", "Attributes and metadata"]
  ["MetadataAsValue", "Attributes and metadata"]
  ["TargetMachine", "Targets"]
  ["Target", "Targets"]
  ["DataLayout", "Targets"]
  ["Triple", "Targets"]
  ["Optimize", "Optimization"]
  ["PassBuilder", "Optimization"]
  ["Pass", "Optimization"]
  ["WriteOutputFile", "Object emission"]
  ["EmitToBuffer", "Object emission"]
  ["ObjectFile", "Object emission"]
  ["Archive", "Object emission"]
  ["Section", "Object emission"]
  ["Comdat", "Object emission"]
  ["Build", "IR construction"]
  ["Const", "IR construction"]
  ["Type", "IR construction"]
  ["BasicBlock", "IR construction"]
  ["Builder", "IR construction"]
  ["Global", "IR construction"]
  ["Function", "IR construction"]
  ["Value", "IR construction"]
  ["Inst", "IR construction"]
  ["Operand", "IR construction"]
  ["Intrinsic", "Intrinsics"]
  ["AddCase", "IR construction"]
  ["AddClause", "IR construction"]
  ["AddHandler", "IR construction"]
  ["AddIncoming", "IR construction"]
  ["AddAlias", "IR construction"]
  ["AddNamedGlobal", "IR construction"]
  ["Undef", "IR construction"]
  ["Poison", "IR construction"]
  ["Param", "IR construction"]
  ["Linkage", "IR construction"]
  ["Visibility", "IR construction"]
  ["Alignment", "IR construction"]
  ["Initializer", "IR construction"]
  ["InlineAsm", "IR construction"]
  ["ReplaceAllUses", "IR construction"]
  ["FastMath", "IR construction"]
  ["AlgebraicMath", "IR construction"]
  ["AllowReassoc", "IR construction"]
  ["AggregateElement", "IR construction"]
  ["VectorSize", "IR construction"]
  ["InsertBlock", "IR construction"]
  ["IsDeclaration", "IR construction"]
  ["ThreadLocal", "IR construction"]
  ["DSOLocal", "IR construction"]
  ["Volatile", "IR construction"]
  ["Ordering", "IR construction"]
  ["TailCall", "IR construction"]
  ["Predicate", "IR construction"]
  ["Buffer", "Infrastructure and host queries"]
  ["Symbols", "Infrastructure and host queries"]
  ["Linker", "Infrastructure and host queries"]
  ["HostCPU", "Infrastructure and host queries"]
  ["Feature", "Infrastructure and host queries"]
  ["Statistics", "Infrastructure and host queries"]
  ["Multithreaded", "Infrastructure and host queries"]
  ["Message", "Infrastructure and host queries"]
  ["Mangled", "Infrastructure and host queries"]
  ["Bundle", "Infrastructure and host queries"]
  ["Offload", "Infrastructure and host queries"]
  ["SystemDialogs", "Infrastructure and host queries"]
  ["SymbolicFile", "Infrastructure and host queries"]
  ["Arm64Coff", "Infrastructure and host queries"]
  ["ECObject", "Infrastructure and host queries"]
  ["Zlib", "Infrastructure and host queries"]
  ["Zstd", "Infrastructure and host queries"]
  ["Version", "Infrastructure and host queries"]
  ["Install", "Infrastructure and host queries"]
  ["Initialize", "Infrastructure and host queries"]
  ["Dispose", "Infrastructure and host queries"]
  ["Module", "Module and context"]
  ["Context", "Module and context"]
  ["Diagnostic", "Diagnostics"]
  ["Error", "Diagnostics"]
  ["Timer", "Diagnostics"]
  ["Remark", "Diagnostics"]
]

def classify [name: string]: nothing -> string {
  for row in $AREAS {
    if ($name | str contains $row.pattern) {
      return $row.area
    }
  }
  "Other"
}

def main [crate: path] {
  let ffi = ([$crate "src" "llvm" "ffi.rs"] | path join)
  if not ($ffi | path exists) {
    error make {msg: $"($ffi) does not exist; point this at rustc_codegen_llvm"}
  }

  let declared = (
    open --raw $ffi
    | parse --regex 'fn (?P<name>LLVM[A-Za-z0-9_]+)'
    | get name
    | uniq
    | sort
  )

  # Every LLVM identifier the crate mentions outside the declarations, which
  # is what "the backend calls this" means here.
  let sources = (
    glob ([$crate "**" "*.rs"] | path join)
    | where {|f| ($f != $ffi) and not ($f | str ends-with "enzyme_ffi.rs")}
  )
  let mentions = (
    $sources
    | each {|f| open --raw $f | parse --regex '(?P<name>LLVM[A-Za-z0-9_]+)' | get name}
    | flatten
  )
  let counts = ($mentions | reduce --fold {} {|name, acc|
    $acc | upsert $name (($acc | get -o $name | default 0) + 1)
  })

  let table = (
    $declared
    | each {|name| {
        name: $name,
        area: (classify $name),
        rust_shim: ($name | str starts-with "LLVMRust"),
        calls: ($counts | get -o $name | default 0),
      }}
  )

  let by_area = (
    $table
    | group-by area
    | transpose area rows
    | each {|group| {
        area: $group.area,
        declared: ($group.rows | length),
        uses: ($group.rows | get calls | math sum),
        shims: ($group.rows | where rust_shim | length),
      }}
    | sort-by declared --reverse
  )

  print "| Area | Entry points | Call sites | Rust shims |"
  print "| --- | --- | --- | --- |"
  for row in $by_area {
    print $"| ($row.area) | ($row.declared) | ($row.uses) | ($row.shims) |"
  }
  let declared_total = ($table | length)
  let uses_total = ($table | get calls | math sum)
  let shim_total = ($table | where rust_shim | length)
  print $"| **Total** | **($declared_total)** | **($uses_total)** | **($shim_total)** |"
  print ""
  let never = ($table | where calls == 0)
  print $"Sources scanned: ($sources | length) files outside ffi.rs."
  print $"Entry points nothing outside ffi.rs mentions: ($never | length)."
}
