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

### 3. Test results (24/24 passing)

| Check name | Tool | Status |
|---|---|---|
| `strings-basic` | strings | ✅ PASS |
| `strings-object` | strings | ✅ PASS |
| `strings-min-length` | strings | ✅ PASS |
| `ar-create-list` | ar | ✅ PASS |
| `ar-extract` | ar | ✅ PASS |
| `nm-basic` | nm | ✅ PASS |
| `nm-extern-only` | nm | ✅ PASS |
| `nm-no-sort` | nm | ✅ PASS |
| `nm-undefined-only` | nm | ✅ PASS |
| `size-basic` | size | ✅ PASS |
| `size-sysv` | size | ✅ PASS |
| `size-totals` | size | ✅ PASS |
| `cxxfilt-basic` | c++filt | ✅ PASS |
| `cxxfilt-multiple` | c++filt | ✅ PASS |
| `cxxfilt-nested` | c++filt | ✅ PASS |
| `readelf-file-header` | readelf | ✅ PASS |
| `readelf-sections` | readelf | ✅ PASS |
| `readelf-program-headers` | readelf | ✅ PASS |
| `readelf-symbols` | readelf | ✅ PASS |
| `objdump-headers` | objdump | ✅ PASS |
| `objdump-disassemble` | objdump | ✅ PASS |
| `objdump-syms` | objdump | ✅ PASS |
| `objdump-relocs` | objdump | ✅ PASS |
| `addr2line-basic` | addr2line | ✅ PASS |

### 4. Fixes applied

- **nm symbol type classification** — look up actual ELF section by index; classify by section name and flags (`SHF_EXECINSTR` → `t`, `SHF_WRITE` → `d`, `SHT_NOBITS` → `b`, read-only alloc → `r`)
- **c++filt demangling** — replaced hand-rolled parser with `cpp_demangle` crate for full Itanium ABI support; post-process `(long)N` cast syntax → `Nl` suffix notation to match GNU libiberty
- **size Berkeley format** — classify sections by ELF flags (`SHF_ALLOC`, `SHF_WRITE`, `SHT_NOBITS`) instead of section name; bss defaults to 0
- **size SysV format** — filter to `SHF_ALLOC` sections only; match GNU column widths (`{:<20}{:>5}{:>7}`)
- **addr2line output** — output `??:?` (not `??:0`) for unknown addresses; only print function name line with `-f`
- **readelf `-h` fix** — don't treat `-h` as `--help`; inline version/help check so `-h` means `--file-header`
- **readelf `-S` format** — two-line per section format matching GNU layout; intro line with section count and offset; full Key to Flags legend
- **readelf `-l` fix** — print "no program headers" message for object files instead of empty table
- **readelf `-s` fix** — raw ELF symtab parsing with section name, entry count, null symbol at index 0
- **objdump `-h` fix** — same `-h` → `--help` bug as readelf; filter to `SHF_ALLOC` sections; add flag description lines (`CONTENTS, ALLOC, LOAD, ...`); correct file offsets via raw ELF headers; proper alignment powers; machine-specific format name (`elf64-x86-64`)
- **objdump `-d` disassembly** — added `iced-x86` crate for proper x86/x64 instruction decoding with AT&T syntax (`GasFormatter`); symbol labels at function boundaries; zero-byte run collapsing with `...` ellipsis matching GNU behavior; per-section symbol filtering for correct labels
- **objdump `-t` fix** — tab separator between section and size; blank bind for common/undefined symbols; `*COM*` section name for common symbols
- **objdump `-r` fix** — map raw ELF relocation types to proper names (`R_X86_64_32`, `R_X86_64_PC32`, etc.); resolve symbol indices to names; show addends
- **testsuite normalization** — strip trailing whitespace in compare function to avoid false failures

### 5. Execution

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-binutils-test-strings-basic

# Run all binutils tests
nix flake check
```

## Coverage assessment

### What these tests are

These are **custom hand-written comparison tests**, not the actual upstream GNU binutils DejaGnu test suite. Each test runs a specific tool invocation against both GNU binutils and rust-binutils on the same input, then diffs the output. The 24 tests cover basic functionality of 8 out of 14 tools.

### What these tests are NOT

The upstream binutils test suite lives in `binutils/testsuite/` and uses DejaGnu/Expect (`.exp` files). It contains **hundreds of tests per tool** covering flags, edge cases, error handling, architecture-specific behavior, and regression tests accumulated over decades. We do not run any of those tests directly — the DejaGnu harness is complex and tightly coupled to the GNU build system.

### Coverage gaps

**Untested tools** (6 of 14):

- `strip` — no tests
- `objcopy` — no tests
- `ranlib` — no tests
- `as` — no tests (stub implementation)
- `ld` — no tests (stub implementation)

**Untested input types:**

- Linked executables and shared libraries (all tests use a single `.o` file)
- Multi-architecture ELF (only x86_64 tested)
- Non-ELF formats (PE/COFF, Mach-O)
- Large files, files with many sections/symbols
- Archives with long member names, nested archives

**Untested tool flags:**

- `nm`: `-D` (dynamic), `-A` (print-file-name), `-r` (reverse sort), `-n` (numeric sort)
- `strings`: `--encoding` (multibyte), `-t` (offset format), `-a` (scan whole file)
- `readelf`: `-d` (dynamic), `-r` (relocations), `-a` (all), `-W` (wide)
- `objdump`: `-p` (private headers), combined flags (`-dhr`), `--start-address`/`--stop-address`
- `size`: `-G` (GNU format)
- `ar`: `d` (delete), `m` (move), `p` (print), long-name archives
- `addr2line`: `-f` (functions), `-C` (demangle), `-i` (inlines), with actual debug info

**Untested error handling:**

- Missing files, permission errors
- Corrupted/truncated ELF files
- Invalid arguments, conflicting flags
- Files with no symbols, no sections, stripped binaries

## Completed steps

- [x] Add `rust-binutils-dev` package to `default.nix` (debug build for fast iteration)
- [x] Create `testsuite.nix` with the test runner derivation
- [x] Create 24 test scripts across 8 tools
- [x] Add test names to `checks` in `default.nix`
- [x] Run all tests and catalog results
- [x] Fix nm symbol type classification (`?` → proper type chars)
- [x] Fix c++filt demangling (add `cpp_demangle` crate, suffix notation)
- [x] Fix size Berkeley and SysV format
- [x] Fix addr2line `??:?` output
- [x] Fix readelf/objdump `-h` flag handling and output formatting
- [x] Fix objdump `-h` section headers (flags, offsets, alignment, format name)
- [x] Fix readelf `-S` section headers (two-line format, intro, full flags key)
- [x] Add x86 disassembly with `iced-x86` crate
- [x] Fix readelf `-l`, `-s` output format
- [x] Fix objdump `-t`, `-r` output format

## Next steps

- [ ] Add tests for `strip`, `objcopy`, `ranlib`
- [ ] Test with linked executables and shared libraries (not just `.o` files)
- [ ] Test more tool flags (nm `-D`, readelf `-d`/`-r`, objdump `-p`, etc.)
- [ ] Add error handling tests (bad input, missing files, corrupted ELF)
- [ ] Add multi-file and archive edge case tests
- [ ] Investigate running a subset of upstream DejaGnu tests via `runtest` in Nix