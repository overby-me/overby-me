# LLVM-rs

LLVM-compatible compiler infrastructure written in Rust. Compatible at the
**LLVM IR level** (textual `.ll` today, bitcode later) and eventually at the
**LLVM-C ABI level**, never at the C++ API level. Scope is deliberately "the
LLVM that rustc actually uses", not all of `llvm-project`.

The end state is a pure-Rust codegen path for rustc:
`rustc -> rust/llvm -> object files -> wild`, with `rust/libc` underneath.
[PLAN.md](./PLAN.md) is the plan of record; [STATUS.md](./STATUS.md) is the
honest account of what exists right now.

## Where this is

**Tier T0, in progress.** What works today is the IR core: parse textual LLVM
IR, verify it, print it back. There is no optimizer, no code generator, no
object emission, and no rustc backend yet. Nothing here compiles a Rust
program. See [STATUS.md](./STATUS.md) for the per-area breakdown and for the
claims that are explicitly *not* being made.

## Layout

| Crate | What it is |
| --- | --- |
| `crates/llvm-support` | APInt, APFloat, DataLayout, Triple: the value and target primitives everything else is phrased in |
| `crates/llvm-ir` | Context, Module, types, constants, instructions, attributes, metadata, and the verifier |
| `crates/llvm-ir-print` | Textual `.ll` printer, including LLVM's value-numbering rules |
| `crates/llvm-ir-parse` | Textual `.ll` lexer and parser |
| `crates/llvm-tools` | Binaries with upstream-compatible CLI subsets (`opt` today) |

Crates named in [PLAN.md](./PLAN.md) §4.1 but not present yet (`llvm-codegen`,
`llvm-target-x86`, `llvm-bitcode`, `rustc-codegen-llvmrs`, ...) are not stubs
that were left empty; they have not been started.

## Using it

```console
# Parse, verify and re-print a module (the subset of upstream opt we implement)
nix run .#rust-llvm -- -S -passes=verify input.ll -o output.ll

# Same thing from a checkout
cargo run -p llvm-tools --bin opt -- -S -passes=verify input.ll
```

`opt` accepts `-S`, `-o`, `-passes=` (only `verify` and `no-op-module` are
implemented), `--verify-each`, and reads `-` for stdin. Anything else is a hard
error rather than a silent no-op, so a caller never gets a file that quietly
skipped the work it asked for.

## Building and testing

Everything builds and tests through nix; there is no second execution path.

```console
nix build .#rust-llvm                          # the tools
nix build .#checks.x86_64-linux.llvm-fmt       # rustfmt
nix build .#checks.x86_64-linux.llvm-clippy    # clippy, -D warnings
nix build .#checks.x86_64-linux.llvm-unit      # cargo test
nix build .#checks.x86_64-linux.llvm-roundtrip # corpus round-trip fidelity
nix build .#checks.x86_64-linux.llvm-verify-corpus
```

Never run `nix flake check` in this repo (it gets OOM-killed); build the named
checks individually, or run `just check` for the bounded-memory sweep.

For iteration, plain cargo works and needs no nightly: the library crates
build on stable Rust and have no third-party dependencies at all.

```console
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Corpus

`corpus/rustc/` holds `.ll` files produced by real `rustc --emit=llvm-ir`, and
`corpus/handwritten/` holds small files that pin specific syntax. The
round-trip check requires that parsing and re-printing every corpus file
reproduces the input byte for byte, which is a much stronger property than
"the parser accepted it" and is what keeps the printer honest about LLVM's
formatting and numbering rules.

Upstream LLVM's own `llvm/test/Assembler` and `llvm/test/Verifier` suites are
used as an oracle from `nixpkgs` sources inside check derivations rather than
being vendored into the tree.

## License

Apache-2.0 WITH LLVM-exception, deliberately matching upstream LLVM so that
vendoring upstream tests and any future two-way flow stay clean. See
[LICENSE](./LICENSE) and [PLAN.md](./PLAN.md) §12.

This is a clean-room implementation: LangRef, the textual IR format and the
upstream test suites are read as specifications, C++ LLVM source is not ported.
