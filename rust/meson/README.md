# rust-meson

A drop-in [Meson](https://mesonbuild.com/) build system replacement written in Rust, inspired by [muon](https://github.com/muon-build/muon).

**Version:** 0.1.0 · **License:** MIT · **Binary:** `meson`

## Architecture

Unlike AST-walking interpreters, rust-meson uses a bytecode compiler and stack-based VM:

```text
meson.build → Lexer → Parser → AST → Compiler → Bytecode → VM → Build Graph → Backend → build.ninja
```

## Features

- **Full CLI:** `setup`, `compile`, `test`, `install`, `introspect`, `init`, `dist`, `format`
- **Bytecode VM:** compiled execution rather than AST interpretation
- **Compiler detection:** automatic discovery of C/C++ and other compilers
- **Dependency finding:** pkg-config, CMake, config-tool, and system methods
- **Ninja backend:** generates `build.ninja` files
- **Options system:** built-in and project-specific options
- **Wrap/subproject support:** dependency vendoring and subproject management
- **Cross-compilation:** cross-file support
- **18+ modules:** fs, pkgconfig, python, gnome, cmake, rust, windows, i18n, qt, sourceset, keyval, wayland, cuda, hotdoc, java, dlang, simd, icestorm, external_project, modtest

## Source Layout

```text
src/
├── main.rs            # Entry point
├── cli.rs             # CLI argument parsing
├── lexer.rs           # Tokenizer
├── parser.rs          # Parser
├── ast.rs             # AST types
├── compiler.rs        # Bytecode compiler
├── vm.rs              # Stack-based virtual machine
├── interpreter.rs     # High-level interpretation logic
├── backend.rs         # Ninja backend
├── objects.rs         # Meson object types
├── options.rs         # Build options handling
├── compilers.rs       # Compiler detection
├── dependencies.rs    # Dependency resolution
├── wrap.rs            # Wrap/subproject support
├── builtins/
│   ├── functions.rs   # Built-in functions
│   └── methods.rs     # Built-in methods
└── modules/           # 21 modules (fs, pkgconfig, python, gnome, cmake, ...)
```

## Dependencies

serde, serde_json, toml, glob, regex-lite, sha2

## Building

```sh
cargo build --release
```

Or with Nix:

```sh
nix build .#rust-meson
```

## Tests

284/284 upstream Meson test cases passing (`test cases/common/`), each exposed as an individual Nix check:

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-meson-test-1-trivial

# Run all tests
nix flake show --json | jq -r '.checks."x86_64-linux" | keys[] | select(startswith("rust-meson-test-"))' \
  | xargs -I{} nix build --keep-going --no-link '.#checks.x86_64-linux."{}"'
```
