# Implementation Plan

A phased roadmap for replacing nixpkgs stdenv's C-based tools with Rust rewrites. Each phase builds on the previous, expanding the "oxidized surface" of the build toolchain while maintaining full backward compatibility with existing nixpkgs derivations.

## Overview

The nixpkgs standard environment (`stdenv`) is the foundation that builds every package in Nix. It consists of ~15 C-based tools (bash, coreutils, make, grep, sed, etc.) plus a shell-script orchestration layer (`setup.sh` / `mkDerivation`). This plan replaces each component with a Rust equivalent — either an existing community rewrite or a new repo-root subproject.

### Architecture

```text
┌──────────────────────────────────────────────────────┐
│                    mkDerivation                       │
│   (Phase 6: Rust binary replaces setup.sh phases)     │
├──────────────────────────────────────────────────────┤
│               stdenv.initialPath                      │
│                                                       │
│  ┌─────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐   │
│  │  shell   │ │coreutils │ │  make  │ │ tar+gz+  │   │
│  │  (P1)    │ │  (P1)    │ │  (P4)  │ │ bz2+xz   │   │
│  │rust-bash │ │ uutils   │ │        │ │  (P3)    │   │
│  └─────────┘ └──────────┘ └────────┘ └──────────┘   │
│  ┌─────────┐ ┌──────────┐ ┌────────┐ ┌──────────┐   │
│  │   sed    │ │  grep    │ │  awk   │ │diffutils │   │
│  │  (P2)    │ │  (P2)    │ │  (P2)  │ │  (P2)    │   │
│  └─────────┘ └──────────┘ └────────┘ └──────────┘   │
│  ┌─────────┐ ┌──────────┐ ┌────────────┐            │
│  │  find    │ │  patch   │ │  patchelf  │            │
│  │ xargs    │ │  (P4)    │ │   strip    │            │
│  │  (P2)    │ │          │ │   (P5)     │            │
│  └─────────┘ └──────────┘ └────────────┘            │
├──────────────────────────────────────────────────────┤
│              Nix abstractions (rust-nixpkgs)          │
│  components/*.nix │ stdenv.nix │ lib.nix │ tests      │
└──────────────────────────────────────────────────────┘
```

### Component Inventory

| Component | Original | Rust Replacement | Source | Phase | Status |
|-----------|----------|-----------------|--------|-------|--------|
| Shell | bash | [rust-bash](../bash) | repo | 1 | ✅ Available |
| Core utilities | coreutils | [uutils](https://github.com/uutils/coreutils) | nixpkgs | 1 | ✅ Available |
| Stream editor | gnused | [rust-sed](../sed) | repo | 2 | ✅ Available |
| Pattern grep | gnugrep | rust/grep | repo | 2 | ✅ Available |
| Awk | gawk | rust/awk | repo | 2 | ✅ Available |
| File search | findutils | [uutils-findutils](https://github.com/uutils/findutils) | nixpkgs | 2 | ✅ Available |
| Diff | diffutils | [rust-diffutils](../diffutils) | repo | 2 | ✅ Available |
| Tar archive | gnutar | rust/tar | repo | 3 | ✅ Available |
| Gzip | gzip | rust/gzip | repo | 3 | ✅ Available |
| Bzip2 | bzip2 | rust/bzip2 | repo | 3 | ✅ Available |
| XZ/LZMA | xz | rust/xz | repo | 3 | ✅ Available |
| Build driver | gnumake | rust/make | repo | 4 | ✅ Available |
| Patch | gnupatch | rust/patch | repo | 4 | ✅ Available |
| ELF patcher | patchelf | rust/patchelf | repo | 5 | ✅ Available |
| Symbol strip | binutils (strip) | rust/strip | repo | 5 | ✅ Available |
| Build phases | setup.sh | rust-mkderivation (future) | repo | 6 | ⏳ Planned |

---

## Phase 0 — Foundation

**Goal:** Establish the project structure, Nix abstractions, and testing infrastructure.

**Status:** ✅ Complete

### Deliverables

- [x] Project layout: `rust-nixpkgs/` with `default.nix`, `lib.nix`, `stdenv.nix`
- [x] Component registry: `components/*.nix` with status tracking for all 15+ tools
- [x] `lib.nix`: `mkComponent`, `loadComponents`, `mkRustStdenv`, `mkReplacements`
- [x] `stdenv.nix`: stdenv override assembler using available components
- [x] `default.nix`: flakelight integration (devShell, overlay, test package)
- [x] PLAN.md: this document
- [x] README.md: project overview and usage

### Design Decisions

1. **Component files, not crate stubs.** Each `components/*.nix` file declares the replacement mapping. Actual Rust rewrites live under `rust/` (e.g. `rust/make/`) or come from nixpkgs (e.g. `pkgs.uutils-coreutils-noprefix`).

2. **Null means "not yet available."** Components with `replacement = null` are tracked in the registry for status reporting but silently skipped during stdenv assembly.

3. **GNU flag compatibility is mandatory.** Drop-in replacements must accept the same flags as the originals. Tools like ripgrep and fd, while excellent, are not flag-compatible and cannot serve as stdenv replacements without a compatibility shim.

---

## Phase 1 — Drop-in Available Components

**Goal:** Wire in Rust tools that already exist and are packaged in nixpkgs.

**Status:** ✅ Complete

### Components

| Tool | Replacement | Notes |
|------|-------------|-------|
| bash | rust-bash (`pkgs.rust-bash`) | Bash-compatible shell written in Rust; provides `/bin/bash` and `/bin/sh` |
| coreutils | uutils (`pkgs.uutils-coreutils-noprefix`) | Cross-platform coreutils rewrite; noprefix variant matches GNU binary names |

### Build Tests (all passing)

| Package | Version | Build System | Notes |
|---------|---------|-------------|-------|
| GNU hello | 2.12.1 | autotools | Trivial autotools package |
| zlib | 1.3.1 | configure+make | Critical C library |
| GNU patch | 2.8 | autotools | Patch application tool |
| GNU coreutils | 9.6 | autotools | 106 programs |
| GNU grep | 3.11 | autotools | Regex search tool |
| GNU sed | 4.9 | autotools | Stream editor |
| GNU diffutils | 3.10 | autotools | diff/cmp/sdiff/diff3 |
| GNU make | 4.4.1 | autotools | Built with rust-make (self-referential!) |

### Tasks

- [x] Declare shell component with rust-bash replacement
- [x] Declare coreutils component with uutils replacement
- [x] Test: build a trivial derivation with the partially-oxidized stdenv
- [x] Test: build a real autotools package (e.g. hello) with the partially-oxidized stdenv — **GNU hello builds and runs successfully**
- [x] Document known incompatibilities and workarounds
- [x] Validate rust-bash can execute stdenv's `setup.sh` phases without modification — **setup.sh loads successfully**, all 63 functions defined including `genericBuild` and all build phase functions

### Known Incompatibilities

1. **rust-bash as stdenv shell is experimental** — rust-bash can now source and execute `setup.sh` (all functions load correctly), but running full builds as the shell requires further testing. Key features implemented: `[[ ]]`, arrays, `(( ))`, nameref, indirect expansion, process substitution, `exec {fd}<`, FUNCNAME, local variable scoping.

2. **`allowedRequisites` must be disabled** — Rust replacement packages are built with the normal C stdenv, so their closures transitively reference the C originals. We set `allowedRequisites = null` to bypass this. A fully bootstrapped Rust stdenv (Phase 7) would rebuild replacements with themselves.

3. **uutils-diffutils is not a drop-in** — Only provides a single `diffutils` binary, not the individual `diff`, `cmp`, `sdiff`, `diff3` commands.

4. **patchelf and strip are not in `initialPath`** — Used by fixup hooks separately, not via `initialPath` replacement.

### Risks

- **rust-bash compatibility:** rust-bash is still maturing; some bash-isms in `setup.sh` or configure scripts may fail. Build-time usage may hit edge cases not covered by interactive use.
- **uutils coverage:** uutils implements most but not all GNU coreutils. Missing or subtly different behavior (e.g. `sort --version-sort`, `date` format strings) could break packages.

---

## Phase 2 — Text Processing & Search

**Goal:** Replace the text processing toolkit used pervasively by configure scripts, Makefiles, and stdenv hooks.

**Status:** ✅ Complete

### Components

| Tool | Replacement | Source | Notes |
|------|-------------|--------|-------|
| gnused | rust-sed | repo | Full GNU sed replacement — supports all delimiters (including &), BRE/ERE, in-place editing, hold space, N command, branch/label. Replaces uutils-sed which had critical `&` delimiter bug. |
| gnugrep | rust/grep | repo | GNU-flag-compatible with BRE/ERE/PCRE, -w, -c, -l, -L, context |
| gawk | rust/awk | repo | Lexer/parser/interpreter with POSIX awk + GNU extensions |
| findutils | uutils-findutils | nixpkgs | From [uutils/findutils](https://github.com/uutils/findutils), runs GNU testsuite |
| diffutils | rust-diffutils | repo | Myers diff algorithm with normal/unified/context/ed/rcs output, diff/cmp/sdiff/diff3 via argv[0] detection |

### Testing Strategy

- Differential testing: run original tool and Rust replacement on the same input, compare outputs
- Configure script corpus: collect configure scripts from the top 100 nixpkgs packages, verify they produce identical results
- stdenv hook tests: verify `substituteInPlace`, `fixupPhase`, and other sed/grep-heavy hooks work

---

## Phase 3 — Archive & Compression

**Goal:** Replace the archive and compression tools used to unpack source tarballs.

**Status:** ✅ Complete

### Components

| Tool | Replacement | Rust Foundation |
|------|-------------|-----------------|
| gnutar | rust/tar | `tar` crate — GNU-compatible CLI wrapper |
| gzip | rust/gzip | `flate2` crate — gzip/gunzip/zcat |
| bzip2 | rust/bzip2 | `bzip2` crate — bzip2/bunzip2/bzcat |
| xz | rust/xz | `xz2` crate — xz/unxz/xzcat/lzma/unlzma/lzcat |

### Testing Strategy

- Round-trip testing: compress with GNU tool, decompress with Rust tool (and vice versa)
- Archive format testing: verify all tar formats (ustar, pax, GNU long names) are handled
- Nixpkgs source corpus: unpack the top 200 source tarballs from nixpkgs with both toolchains, diff the results

---

## Phase 4 — Build System

**Goal:** Replace the build system driver (make) and patch application tool.

**Status:** ✅ Complete

### Components

| Tool | Replacement | Status |
|------|-------------|--------|
| gnumake | rust/make | ✅ Available — suffix rules, pattern rules, recursive variables, nested expansion, continuation lines. Successfully builds GNU coreutils (106 programs). |
| gnupatch | rust/patch | ✅ Available — unified/context/normal diff with fuzz matching |

### Testing Strategy

- Patch application testing on the full set of nixpkgs patches
- Differential testing on Makefile corpora from nixpkgs packages (when rust-make is attempted)

---

## Phase 5 — Binary Fixup

**Goal:** Replace the ELF manipulation tools used by Nix's fixup phase.

**Status:** ✅ Complete

### Components

| Tool | Replacement | Rust Foundation |
|------|-------------|-----------------|
| patchelf | rust/patchelf | `goblin` crate for ELF parsing/writing |
| strip (binutils) | rust/strip | `object` crate for ELF section manipulation |

### Testing Strategy

- ELF corpus testing: strip/patchelf on binaries from a range of compilers and languages
- Round-trip validation: patchelf output must be loadable by `ld-linux.so`
- Closure size comparison: verify stripped binaries have equivalent size

---

## Phase 6 — mkDerivation

**Goal:** Replace the shell-script-based build phase orchestration with a Rust binary.

**Status:** ⏳ Planned

### Current Architecture

Today, `mkDerivation` works by:

1. Setting up a bash environment with all `buildInputs` on `$PATH`
2. Sourcing `setup.sh` which defines phase functions
3. Running the `genericBuild` function which calls phases in order:
   `unpackPhase → patchPhase → configurePhase → buildPhase → checkPhase → installPhase → fixupPhase → installCheckPhase`
4. Each phase is a bash function that can be overridden

### Rust Replacement Design

A `rust-mkderivation` binary would:

1. Read build configuration from environment variables (same as today)
2. Execute each phase as a structured step (not a bash function)
3. Invoke build commands directly (e.g. `./configure`, `make`, `make install`)
4. Apply patches, run fixup, strip binaries — all using Rust implementations
5. Provide hook points for custom pre/post phase scripts

**Key benefits:**

- Faster phase execution (no bash interpretation overhead for orchestration)
- Better error messages with structured context
- Type-safe hook system instead of fragile shell function overrides
- Parallel phase execution where dependencies allow

**Compatibility:** The Rust builder must support a `bash` fallback for packages with custom phase overrides (the vast majority of packages use standard phases).

### Deliverables

- [ ] `rust-mkderivation` binary at repo root (or `crates/mkderivation` within this project)
- [ ] Phase executor with all standard phases
- [ ] Hook system (setup-hooks, pre/post phase hooks)
- [ ] Nix `mkDerivationRs` function that uses the Rust builder
- [ ] Compatibility layer for packages with shell-based phase overrides

---

## Phase 7 — Full Oxidized stdenv

**Goal:** Assemble a complete stdenv where every tool in the initial path is written in Rust.

**Status:** ⏳ Planned

### Assembly

```nix
stdenvRs = pkgs.stdenv.override {
  initialPath = [
    rust-bash              # Phase 1
    uutils-coreutils       # Phase 1
    uutils-sed             # Phase 2 (from uutils project)
    rust-grep              # Phase 2
    rust-awk               # Phase 2
    rust-findutils         # Phase 2
    rust-diffutils         # Phase 2
    rust-tar               # Phase 3
    rust-gzip              # Phase 3
    rust-bzip2             # Phase 3
    rust-xz                # Phase 3
    rust-make              # Phase 4
    rust-patch             # Phase 4
  ];
  shell = "${rust-bash}/bin/bash";
};
```

### Validation

1. **Self-build test:** Can the Rust stdenv build itself? (bootstrap)
2. **Mass rebuild:** Build the top 500 nixpkgs packages with the Rust stdenv
3. **Closure comparison:** Compare closure sizes between C stdenv and Rust stdenv builds
4. **Performance benchmarks:** Build time comparison on representative packages

### Integration with rust-nixos

When combined with rust-nixos, this achieves a fully oxidized Linux system:

- **Build time** (rust-nixpkgs): packages are built with Rust tools
- **Run time** (rust-nixos): the running system uses Rust init, shell, coreutils, sudo

---

## Phase 8 — Ecosystem & Polish

**Goal:** Upstream integration, documentation, and ecosystem tooling.

**Status:** ⏳ Planned

### Tasks

- [ ] Contribute stdenv overlay to nixpkgs as an opt-in alternative stdenv
- [ ] CI pipeline: automated mass rebuild with the Rust stdenv
- [ ] Compatibility database: track which nixpkgs packages build successfully
- [ ] Performance dashboard: build time comparisons
- [ ] Documentation: migration guide for package maintainers
- [ ] Upstream patches to uutils and other community projects for issues found during integration

---

## Adding a New Component

All Rust rewrites live under `rust/` in the monorepo (e.g. `rust/sed`, `rust/grep`).
Do **not** use `-rs` suffixes in directory names — the `rust/` prefix is sufficient.
Do **not** touch bash or make replacements yet.

When creating a new Rust rewrite (e.g. `rust/sed`):

1. **Create the subproject** under `rust/`:

   ```text
   rust/sed/
   ├── Cargo.toml
   ├── Cargo.lock
   ├── src/
   │   └── main.rs
   ├── default.nix      # Package definition (flakelight module)
   └── justfile
   ```

2. **Wire it into rust-nixpkgs** by updating the component file:

   ```nix
   # rust/nixpkgs/components/sed.nix
   mkComponent {
     name = "gnused";
     original = pkgs.gnused;
     replacement = pkgs.uutils-sed;  # ← from the uutils project
     status = status.available;     # ← update status
     source = source.repo;
     phase = 2;
     ...
   }
   ```

3. **Import the subproject** in the root `flake.nix`:

   ```nix
   imports = [
     ...
     ./rust/sed
   ];
   ```

   Commitlint scopes are auto-derived from directory structure, so `rust/sed`
   is automatically available as a scope.

4. **Test** with `just test` in `rust/nixpkgs/`.

---

## Related Projects

| Project | Relationship |
|---------|-------------|
| [rust-nixos](../nixos) | Runtime replacement — swaps C tools in a running NixOS system |
| [rust-systemd](../systemd) | Rust systemd replacement — PID 1, journald, networkd, etc. |
| [rust-pkg-config](../pkg-config) | Rust pkg-config replacement — already used in the build toolchain |

Together, these projects work toward a fully oxidized Linux system built and run entirely with Rust user space tools.