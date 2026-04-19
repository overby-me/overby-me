# Rust Meson

Drop-in Meson build system replacement in Rust, inspired by [muon](https://github.com/muon-build/muon).

## Architecture

Bytecode compiler + stack VM (not AST-walking):

    meson.build > Lexer > Parser > AST > Compiler > Bytecode > VM > Build Graph > Backend > build.ninja

## Test Suite

Each upstream meson test case (test cases/common/, ~285 tests) is an individual
nix check. Tests are self-validating via assert() in meson.build.

    nix build .#checks.x86_64-linux.rust-meson-test-{number}-{slug}
    nix build .#checks.x86_64-linux.rust-meson-test-* --keep-going --no-link

**Status: 253/285 passing (89%)**
