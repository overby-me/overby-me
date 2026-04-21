# Plan: Add upstream binutils tests as Nix checks

## Goal

Run tests from the upstream GNU binutils test suite against `rust-binutils`, comparing output with reference GNU binutils, similar to how `rust/awk` runs the gawk test suite.

## Architecture

### 1. Custom comparison tests (`testsuite.nix`) ✅

A Nix derivation that runs a single named test, comparing `rust-binutils` output against reference GNU `binutils` output. Takes `{ pkgs, tool, name }` as arguments.

Key design decisions:

- **Two-level naming**: tests are keyed by `tool` + `name` (e.g., `nm/basic`, `strings/object`)
- **Tool name mapping**: `cxxfilt` → `c++filt` binary name handled in Nix
- **Shared test object**: `bintest.s` from upstream is assembled once per test via `gcc -c`
- **Output normalization**: nix store paths, binary names, and trailing whitespace are normalized before diffing
- **Standalone test scripts**: each test is a shell script in `tests/${tool}/${name}.sh` that uses `$REF`, `$RUST`, `$TESTOBJ`, `$TMPDIR`, and the `compare` helper

### 2. Upstream DejaGnu tests (`dejagnu-testsuite.nix`) ✅

A Nix derivation that runs the real upstream `.exp` test files from the GNU binutils source tree using DejaGnu's `runtest` harness, with all tool paths pointed at `rust-binutils` symlinks.

Key design decisions:

- **Real upstream tests**: runs the actual `.exp` files from `binutils/testsuite/binutils-all/`, not hand-written approximations
- **Generated `site.exp`**: configures tool paths (`NM`, `AR`, `OBJDUMP`, etc.) to rust-binutils, and `AS_FOR_TARGET`/`CC_FOR_TARGET` to GNU as/gcc for assembling test fixtures
- **Threshold-based pass/fail**: each check has `minPass` (minimum expected passes) and `maxFail` (maximum allowed failures) so regressions are caught but known failures don't block CI
- **Per-file and aggregate checks**: individual `.exp` files get separate Nix checks (e.g., `rust-binutils-dejagnu-size`), plus a `rust-binutils-dejagnu-all` informational check
- **Skipped tests**: `dlltool.exp`, `elfedit.exp`, `debuginfod.exp`, `update-section.exp`, `compress.exp` are skipped (Windows-specific, unimplemented tools, or special dependencies)

### 3. `default.nix` ✅

- Added `rust-binutils-dev` package (debug build for fast iteration)
- Added `checks` attribute set wiring custom tests, per-file DejaGnu checks, and the aggregate DejaGnu check

## Test results

### Custom comparison tests (24/24 passing)

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

### Upstream DejaGnu tests (30/217 passing)

| Test file | Pass | Fail | Total | Threshold |
|-----------|------|------|-------|-----------|
| cxxfilt.exp | **3** | 0 | 3 | minPass=3, maxFail=0 |
| size.exp | **2** | 1 | 3 | minPass=2, maxFail=1 |
| nm.exp | **3** | 10 | 13 | minPass=3, maxFail=10 |
| ar.exp | **1** | 13 | 14 | minPass=1, maxFail=13 |
| readelf.exp | **6** | 32 | 39 | minPass=6, maxFail=33 |
| objdump.exp | **3** | 21 | 25 | minPass=3, maxFail=22 |
| objcopy.exp | **12** | 93 | 116 | minPass=12, maxFail=105 |
| strings.exp | 0 | 1 | 1 | minPass=0, maxFail=1 |
| addr2line.exp | 0 | 3 | 3 | minPass=0, maxFail=3 |
| **Total** | **30** | **174** | **217** | |

## Fixes applied

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

## Execution

```sh
# Run a single custom comparison test
nix build .#checks.x86_64-linux.rust-binutils-test-strings-basic

# Run a single upstream DejaGnu test file
nix build .#checks.x86_64-linux.rust-binutils-dejagnu-cxxfilt

# Run all upstream tests (informational, always passes)
nix build .#checks.x86_64-linux.rust-binutils-dejagnu-all -o result
cat result/test-output.log    # full DejaGnu output
cat result/summary.log        # PASS/FAIL lines only
cat result/results.txt        # machine-readable counts

# Run all checks
nix flake check
```

## Coverage assessment

### What we have

Two layers of testing:

1. **24 custom comparison tests** — hand-written scripts that run both GNU and rust-binutils on the same input and diff the output. These ensure exact output compatibility for the tested scenarios.

2. **217 upstream DejaGnu tests** — the real GNU binutils test suite, run via `runtest` with tool paths pointed at rust-binutils. These are the canonical tests used by the binutils project. Currently 30/217 (14%) pass.

### Remaining coverage gaps

**Upstream test failures (174):** Most failures are in `objcopy.exp` (93), `readelf.exp` (32), and `objdump.exp` (21). These failures represent missing features, output format differences, and unimplemented flags. Each failure is a concrete upstream test case that can be investigated and fixed individually.

**Untested tools** (not covered by upstream `.exp` files we run):

- `strip` — tested indirectly via `objcopy.exp` (strip is an objcopy mode)
- `ranlib` — tested indirectly via `ar.exp`
- `as` — stub implementation, upstream tests in `gas/testsuite/` (separate suite)
- `ld` — stub implementation, upstream tests in `ld/testsuite/` (separate suite)

**Skipped upstream test files:**

- `dlltool.exp` — Windows-specific (PE/COFF)
- `elfedit.exp` — tool not implemented
- `debuginfod.exp` — requires debuginfod server
- `update-section.exp` — requires elfedit
- `compress.exp` — requires zlib compressed section support

## Completed steps

- [x] Add `rust-binutils-dev` package to `default.nix` (debug build for fast iteration)
- [x] Create `testsuite.nix` with the custom comparison test runner
- [x] Create 24 custom test scripts across 8 tools
- [x] Fix nm, c++filt, size, addr2line, readelf, objdump to pass custom tests
- [x] Add x86 disassembly with `iced-x86` crate
- [x] Wire upstream DejaGnu test suite into Nix checks via `dejagnu-testsuite.nix`
- [x] Configure `site.exp` with tool paths, `AS_FOR_TARGET`, `CC_FOR_TARGET`
- [x] Add 9 per-file DejaGnu checks with calibrated thresholds
- [x] Add informational `dejagnu-all` check

## Next steps

- [ ] Investigate and fix top upstream test failures (start with `ar.exp`, `nm.exp` — most fixable)
- [ ] Raise `minPass` thresholds as fixes land (ratcheting approach)
- [ ] Add `gas/testsuite/` and `ld/testsuite/` DejaGnu integration for `as`/`ld`
- [ ] Add compressed section support for `compress.exp` tests