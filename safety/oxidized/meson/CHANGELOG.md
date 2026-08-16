# Changelog

All notable changes to rust-meson are documented in this file.

Format based on [Keep a Changelog](https://keepachangelog.com/).

## [0.1.0] - Unreleased

### Added

- Bytecode compiler and stack-based VM for the meson.build DSL
- All built-in functions: `project`, `executable`, `library`, `dependency`, `test`, `install_*`, etc.
- Type methods for str, list, dict, int, bool, feature, dep, compiler, meson, machine, etc.
- Compiler detection and compile checks (C, C++, Rust, Fortran, D, etc.)
- Dependency finding via pkg-config, CMake, config-tool, and system library probing
- Ninja backend generating `build.ninja` with compile, link, and custom target rules
- Options system with `meson_options.txt` / `meson.options` parsing and version comparison
- Full CLI: `setup`, `compile`, `test`, `install`, `introspect`, `init`, `dist`, `format`
- 18+ modules: fs, pkgconfig, python, gnome, cmake, rust, windows, i18n, qt, sourceset, keyval, wayland, cuda, hotdoc, java, dlang, simd, icestorm, external_project
- Wrap/subproject download, extraction, and nested subproject support
- Cross-compilation and native/cross machine file support
- Nix-based test suite: 284/284 upstream meson test cases (`test cases/common/`) passing
- Hello world end-to-end Nix test (init → setup → build → run)

### Fixed

- Static-aware `meson.override_dependency()` with `static:` kwarg support
- Qualified dependency override table for correct static/shared lookup matching
- Subproject `default_library` inheritance for implicit override tagging
- `unstable-external_project` module `.dependency()` returns synthesized found deps
- `find_pkgconfig()` honors overridden pkg-config from `meson.override_find_program()`
- Shebang interpreter fallback for hermetic build sandboxes (missing `/usr/bin/env`)
- 154 individual test case fixes across subprojects, options, modules, and builtins
