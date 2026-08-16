# Surface inventory

How much LLVM `rustc_codegen_llvm` actually reaches for, measured rather than
estimated. This is the number PLAN.md section 3 asks for: the fork's call
sites are the specification for what this project has to provide, so counting
them turns "the LLVM that rustc uses" from a phrase into a scope.

Reproduce it with the committed script, which needs no clone:

```console
nix build nixpkgs#rustc.src
tar -xzf result --wildcards '*/compiler/rustc_codegen_llvm/*'
nu docs/surface-inventory.nu <extracted>/compiler/rustc_codegen_llvm
```

**Measured against rustc 1.95.0**, the version this repository's devshell
ships. 53 source files outside `ffi.rs` were scanned.

## The count

| Area | Entry points | Call sites | Rust shims |
| --- | --- | --- | --- |
| IR construction | 194 | 306 | 22 |
| Debug info | 40 | 44 | 12 |
| Infrastructure and host queries | 28 | 33 | 23 |
| Attributes and metadata | 23 | 52 | 15 |
| LTO, PGO and coverage | 19 | 20 | 19 |
| Other | 18 | 28 | 7 |
| Module and context | 14 | 17 | 11 |
| Targets | 9 | 13 | 6 |
| Optimization | 6 | 6 | 4 |
| Diagnostics | 5 | 6 | 5 |
| Object emission | 4 | 4 | 1 |
| Intrinsics | 2 | 2 | 0 |
| Bitcode | 1 | 1 | 0 |
| **Total** | **363** | **532** | **125** |

"Entry points" is how many distinct functions `ffi.rs` declares in that area.
"Call sites" is how many times the rest of the crate mentions them. "Rust
shims" is how many are `LLVMRust*`, which are rustc's own C++ wrappers rather
than the stable C API.

Nothing is declared and unused: every one of the 363 is mentioned somewhere
outside `ffi.rs`. The backend does not carry dead declarations, so there is no
subset to skip on the grounds that nobody calls it.

## What the shape says

**Two thirds of the surface is the tier that already exists.** IR
construction, attributes and metadata, module and context, and targets come
to 240 entry points, 66% of the total. That is the area T0 has been building,
and it is the area where an entry point is usually small: `LLVMGetUndef`
against `LLVMBuildCall2` are one line each in the count and nothing alike in
the work.

**A third of the surface is not the C API.** 125 of the 363 are `LLVMRust*`
wrappers, which exist because the stable C API does not expose what rustc
needs. PLAN.md section 2.2 treats these as churn risk, and the distribution
says where the churn lands: every one of the 19 LTO, PGO and coverage entry
points is a shim, as are 23 of the 28 infrastructure ones, while IR
construction is only 22 shims out of 194. The parts of the surface that are
stable C API are the parts we are implementing first, which is fortunate
rather than planned.

**Debug info is the largest single area outside IR construction**, at 40
entry points. That matches where the printer's remaining differences are: the
DWARF modelling that `llvm-debuginfo` will do at T1 is a real chunk of work,
not a detail.

**Object emission and bitcode are almost nothing here**, at 4 and 1 entry
points. That is not because they are small but because rustc hands whole
modules to LLVM and lets it write the file; the work behind those five calls
is an object writer and a bitcode writer, which are T2 and T3 tiers of their
own. The count measures the width of the interface, not the depth behind it.

## What this number is not

It is not an estimate of effort. An entry point is a name, and the names are
wildly uneven: `LLVMRustOptimize` is one entry point and an entire optimizer.
Read the table as a map of where the interface is wide, and read PLAN.md
section 8 for what each tier costs.

It is also not a conformance target. The backend fork (task B5) will call
these through a native Rust API rather than an FFI mirror, so the shape of
the eventual interface is a design question this measurement informs rather
than settles.
