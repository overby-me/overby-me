# Fe-C 🦀

Gradual memory-safety hardening for unsafe Rust and mixed-language programs.

**Fe-C** is the iron–carbon alloy system. Pure iron is soft; a controlled amount
of carbon, correctly alloyed, gives cast iron and steel. Safe Rust plus a
controlled amount of C-heritage `unsafe`, correctly checked, is stronger than
either. The name is also a sideways nod to [Fil-C](https://fil-c.org) — read
[Fe-C is not Fil-C](#fe-c-is-not-fil-c) before assuming anything about
guarantees.

## What it does

Safe Rust is proven at compile time, so Fe-C spends its runtime budget only
where the type system abdicates:

- **Deterministic boundary checks** where raw pointers become safe pointers
  (`&*p`, `&mut *p`, `Box::from_raw`): bounds, liveness, and provenance are
  validated against a global allocation table at the cast site — failing at the
  root cause, not at a distant dereference.
- **FFI enforcement**: libc symbol interposition (allocator family, `mem*`,
  ptr+len syscall wrappers), `extern "C"` prologue checks, and sampled
  guard-page/canary allocations (GWP-ASan style) so even opaque C's own heap
  misuse traps in hardware.
- **A per-crate hardening dial**, set like an optimization level.

## Hardening modes

| Mode | Metallurgy | Checked | Assumed |
| ---- | ---------- | ------- | ------- |
| `--harden=case` | Case hardening: hard surface, fast ductile core | Raw→safe casts, FFI boundaries, alloc/free, optional re-checks after potential deallocation | Rust aliasing + validity rules hold inside elided (type-proven) regions; the optimizer shares those assumptions |
| `--harden=through` | Through hardening: uniform to the core | Every access, via opaque runtime calls the optimizer cannot reason around | Only the `cementite` runtime and syscall stubs (the same shape of trusted base Fil-C's runtime keeps) |

`fil` is a **reserved alias** for `through`, to be enabled only once the
guarantee is actually earned (precedent: Zig's proposed `fil` ABI mode).
Modes are per-crate, recorded in crate metadata, mediated by cross-mode call
adapters. Both modes are first-class from v0; see [PLAN.md](./PLAN.md).

## Fe-C is not Fil-C

- **Fil-C** is unconditional: garbage in, memory safety out, for arbitrary
  code; its compiler never optimizes on assumptions the runtime doesn't
  enforce; the cost is ~1.5–4× everywhere.
- **Fe-C `case`** is conditional: deterministic detection of the bug classes
  Rust programs actually ship (unsafe/FFI misuse), at near-zero cost for safe
  code — but an aliasing-UB path can corrupt state without crossing a checked
  boundary. It is a testing/hardening tool, **not a containment boundary for
  hostile code**.
- **Fe-C `through`** aims at the Fil-C-grade guarantee inside Rust's world,
  staged deliberately (see PLAN.md §5–7).
- Fe-C does not check Rust's aliasing model in any mode — that is
  [Miri](https://github.com/rust-lang/miri)'s job; run both.
- For lifetime-bound bugs (too-relaxed signatures letting borrows outlive their
  frame), Fe-C **detects and contains the consequence**; it does not prevent
  the mistake. Correct signatures, `cargo audit`, and static analysis do.
- `case` mode's free-during-scope detection is single-thread-sound: a
  concurrent free between re-check and dereference can be missed. `through`
  mode has no such window.
- `through` mode guarantees memory safety for programs free of **data races on
  pointer slots** (already UB in Rust). Fil-C is stronger here: it makes races
  harmless to memory unconditionally. See `docs/through-mode-coherence.md`.
- If you need guaranteed containment today: Fil-C, CHERI hardware, or a wasm
  boundary.

## Architecture

One Rust workspace. No compiler fork, no LLVM linkage, no submodules.

| Crate | Role |
| ----- | ---- |
| `cargo-fe-c` | Cargo subcommand + `RUSTC_WRAPPER`; instruments the whole graph incl. `std` via `-Zbuild-std` |
| `fe-c-driver` | rustc-as-a-library (`rustc_public` where possible); MIR analysis + rewriting of accesses into plain runtime calls; per-crate mode metadata |
| `cementite` | The hard phase (Fe₃C): allocation table (never-recycled IDs + liveness epochs), check functions, quarantining `#[global_allocator]`, libc interceptors |

Toolchain coupling budget: one pinned nightly in `rust-toolchain.toml`, bumped
by an automated pipeline (the Kani maintenance model).

## Substrate

Fe-C pairs with [`../libc`](../libc) (vendored Eyra lineage) so that
"whole-process" means whole process: the libc's own `unsafe` is instrumented
like everyone else's. The unchecked residue shrinks to syscall stubs and a few
lines of asm.

## Nix ❄️

Everything routes through the flake:

- `nix build .#cargo-fe-c` / `.#cementite`
- `nix build .#fe-c-sysroot-case-x86_64` — instrumented `core`/`alloc`/`std`
  as cached derivations, rebuilt only on nightly bumps
- `nix develop` — pinned nightly + `rustc-dev` + `rust-src` + miri
- `nix flake check` — the CI entrypoint (fmt, lints, unit, RustSec corpus,
  false-positive suite, selfhost, miri-on-runtime); run by the tangled spindle
  pipeline

## Status

Design phase. Nothing here is a security claim yet.

- **Starting work (human or agent): [CLAUDE.md](./CLAUDE.md)** — hard rules,
  settled decisions, ordered task queue.
- Design record: [PLAN.md](./PLAN.md). Evidence: [docs/traces/](./docs/traces).

Monorepo table row:

```markdown
| [Fe-C 🦀](https://tangled.org/@overby.me/overby.me/tree/main/rust/fe-c) | Gradual memory-safety hardening for unsafe Rust and mixed-language programs |
```

## Prior art

[Fil-C](https://fil-c.org) (InvisiCaps, FUGC) ·
[SafeFFI](https://arxiv.org/abs/2510.20688) (USENIX Security '26) ·
[Miri](https://github.com/rust-lang/miri) ·
[rustc_public](https://github.com/rust-lang/rustc_public) ·
[Kani](https://github.com/model-checking/kani) (maintenance model) ·
CHERI rustc / strict provenance ·
[Eyra](https://github.com/sunfishcode/eyra) ·
GWP-ASan · Zig [#36237](https://codeberg.org/ziglang/zig/issues/36237)
