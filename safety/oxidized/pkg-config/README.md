# rust-pkg-config

A pure Rust rewrite and drop-in replacement for [pkg-config](https://www.freedesktop.org/wiki/Software/pkg-config/) / [pkgconf](https://github.com/pkgconf/pkgconf).

## Overview

`rust-pkg-config` is a fully compatible, single-binary replacement for `pkg-config` and `pkgconf`, written entirely in Rust. It provides:

- **`pkgconf`** — a CLI binary that is a drop-in replacement for both `pkg-config` and `pkgconf`
- **`libpkgconf`** — a Rust library crate providing the core functionality for parsing `.pc` files, resolving dependencies, managing compiler/linker flags, and comparing versions

The implementation is modeled after [pkgconf](https://github.com/pkgconf/pkgconf) (the modern, maintained C implementation),
**not** the legacy freedesktop.org `pkg-config`.
This means we follow pkgconf's architecture for the dependency graph solver,
fragment handling, cross-compilation personality support, and other advanced features.

## Features

- **Full `.pc` file parsing** with variable interpolation, line continuation, and comment handling
- **RPM-style version comparison** (`rpmvercmp` algorithm) with tilde pre-release support
- **Dependency graph resolution** with cycle detection, depth limiting, and diamond dependency deduplication
- **Fragment management** with correct deduplication semantics and system directory filtering
- **Cross-compilation personalities** via triplet-based configuration files
- **MSVC syntax rendering** (`-I` → `/I`, `-L` → `/LIBPATH:`, `-l` → `.lib`)
- **Sysroot handling** with both FDO and pkgconf1 rules
- **Prefix redefinition** for relocatable packages
- **Audit logging** for debugging dependency resolution
- **Full environment variable compatibility** with `pkg-config` and `pkgconf`

## Installation

### From source

```sh
cargo install --path crates/pkgconf
```

## Usage

```sh
# Query cflags for a package
pkgconf --cflags glib-2.0

# Query linker flags
pkgconf --libs zlib

# Check if a package exists with a minimum version
pkgconf --atleast-version=1.2.8 zlib

# Print the version of a package
pkgconf --modversion openssl

# Query a specific variable
pkgconf --variable=libdir openssl

# Static linking
pkgconf --static --libs zlib

# List all known packages
pkgconf --list-all

# Output in MSVC syntax
pkgconf --msvc-syntax --cflags --libs zlib

# Dependency graph visualization (Graphviz dot format)
pkgconf --digraph gtk+-3.0

# Cross-compilation with a personality
pkgconf --personality=x86_64-w64-mingw32 --cflags zlib

# Validate .pc files
pkgconf --validate mylib
```

All standard `pkg-config` and `pkgconf` flags are supported. See `pkgconf --help` for the full list.

## Testing

```sh
# Run all tests (627 tests)
cargo test

# Run library unit tests (406 tests)
cargo test -p libpkgconf

# Run CLI integration tests (142 tests)
cargo test -p pkgconf --test integration

# Run edge case tests (69 tests)
cargo test -p pkgconf --test edge_cases

# Run benchmarks
cargo bench -p libpkgconf

# Differential testing against system pkg-config
cargo build && ./tests/differential.sh --verbose
```
