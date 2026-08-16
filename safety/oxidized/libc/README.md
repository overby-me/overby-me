# Libc-rs 🦀

All-Rust libc substrate for Linux — the vendored [Eyra](https://github.com/sunfishcode/eyra)
lineage, adopted into the monorepo.

Directory `safety/oxidized/libc` follows the repo convention (the component being
replaced); the code keeps its upstream crate identities:

| Vendored crate | Role | Upstream |
| -------------- | ---- | -------- |
| `origin` | Program & thread startup/shutdown in Rust (asm minimized) | sunfishcode/origin |
| `c-ward` (`c-scape`, `c-gull`) | ABI-compatible libc functions implemented in Rust | sunfishcode/c-ward |
| `eyra` | Facade crate that slides the above under `std` | sunfishcode/eyra |

**Not vendored:** `rustix` (and friends) stay normal crates.io dependencies —
it is the one layer that remains large and very actively maintained.

## Why

1. **Whole-process Fe-C coverage.** Rewriting doesn't eliminate danger; it
   converts *opaque C* into *instrumentable unsafe Rust*. With this substrate,
   [`../fe-c`](../fe-c) checks the entire process — libc included — and the
   unchecked residue shrinks to syscall stubs and a few lines of asm.
2. **The Rust-userspace trajectory.** This is the libc row of the NixOS-rs
   plan: a userland that is Rust all the way down to the syscall.
3. **Upstream's own reasons**: whole-program LTO through the libc, the
   `set_var` soundness fix, and fully static linking that still honors the
   platform NSS/DNS config.

## Constraints (upstream-honest)

- Nightly Rust only; Linux only (x86-64, x86, aarch64, riscv64).
- No dynamic linking.
- Cannot run under Miri (syscalls issued from asm are not recognized), though
  the code targets strict provenance and I/O safety throughout.
- Contains substantial unavoidable `unsafe`; per upstream's own README it
  should **not** be presumed safer than mature C until proven — proving it is
  exactly what Fe-C `case` mode is for (see `../fe-c`).

## Provenance & licensing

- Each vendored tree carries an `UPSTREAM` file: repo URL, commit hash, date,
  local patch list. Plain source import at a pinned commit — no submodules
  (jj-friendly).
- Upstream is quiescent, not dead: repo unarchived, last tagged release
  0.16.0 (Oct 2023), sparse crates.io pushes since. Policy: adopt-and-patch
  here, diff upstream periodically, send patches back while it answers.
- Licenses preserved verbatim: Apache-2.0 / Apache-2.0 WITH LLVM-exception /
  MIT, plus the upstream COPYRIGHT notice.

## Usage

Classic Eyra hookup (works with plain `cargo build` on the pinned nightly):

```toml
# Cargo.toml
[dependencies]
std = { package = "eyra", path = "../libc/eyra" }
```

```rust
// build.rs
fn main() { println!("cargo:rustc-link-arg=-nostartfiles"); }
```

All-from-source (the Fe-C path): `-Zbuild-std` + `extern crate eyra;` instead
of the rename trick. Fully static: `-C target-feature=+crt-static`
(`experimental-relocate` for static-PIE + ASLR).

## Nix ❄️

- `nix build .#libc-hello` — smoke binary, plus `-static` and `-static-pie`
  variants
- `nix build .#libc-sysroot` — the eyra-linked std as a derivation, consumed
  by other `rust/` projects (and by `fe-c-sysroot-*` once P2 lands)
- `nix flake check`: hello, static DNS resolution, upstream example subset
  (ripgrep/coreutils smoke), and — phase-gated — the Fe-C `case`-mode build
- Shares the repo-wide pinned nightly with `../fe-c`.

## Status

Pre-vendor. See [PLAN.md](./PLAN.md) for the import procedure and bring-up
phases.

Monorepo table row:

```markdown
| [Libc-rs 🦀](https://tangled.org/@overby.me/overby.me/tree/main/rust/libc) | All-Rust libc substrate (vendored Eyra/origin/c-ward lineage) |
```
