# Plan: Add upstream binutils tests as Nix checks

## Goal

Run tests from the upstream GNU binutils test suite against `rust-binutils`, comparing output with reference GNU binutils, similar to how `rust/awk` runs the gawk test suite.

## Architecture (modeled after `rust/awk`)

### 1. `testsuite.nix` ✅

A Nix derivation that runs a single named test, comparing `rust-binutils` output against reference GNU `binutils` output. Takes `{ pkgs, tool, name }` as arguments.

Key design decisions:

- **Two-level naming**: tests are keyed by `tool` + `name` (e.g., `nm/basic`, `strings/object`)
- **Tool name mapping**: `cxxfilt` → `c++filt` binary name handled in Nix
- **Shared test object**: `bintest.s` from upstream is assembled once per test via `gcc -c`
- **Output normalization**: nix store paths, binary names, and trailing whitespace are normalized before diffing
- **Standalone test scripts**: each test is a shell script in `tests/${tool}/${name}.sh` that uses `$REF`, `$RUST`, `$TESTOBJ`, `$TMPDIR`, and the `compare` helper

### 2. `default.nix` ✅

- Added `rust-binutils-dev` package (debug build for fast iteration)
- Added `checks` attribute set wiring all test definitions to `testsuite.nix`

### 3. Test results (14/14 passing)

| Check name | Tool | Status | Issue |
|---|---|---|---|
| `strings-basic` | strings | ✅ PASS | — |
| `strings-object` | strings | ✅ PASS | — |
| `ar-create-list` | ar | ✅ PASS | — |
| `nm-basic` | nm | ✅ PASS | — |
| `nm-extern-only` | nm | ✅ PASS | — |
| `size-basic` | size | ✅ PASS | — |
| `size-sysv` | size | ✅ PASS | — |
| `cxxfilt-basic` | c++filt | ✅ PASS | — |
| `cxxfilt-multiple` | c++filt | ✅ PASS | — |
| `readelf-file-header` | readelf | ✅ PASS | — |
| `readelf-sections` | readelf | ✅ PASS | — |
| `objdump-headers` | objdump | ✅ PASS | — |
| `objdump-disassemble` | objdump | ✅ PASS | — |
| `addr2line-basic` | addr2line | ✅ PASS | — |

### 4. Fixes applied

- **nm symbol type classification** — look up actual ELF section by index; classify by section name and flags (`SHF_EXECINSTR` → `t`, `SHF_WRITE` → `d`, `SHT_NOBITS` → `b`, read-only alloc → `r`)
- **c++filt demangling** — replaced hand-rolled parser with `cpp_demangle` crate for full Itanium ABI support
- **size Berkeley format** — classify sections by ELF flags (`SHF_ALLOC`, `SHF_WRITE`, `SHT_NOBITS`) instead of section name; bss defaults to 0
- **size SysV format** — filter to `SHF_ALLOC` sections only; match GNU column widths (`{:<20}{:>5}{:>7}`)
- **addr2line output** — output `??:?` (not `??:0`) for unknown addresses; only print function name line with `-f`
- **readelf `-h` fix** — don't treat `-h` as `--help`; inline version/help check so `-h` means `--file-header`
- **readelf `-S` format** — two-line per section format matching GNU layout; intro line with section count and offset; full Key to Flags legend
- **objdump `-h` fix** — same `-h` → `--help` bug as readelf; filter to `SHF_ALLOC` sections; add flag description lines (`CONTENTS, ALLOC, LOAD, ...`); correct file offsets via raw ELF headers; proper alignment powers; machine-specific format name (`elf64-x86-64`)
- **objdump `-d` disassembly** — added `iced-x86` crate for proper x86/x64 instruction decoding with AT&T syntax (`GasFormatter`); symbol labels at function boundaries; zero-byte run collapsing with `...` ellipsis matching GNU behavior; per-section symbol filtering for correct labels
- **testsuite normalization** — strip trailing whitespace in compare function to avoid false failures

### 5. Execution

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-binutils-test-strings-basic

# Run all binutils tests
nix flake check
```

## Completed steps

- [x] Add `rust-binutils-dev` package to `default.nix` (debug build for fast iteration)
- [x] Create `testsuite.nix` with the test runner derivation
- [x] Create test scripts for `strings`, `nm`, `size`, `cxxfilt`, `readelf`, `objdump`, `ar`, `addr2line`
- [x] Add test names to `checks` in `default.nix`
- [x] Run all tests and catalog results
- [x] Fix nm symbol type classification (`?` → proper type chars)
- [x] Fix c++filt demangling (add `cpp_demangle` crate)
- [x] Fix size Berkeley and SysV format
- [x] Fix addr2line `??:?` output
- [x] Fix readelf/objdump `-h` flag handling and output formatting
- [x] Fix objdump `-h` section headers (flags, offsets, alignment, format name)
- [x] Fix readelf `-S` section headers (two-line format, intro, full flags key)

## Next steps

- [ ] Add more tests as tools improve (e.g., `strip`, `objcopy`, `ranlib`)
- [ ] Test with linked executables and shared libraries (not just `.o` files)
- [ ] Add edge case tests (empty files, corrupted ELF, archives with many members)