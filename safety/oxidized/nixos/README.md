# rust-nixos

NixOS with a Rust user space.

## Overview

`rust-nixos` is a NixOS configuration that systematically replaces core C user space components with Rust alternatives using NixOS's `system.replaceDependencies.replacements` mechanism. The result is a bootable NixOS system where the init system, shell, privilege escalation, and core utilities are all written in Rust.

This serves as both a proof-of-concept for an oxidized Linux distribution and as an integration test bed for Rust system components — particularly [rust-systemd](../systemd), which replaces PID 1 and the entire systemd suite.

## Replacements

| Component | C Original | Rust Replacement | Module | Status |
|-----------|-----------|-----------------|--------|--------|
| Init / service manager | [systemd](https://github.com/systemd/systemd) | [rust-systemd](../systemd) | [`systemd.nix`](systemd.nix) | ✅ Active |
| Privilege escalation | [sudo](https://www.sudo.ws/) | [sudo-rs](https://github.com/trifectatechfoundation/sudo-rs) | [`sudo.nix`](sudo.nix) | ✅ Active |
| Shell | [bash](https://www.gnu.org/software/bash/) | [brush](https://github.com/reubeno/brush) | [`bash.nix`](bash.nix) | 🚧 Experimental |
| Core utilities | [coreutils](https://www.gnu.org/software/coreutils/) | [uutils](https://github.com/uutils/coreutils) | [`coreutils.nix`](coreutils.nix) | 🚧 Experimental |

Modules marked **Active** are enabled in the default `rust-nixos` configuration. **Experimental** modules are available but commented out in [`default.nix`](default.nix) pending further integration work.

## How It Works

NixOS's `system.replaceDependencies.replacements` performs a closure-wide substitution — every package in the system closure that depends on the original package gets rebuilt (or binary-patched) to reference the replacement instead. This means the swap is not just surface-level; the entire dependency graph is rewritten.

For example, `systemd.nix` sets `systemd.package = pkgs.rust-systemd-systemd`, which is a wrapper package that starts from the real systemd store path (to get unit files, udev rules, tmpfiles configs, etc.) and overlays the Rust binaries on top. The result is a package that is layout-compatible with systemd but runs Rust code.

The `bash.nix` module is more involved — it builds a small C wrapper that translates bash's CLI conventions (single-character flags like `-eu`) into brush's option syntax, handles signal setup for serial consoles, and `execv`s into brush.

## Configurations

| Configuration | Description |
|--------------|-------------|
| `nixos-nix` | Vanilla NixOS baseline (no Rust replacements) |
| `rust-nixos` | NixOS with Rust user space (active replacements enabled) |

Both configurations share [`base.nix`](base.nix), which sets up a QEMU guest with systemd-networkd, systemd-resolved, and an auto-login user.

## Usage

### Prerequisites

- [Nix](https://nixos.org/) with flakes enabled
- [just](https://github.com/casey/just) (available in the dev shell)
- [cloud-hypervisor](https://www.cloudhypervisor.org/) for VM execution
- A TAP network device (`vmtap0`) for VM networking, or `sudo` access for automatic setup

### Build & Run

```shell
# Build the disk image
just build

# Boot the VM interactively (serial console)
just run
```

### Testing

```shell
# Run automated boot test (checks for login prompt, detects panics)
just test

# Boot test with custom timeout
just test-timeout 60

# Boot test with log file
just test-log boot.log

# Quick pass/fail (no streaming output)
just test-quiet

# Boot test then keep VM running for debugging
just test-keep
```

The [`test-boot.sh`](test-boot.sh) script launches the VM in cloud-hypervisor, captures serial output, and checks for success patterns (login prompt) and failure patterns (kernel panic, Rust panics, emergency mode). It handles network setup automatically in CI environments.

### Analysis

```shell
# Compare closure sizes between vanilla and oxidized NixOS
just compare-closure

# Diff package lists between the two configurations
just compare-packages

# Explore the dependency tree
just tree
```

## Project Structure

```text
rust-nixos/
├── default.nix      # Nix entry point: dev shell + NixOS configurations
├── base.nix         # Shared NixOS config (QEMU guest, networkd, resolved, users)
├── systemd.nix      # systemd → rust-systemd replacement
├── sudo.nix         # sudo → sudo-rs replacement
├── bash.nix         # bash → brush replacement (experimental)
├── coreutils.nix    # coreutils → uutils replacement (experimental)
├── justfile         # Build, run, and test commands
└── test-boot.sh     # Automated VM boot test script
```

## Related Projects

- [rust-systemd](../systemd) — The Rust systemd replacement that powers this configuration
- [rust-pkg-config](../pkg-config) — A Rust pkg-config implementation used in the build toolchain