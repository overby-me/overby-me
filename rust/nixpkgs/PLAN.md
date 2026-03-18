# Implementation Plan

A phased roadmap for replacing nixpkgs stdenv's C-based tools with Rust rewrites. Each phase builds on the previous, expanding the "oxidized surface" of the build toolchain while maintaining full backward compatibility with existing nixpkgs derivations.

## Overview

The nixpkgs standard environment (`stdenv`) is the foundation that builds every package in Nix. It consists of ~15 C-based tools (bash, coreutils, make, grep, sed, etc.) plus a shell-script orchestration layer (`setup.sh` / `mkDerivation`). This plan replaces each component with a Rust equivalent — either an existing community rewrite or a new repo-root subproject.

### Architecture

```text
┌─────────────────────────────────────────────────────┐
│                   mkDerivation                       │
│  (Phase 6: Rust binary replaces setup.sh phases)     │
├─────────────────────────────────────────────────────┤
│              stdenv.initialPath                       │
│                                                       │
│  ┌────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐  │
│  │ shell  │ │coreutils │ │  make    │ │ tar+gz+  │  │
│  │ (P1)   │ │  (P1)    │ │  (P4)   │ │ bz2+xz   │  │
│  │ brush  │ │ uutils   │ │ make-rs │ │  (P3)    │  │
│  └────────┘ └──────────┘ └──────────┘ └──────────┘  │
│  ┌────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐  │
│  │  sed   │ │  grep    │ │  awk    │ │diffutils │  │
│  │ (P2)   │ │  (P2)    │ │  (P2)   │ │  (P2)    │  │
│  └────────┘ └──────────┘ └──────────┘ └──────────┘  │
│  ┌────────┐ ┌──────────┐ ┌──────────┐               │
│  │  find  │ │ patch    │ │patchelf │               │
│  │ xargs  │ │  (P4)    │ │ strip   │               │
│  │ (P2)   │ │         │ │  (P5)   │               │
│  └────────┘ └──────────┘ └──────────┘               │
├─────────────────────────────────────────────────────┤
│              Nix abstractions (rust-nixpkgs)            │
│  components/*.nix │ stdenv.nix │ lib.nix │ tests     │
└─────────────────────────────────────────────────────┘
```

### Component Inventory

| Component | Original | Rust Replacement | Source | Phase | Status |
|-----------|----------|-----------------|--------|-------|--------|
| Shell | bash | [brush](https://github.com/reubeno/brush) | nixpkgs | 1 | ✅ Available |
| Core utilities | coreutils | [uutils](https://github.com/uutils/coreutils) | nixpkgs | 1 | ✅ Available |
| Stream editor | gnused | sed-rs (future) | repo | 2 | ⏳ Planned |
| Pattern grep | gnugrep | grep-rs (future) | repo | 2 | ⏳ Planned |
| Awk | gawk | awk-rs (future) | repo | 2 | ⏳ Planned |
| File search | findutils | findutils-rs (future) | repo | 2 | ⏳ Planned |
| Diff | diffutils | diffutils-rs (future) | repo | 2 | ⏳ Planned |
| Tar archive | gnutar | tar-rs (future) | repo | 3 | ⏳ Planned |
| Gzip | gzip | gzip-rs (future) | repo | 3 | ⏳ Planned |
| Bzip2 | bzip2 | bzip2-rs (future) | repo | 3 | ⏳ Planned |
| XZ/LZMA | xz | xz-rs (future) | repo | 3 | ⏳ Planned |
| Build driver | gnumake | make-rs (future) | repo | 4 | ⏳ Planned |
| Patch | gnupatch | patch-rs (future) | repo | 4 | ⏳ Planned |
| ELF patcher | patchelf | patchelf-rs (future) | repo | 5 | ⏳ Planned |
| Symbol strip | binutils (strip) | strip-rs (future) | repo | 5 | ⏳ Planned |
| Build phases | setup.sh | mkderivation-rs (future) | repo | 6 | ⏳ Planned |

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

1. **Component files, not crate stubs.** Each `components/*.nix` file declares the replacement mapping. Actual Rust rewrites live at the monorepo root (e.g. `../make-rs/`) or come from nixpkgs (e.g. `pkgs.uutils-coreutils-noprefix`).

2. **Null means "not yet available."** Components with `replacement = null` are tracked in the registry for status reporting but silently skipped during stdenv assembly.

3. **GNU flag compatibility is mandatory.** Drop-in replacements must accept the same flags as the originals. Tools like ripgrep and fd, while excellent, are not flag-compatible and cannot serve as stdenv replacements without a compatibility shim.

---

## Phase 1 — Drop-in Available Components

**Goal:** Wire in Rust tools that already exist and are packaged in nixpkgs.

**Status:** 🔶 In progress

### Components

| Tool | Replacement | Notes |
|------|-------------|-------|
| bash | brush (`pkgs.brush`) | Bash-compatible Rust shell; already tested as NixOS runtime shell in rust-nixos |
| coreutils | uutils (`pkgs.uutils-coreutils-noprefix`) | Cross-platform coreutils rewrite; noprefix variant matches GNU binary names |

### Tasks

- [x] Declare shell component with brush replacement
- [x] Declare coreutils component with uutils replacement
- [ ] Test: build a trivial derivation with the partially-oxidized stdenv
- [ ] Test: build a real autotools package (e.g. hello) with the partially-oxidized stdenv
- [ ] Document known incompatibilities and workarounds
- [ ] Validate brush can execute stdenv's `setup.sh` phases without modification

### Risks

- **brush compatibility:** brush is still maturing; some bash-isms in `setup.sh` or configure scripts may fail. The rust-nixos wrapper handles signal setup for interactive use, but build-time usage may hit different edge cases.
- **uutils coverage:** uutils implements most but not all GNU coreutils. Missing or subtly different behavior (e.g. `sort --version-sort`, `date` format strings) could break packages.

---

## Phase 2 — Text Processing & Search

**Goal:** Replace the text processing toolkit used pervasively by configure scripts, Makefiles, and stdenv hooks.

**Status:** ⏳ Planned

### Components

| Tool | Replacement | Priority | Difficulty |
|------|-------------|----------|------------|
| gnused | sed-rs | Critical | High — full GNU sed with -i, -E, -z, address ranges, branch/label commands |
| gnugrep | grep-rs | Critical | High — must support -P (PCRE), -w, -c, -l, -L, -Z, BRE/ERE, recursive |
| gawk | awk-rs | High | High — POSIX awk + GNU extensions (gensub, nextfile, BEGINFILE/ENDFILE) |
| findutils | findutils-rs | High | Medium — find + xargs with full predicate/action support |
| diffutils | diffutils-rs | Medium | Medium — diff + cmp (used by configure scripts) |

### Approach

Each tool becomes a **repo-root subproject** (e.g. `../sed-rs/`, `../grep-rs/`). The Nix packaging follows the same pattern as `rust-pkg-config`:

```nix
# In sed-rs/default.nix
packages.sed-rs = { lib, rustPlatform }: rustPlatform.buildRustPackage { ... };
```

Then in `rust-nixpkgs/components/sed.nix`:

```nix
replacement = pkgs.sed-rs;
```

### Priority Ordering

1. **sed** — most critical; used by `substituteInPlace`, configure scripts, stdenv hooks
2. **grep** — used by virtually every configure script
3. **find/xargs** — used by stdenv fixup phases and many build systems
4. **awk** — used by configure scripts and some Makefiles
5. **diff/cmp** — used by configure scripts for feature detection

### Testing Strategy

- Differential testing: run original tool and Rust replacement on the same input, compare outputs
- Configure script corpus: collect configure scripts from the top 100 nixpkgs packages, verify they produce identical results
- stdenv hook tests: verify `substituteInPlace`, `fixupPhase`, and other sed/grep-heavy hooks work

---

## Phase 3 — Archive & Compression

**Goal:** Replace the archive and compression tools used to unpack source tarballs.

**Status:** ⏳ Planned

### Components

| Tool | Replacement | Rust Foundation |
|------|-------------|-----------------|
| gnutar | tar-rs | `tar` crate (pure Rust tar library) |
| gzip | gzip-rs | `flate2` crate (DEFLATE/gzip codec) |
| bzip2 | bzip2-rs | `bzip2-rs` by Trifecta Tech Foundation |
| xz | xz-rs | `liblzma-rs` / `xz2` crate |

### Approach

Each compression tool is a thin CLI wrapper around a mature Rust codec crate:

- **tar-rs**: Wraps the `tar` crate with GNU tar CLI flags (`-xf`, `-czf`, `--strip-components`, `--transform`, `--owner`, `--group`, `--sort`, `--exclude`)
- **gzip-rs**: Wraps `flate2` with GNU gzip flags (`-d`, `-c`, `-k`, `-1..9`, `-f`, `-n`, `-N`, `-r`)
- **bzip2-rs**: Wraps the Trifecta `bzip2-rs` crate with bzip2 CLI flags
- **xz-rs**: Wraps `liblzma-rs` with xz flags (`-d`, `-k`, `-f`, `-c`, `-z`, `-T`, levels)

### Testing Strategy

- Round-trip testing: compress with GNU tool, decompress with Rust tool (and vice versa)
- Archive format testing: verify all tar formats (ustar, pax, GNU long names) are handled
- Nixpkgs source corpus: unpack the top 200 source tarballs from nixpkgs with both toolchains, diff the results

---

## Phase 4 — Build System

**Goal:** Replace the build system driver (make) and patch application tool.

**Status:** ⏳ Planned

### Components

| Tool | Replacement | Difficulty |
|------|-------------|------------|
| gnumake | make-rs | Very high — GNU Make has a complex, poorly-specified language |
| gnupatch | patch-rs | Medium — unified/context diff application with fuzz matching |

### GNU Make Replacement Strategy

GNU Make is the most complex tool to replace. The language has many subtle semantics (recursive vs. simple variables, secondary expansion, order-only prerequisites, `$(eval)`, `$(call)`, VPATH, implicit rules, pattern rules, static pattern rules, etc.).

**Approach options:**

1. **Full reimplementation** — implement the GNU Make language from scratch in Rust. Most correct but extremely labor-intensive.
2. **Makefile subset** — implement only the subset of GNU Make used by 95%+ of nixpkgs packages (autotools-generated Makefiles follow predictable patterns).
3. **Compatibility shim** — parse and re-emit Makefiles in a simplified form, delegate to a simple make engine.

Recommended: **Option 2** — a subset implementation targeting autotools-generated Makefiles, with explicit error messages for unsupported features.

### Testing Strategy

- Build the autotools "hello" package end-to-end with make-rs
- Differential testing on Makefile corpora from nixpkgs packages
- Patch application testing on the full set of nixpkgs patches

---

## Phase 5 — Binary Fixup

**Goal:** Replace the ELF manipulation tools used by Nix's fixup phase.

**Status:** ⏳ Planned

### Components

| Tool | Replacement | Rust Foundation |
|------|-------------|-----------------|
| patchelf | patchelf-rs | `object` / `goblin` crate for ELF parsing |
| strip (binutils) | strip-rs | `object` crate for ELF section manipulation |

### patchelf-rs

This is a **critical** Nix-specific tool. Every derivation producing ELF binaries relies on patchelf to:

- Rewrite the dynamic linker path (`--set-interpreter`)
- Set RPATH/RUNPATH to exact store paths (`--set-rpath`, `--shrink-rpath`)
- Modify shared library metadata (`--set-soname`, `--add-needed`, `--remove-needed`)

The `goblin` and `object` crates provide ELF parsing, but patchelf's ability to **rewrite** ELF binaries while preserving correctness (alignment, section offsets, program headers) requires careful implementation.

### strip-rs

GNU strip from binutils removes symbol tables and debug sections from ELF files. The stdenv fixup phase runs strip on all binaries and libraries to reduce closure size. A Rust replacement using the `object` crate would:

- Remove `.symtab`, `.strtab`, debug sections
- Optionally preserve `.gnu_debuglink`
- Handle static archives (`.a` files containing `.o` members)

### Testing Strategy

- ELF corpus testing: strip/patchelf on binaries from a range of compilers and languages
- Round-trip validation: patchelf-rs output must be loadable by `ld-linux.so`
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

A `mkderivation-rs` binary would:

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

- [ ] `mkderivation-rs` binary at repo root (or `crates/mkderivation` within this project)
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
    brush                  # Phase 1
    uutils-coreutils       # Phase 1
    sed-rs                 # Phase 2
    grep-rs                # Phase 2
    awk-rs                 # Phase 2
    findutils-rs           # Phase 2
    diffutils-rs           # Phase 2
    tar-rs                 # Phase 3
    gzip-rs                # Phase 3
    bzip2-rs               # Phase 3
    xz-rs                  # Phase 3
    make-rs                # Phase 4
    patch-rs               # Phase 4
  ];
  shell = "${brush}/bin/brush";
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
- [ ] Upstream patches to brush, uutils, and other community projects for issues found during integration

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
     replacement = pkgs.rust-sed;  # ← point to the new package
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