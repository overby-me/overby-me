# rust-nixpkgs

Rust replacements for the C toolchain that builds Nix packages.

## Overview

`rust-nixpkgs` is the build-time counterpart to [rust-nixos](../nixos). Where rust-nixos replaces runtime system components in a running NixOS system, rust-nixpkgs replaces the **build tools** — the stdenv toolchain that Nix uses to compile and package software.

The nixpkgs standard environment (`stdenv`) is built on a stack of C-based GNU tools: bash runs the build scripts, coreutils provides filesystem primitives, make drives compilation, tar/gzip/xz handle source archives, sed/awk/grep do text processing, and patchelf/strip do binary fixup. Every package in nixpkgs is built by this toolchain.

`rust-nixpkgs` provides Nix abstractions — a component registry and stdenv override mechanism — so that each C tool can be incrementally replaced with a Rust drop-in. Individual rewrites live as sibling subprojects under `rust/` (e.g. `rust/grep`, `rust/tar`) or come from existing Rust projects already in nixpkgs (e.g. uutils for coreutils).

## Components

All 15 stdenv tools have Rust replacements available:

| Component | Original | Rust Replacement | Source | Phase |
|-----------|----------|-----------------|--------|-------|
| Shell | bash | [rust-bash](../bash) | repo | 1 |
| Core utilities | coreutils | [uutils](https://github.com/uutils/coreutils) | nixpkgs | 1 |
| Stream editor | gnused | [uutils-sed](https://github.com/uutils/sed) | nixpkgs | 2 |
| Text search | gnugrep | [rust-grep](../grep) | repo | 2 |
| Pattern processing | gawk | [rust-awk](../awk) | repo | 2 |
| File search | findutils | [uutils-findutils](https://github.com/uutils/findutils) | nixpkgs | 2 |
| File comparison | diffutils | [uutils-diffutils](https://github.com/uutils/diffutils) | nixpkgs | 2 |
| Tape archive | gnutar | [rust-tar](../tar) | repo | 3 |
| Gzip compression | gzip | [rust-gzip](../gzip) | repo | 3 |
| Bzip2 compression | bzip2 | [rust-bzip2](../bzip2) | repo | 3 |
| XZ compression | xz | [rust-xz](../xz) | repo | 3 |
| Build driver | gnumake | [rust-make](../make) | repo | 4 |
| Patch application | gnupatch | [rust-patch](../patch) | repo | 4 |
| ELF patching | patchelf | [rust-patchelf](../patchelf) | repo | 5 |

## How It Works

### Component Registry

Each component is declared in `components/*.nix` as a small attrset:

```nix
# components/coreutils.nix
{ pkgs, lib, mkComponent, status, source, ... }:
mkComponent {
  name = "coreutils";
  original = pkgs.coreutils;
  replacement = pkgs.uutils-coreutils-noprefix;
  status = status.available;
  source = source.nixpkgs;
  phase = 1;
  description = "Core file, text, and shell utilities";
  notes = "Using uutils-coreutils-noprefix";
}
```

Components with `replacement = null` are tracked for status reporting but skipped when assembling the stdenv.

### Stdenv Override

The overlay reads the component registry and produces a modified stdenv where every C tool with a Rust replacement is swapped out of the `initialPath`:

```nix
# The overlay provides stdenvRs — a stdenv with Rust tools
pkgs.stdenvRs.mkDerivation {
  pname = "hello";
  # ... this package is built with Rust tools
}
```

### Drop-in Requirement

Replacements must be **flag-compatible drop-ins** for the originals:

- Same binary names (`ls`, `grep`, `make`, not `rg`, `fd`, `just`)
- Same CLI flags (GNU extensions included where configure scripts rely on them)
- Same output format (scripts parse stdout of these tools)
- Same exit codes

## Usage

### Prerequisites

- [Nix](https://nixos.org/) with flakes enabled
- [just](https://github.com/casey/just) (available in the dev shell)

### Commands

```shell
# Enter the dev shell
cd rust/nixpkgs
direnv allow  # or: nix develop .#rust-nixpkgs

# Show component status
just status

# Build the status test derivation
just build

# Test building a simple package with the Rust stdenv
just test

# Compare stdenv initialPath (original vs. Rust)
just compare
```

## Project Structure

```text
rust/nixpkgs/
├── default.nix          # Flakelight entry: devShell, overlay, packages
├── lib.nix              # Component registry helpers (mkComponent, mkRustStdenv)
├── stdenv.nix           # Rust stdenv assembler
├── components/
│   ├── default.nix      # Registry loader (imports all component files)
│   ├── shell.nix        # bash → rust-bash
│   ├── coreutils.nix    # coreutils → uutils
│   ├── sed.nix          # gnused → uutils-sed
│   ├── grep.nix         # gnugrep → rust-grep
│   ├── awk.nix          # gawk → rust-awk
│   ├── findutils.nix    # findutils → uutils-findutils
│   ├── diffutils.nix    # diffutils → uutils-diffutils
│   ├── tar.nix          # gnutar → rust-tar
│   ├── gzip.nix         # gzip → rust-gzip
│   ├── bzip2.nix        # bzip2 → rust-bzip2
│   ├── xz.nix           # xz → rust-xz
│   ├── make.nix         # gnumake → rust-make
│   ├── patch.nix        # gnupatch → rust-patch
│   └── patchelf.nix     # patchelf → rust-patchelf
├── PLAN.md              # Phased replacement roadmap
├── README.md            # This file
└── justfile             # Build, test, and status commands
```

## Related Projects

| Project | Scope |
|---------|-------|
| [rust-nixos](../nixos) | Runtime system — swaps C tools in a running NixOS system |
| [rust-systemd](../systemd) | Rust systemd replacement — PID 1, journald, networkd, etc. |
| [rust-pkg-config](../pkg-config) | Rust pkg-config replacement for the build toolchain |

Together, these projects work toward a fully oxidized Nix ecosystem: packages are **built** with Rust tools (rust-nixpkgs), the resulting **system** runs Rust services (rust-nixos + rust-systemd), and the **build system** itself uses Rust utilities (rust-pkg-config).

## License

See [LICENSE](../LICENSE).
