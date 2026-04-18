# Rust Meson — Test Plan

Drop-in replacement for [Meson](https://github.com/mesonbuild/meson) build system in Rust.
Inspired by [muon](https://github.com/muon-build/muon) C implementation.

## Architecture

Like muon, we use a bytecode compiler + stack VM instead of AST-walking interpreter.

    meson.build > Lexer > Parser > AST > Compiler > Bytecode > VM > Build Graph > Backend > build.ninja

## Test Strategy

### Approach

Following the same pattern as rust/awk, each upstream meson test case becomes an
individual nix check derivation. This gives us:

- **Per-test granularity**: each test is a separate nix build target
- **Hermetic sandboxed execution**: tests run in nix build sandboxes
- **Caching**: passing tests never re-run unless inputs change
- **Parallel execution**: nix build --keep-going runs all tests in parallel

### Test Source

Tests come from **upstream meson source** (pkgs.meson.src), specifically
the test cases/common/ directory, which contains ~285 test cases covering:

- Core language features (variables, operators, control flow)
- String/array/dict operations
- Built-in functions (project, executable, dependency, etc.)
- Compiler detection and probing
- Custom targets and generators
- Subprojects and wraps
- Module system (fs, pkgconfig, python, gnome, etc.)
- Options system
- Install targets

### How Each Test Works

Each test is a nix runCommand derivation that:

1. Extracts the meson source tarball
2. Copies the test case to a writable working directory
3. Runs rust-meson setup builddir from the test case directory
4. Success = meson setup exits 0
5. Tests containing MESON_SKIP_TEST in output are treated as skip (pass)

Meson tests are self-validating: they use assert() statements in meson.build that
cause meson setup to fail with a non-zero exit code if the assertion fails.

### Running Tests

Run a single test:

    nix build .#checks.x86_64-linux.rust-meson-test-{number}-{slug}

View a failing test log:

    nix log .#checks.x86_64-linux.rust-meson-test-{number}-{slug}

Run all tests (parallel, keep going on failures):

    nix build .#checks.x86_64-linux.rust-meson-test-* --keep-going --no-link

### Current Status

Initial test run against test cases/common/: **130/285 passing** (46%%)

#### Key failure categories

| Category | Count | Examples |
|---|---|---|
| is_disabler() builtin missing | ~10 | 1-trivial, 2-cpp, 3-static |
| build_target() builtin missing | ~8 | 4-shared, 5-linkstatic, 89-default-library |
| expect_error testcase syntax | ~12 | 18-includedir, 40-options, 220-fs-module |
| Python module find_python | ~12 | 106, 109, 128, 129, 139, 141 |
| CustomTarget indexing (ct[0]) | ~12 | 105, 208, 209, 210, 216, 226 |
| String / path division | ~6 | 121, 248, 253, 268, 279 |
| Missing compiler methods | ~6 | 118, 119, 127, 132, 133 |
| Subproject resolution | ~10 | 112, 155, 167, 246, 283 |
| subdir_done() missing | 1 | 177 |
| Array/string concat type errors | ~3 | 60, 84, 229 |
| String format @@ escaping | 1 | 35 |
| Feature option methods | ~3 | 192, 193, 196 |
| f-string empty var name | 1 | 237 |
| Other | ~20 | various |

### Implementation Priorities

1. **is_disabler()** - unlocks ~10 tests including trivial, cpp, static
2. **build_target()** - unlocks shared, linkstatic, default-library tests
3. **String / (path join) operator** - unlocks pathjoin and install tests
4. **Array + String flattening in foreach** - unlocks foreach, plusassign tests
5. **CustomTarget indexing** - unlocks many build-system tests
6. **expect_error testcase syntax** - unlocks modern test cases
7. **Python module find_python** - unlocks tests that use Python scripts
8. **subdir_done()** - simple builtin to add
9. **Feature option methods** - enabled/disabled/auto methods on feature
10. **String format @@ escape** - @@0@@ should produce literal @0@

## Phases (Implementation Status)

### Phase 1: Core Language - DONE

### Phase 2: Built-in Functions & Objects - DONE

### Phase 3: Compiler & Dependency Detection - DONE

### Phase 4: Ninja Backend - DONE

### Phase 5: Options System - DONE

### Phase 6: CLI & Commands - DONE

### Phase 7: Modules - DONE

### Phase 8: Polish & Compatibility

- [x] Upstream test suite integration (nix checks)
- [ ] Meson test suite compatibility (130/285 > target 200+)
- [ ] Edge cases and error messages matching Meson
- [ ] Performance optimization
- [ ] Documentation
