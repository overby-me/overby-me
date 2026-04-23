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

### Upstream DejaGnu tests (189/220 passing)

| Test file | Pass | Fail | Total | Threshold |
|-----------|------|------|-------|-----------|
| cxxfilt.exp | **3** | 0 | 3 | minPass=3, maxFail=0 |
| size.exp | **3** | 0 | 3 | minPass=3, maxFail=0 |
| nm.exp | **13** | 0 | 13 | minPass=13, maxFail=0 |
| ar.exp | **12** | 2 | 14 | minPass=12, maxFail=2 |
| readelf.exp | **26** | 12 | 39 | minPass=26, maxFail=12 |
| objdump.exp | **22** | 10 | 33 | minPass=19, maxFail=3 (standalone; `-all` sees 22) |
| objcopy.exp | **106** | 9 | 116 | minPass=106, maxFail=9 |
| strings.exp | **1** | 0 | 1 | minPass=1, maxFail=0 |
| addr2line.exp | **3** | 0 | 3 | minPass=3, maxFail=0 |
| **Total** | **189** | **31** | **220** | |

## Fixes applied

- **nm symbol type classification** — look up actual ELF section by index; classify by section name and flags (`SHF_EXECINSTR` → `t`, `SHF_WRITE` → `d`, `SHT_NOBITS` → `b`, read-only alloc → `r`)
- **nm POSIX format** — `-P`/`--portability` output: `name type value size`; `--format=posix/sysv/bsd` selection
- **nm radix selection** — `-t d/o/x` / `--radix=` for decimal/octal/hex addresses
- **nm `--size-sort`** — sort symbols by size and display size
- **nm `--line-numbers`** — full `gimli`-based DWARF-4 parser with relocation application for `.o` files and `DW_AT_specification` resolution
- **nm `--ifunc-chars`** — custom type chars for GNU indirect functions (`STT_GNU_IFUNC`)
- **nm `--no-weak`** — filter out weak symbols; `STB_GNU_UNIQUE` support (`u` type char)
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
- **objdump `-i` info** — format/architecture listing with `BFD header file version`, `srec`, CPU names
- **objdump `-f` file headers** — `architecture:` line, ELF flags (`HAS_RELOC`, `HAS_SYMS`)
- **objdump archive support** — `-f`, `-h`, `-t`, `-r`, `-d`, `-s` now process `.a` archive members individually
- **objdump `-s` full contents** — hex dump of section contents in GNU format
- **objdump `--disassemble=SYM`** — symbol-specific disassembly with proper range calculation
- **objdump `--show-all-symbols`** — show local symbols during disassembly
- **objdump `-j` section filter** — section name filtering for `-s`
- **readelf `-h` magic line** — trailing space after last byte matching GNU format
- **readelf `-s` symbol table** — proper `Symbol table '.symtab' contains N entries:` header; null symbol at index 0; raw ELF parsing for accurate fields; visibility strings
- **readelf `-r` relocations** — `at offset 0xNN` in header; proper type names via `elf_reloc_type_name()`; symbol resolution; correct pluralization
- **addr2line DWARF implementation** — replaced stub with real `gimli`-based DWARF line program walker; function name lookup via `DW_TAG_subprogram`; `-s`/`--basenames` and `-e` flags
- **readelf `-p`/`-n`/`-t`/`-C` modes** — `-p`/`--string-dump` for printable nul-terminated strings; `-n`/`--notes` for `SHT_NOTE` parsing with NT_GNU_* type names; `-t`/`--section-details` three-line per-section output with full names and decoded flag names; `-C`/`--demangle` via `cpp_demangle`
- **readelf archive support** — regular and thin (`!<thin>\n`) archive inspection with `File: archive(member)` headers
- **readelf section flags** — `R` for `SHF_GNU_RETAIN`, `o` for unknown OS bits, `p` for processor-specific
- **objcopy byte-copy fast path** — when no transformations requested, use `fs::copy()` directly (fixes simple copy, executable copy, ELF group/MBIND/NOBITS, etc.)
- **objcopy `-O verilog/srec/ihex`** — Verilog hex output with `@ADDR` headers and `--verilog-data-width 1/2/4/8/16` byte-swapping; basic SREC and IHEX stubs
- **objcopy section ops** — `--set-section-alignment`, `--set-section-flags`, `--rename-section` parsing/application
- **objcopy symbol ops** — `--strip-symbol` with reloc check (emits `not stripping symbol` warning); `--keep-global-symbol` vs `--globalize-symbol` incompatibility error
- **strip archive support** — detects `!<arch>` magic and strips each ELF member individually
- **STT_NOTYPE symbol handling** — infer symbol kind (Text if section is code, else Data) instead of failing the writer
- **strings `-e` encoding** — added `--encoding=l/b` for UTF-16LE/BE multibyte string scanning
- **ar POSIX argument parsing** — support `ar -r -c archive file` style (multiple dash-prefixed args)
- **ar deterministic mode** — `D`/`U` flags for deterministic (uid=0, gid=0, mtime=0, mode=0o100644) vs real metadata; `SOURCE_DATE_EPOCH` support
- **ar `tv` verbose format** — `rw-r--r-- 0/0 size date name` matching GNU; `O` flag for hex offsets
- **ar `-m` move** — move members to end of archive
- **ar `-d` basename matching** — delete/extract match by basename
- **ar `--output=dir -x`** — extract to specified output directory
- **ar `--record-libdeps`** — add `__.LIBDEP` member with library dependency info
- **nm `--print-armap`** — display archive symbol table index
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

2. **218 upstream DejaGnu tests** — the real GNU binutils test suite, run via `runtest` with tool paths pointed at rust-binutils. These are the canonical tests used by the binutils project. Currently 184/220 (84%) pass.

### Remaining coverage gaps

**Upstream test failures (36):** Most failures are in `readelf.exp` (13), `objdump.exp` (12), and `objcopy.exp` (11). These failures represent missing features, output format differences, and unimplemented flags. Each failure is a concrete upstream test case that can be investigated and fixed individually.

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
- [x] Fix ar.exp failures (1/14 → 12/14): POSIX args, D/U flags, `tv`/`tvO`, `-m`, `-d` basename, `--output`, `--record-libdeps`
- [x] Fix nm.exp failures (3/13 → 12/13): POSIX format, radix, `--size-sort`, `--line-numbers` (DWARF), `--ifunc-chars`, `--no-weak`, `--print-armap`
- [x] Fix addr2line.exp failures (0/3 → 3/3): real DWARF line program implementation, function name lookup, `-s`/`-e` flags
- [x] Fix strings.exp failure (0/1 → 1/1): `--encoding=l/b` UTF-16LE/BE support
- [x] Fix readelf.exp failures (6/39 → 17/39): `-h`, `-s`, `-r`; `-p` string dump, `-n` notes, `-t` section details, `-C/--demangle`, archive (regular & thin) inspection, SHF_GNU_RETAIN/SHF_MASKOS flag handling
- [x] Fix objdump.exp failures (3/25 → 17/29): `-i` info, `-f` file headers, archive support, `-s` hex dump, `--disassemble=SYM`, `--show-all-symbols`, `-j` section filter
- [x] Raise `minPass` thresholds for ar (1→12), nm (3→13), addr2line (0→3), strings (0→1), readelf (6→25), objdump (3→20), objcopy (12→104), size (2→3)
- [x] Fix objdump `-Z -s` (decompress hex dump): remove stray debug eprintln; decompress legacy GNU `ZLIB`-magic format with concatenated zlib streams via flate2
- [x] Add readelf `--debug-dump=links` / `-wK` / `-wN` for `.gnu_debuglink`, `.gnu_debugaltlink`, and `.debug_info` dwo-file links (DWARF5 `DW_AT_dwo_name` + DWARF4 `DW_AT_GNU_dwo_name` with relocation-aware section loader)
- [x] Fix size.exp `-G` GNU format: separate text/data/bss classification rules and 4-column GNU layout
- [x] Fix objcopy strip-13/14/15: validate_relocations() emits unsupported reloc type / invalid symbol index errors; strip no-op fast-path preserves byte-exact section layout
- [x] Fix nm `--line-numbers` for extern globals: implement `nm_build_line_info` with gimli, handles `DW_AT_specification` chains for declaration→definition resolution
- [x] Fix readelf `--debug-dump=loc` (`-wo`) and `--decompress --hex-dump` for SHF_COMPRESSED + legacy `ZLIB`-prefixed sections (flate2 dep)
- [x] Fix more objcopy: `--only-keep-debug` (PROGBITS→NOBITS), strip empty file (.symtab/.strtab removal), SHT_GROUP preservation via fast-path, `-wR`/`--debug-dump=ranges`, group signature symbol lookup
- [x] Fix more readelf.exp failures (17→21/39): `-p` escape sequences (`\n`, `^X`), `--enable-checks` for zero-sized sections, `-j`/`--display-section`, `-r` for `SHT_RELR` bitmap-encoded relocations
- [x] Fix more objcopy.exp failures (71→76/116): additional section/symbol handling edge cases
- [x] Fix more objdump.exp failures (17→19/33): `-Wk`/`--dwarf=links` for `.gnu_debuglink`/`.gnu_debugaltlink` parsing; `-s -j .zdebug_*` compressed-section notice; `--start-address`/`--stop-address` for `-s` and `-d`; DWARF flag parsing fixes
- [x] Fix more objcopy.exp failures (44→71/116): glob-pattern section selectors, `--add-section`, `--add-symbol`, `--strip-section-headers`, SREC start/VMA handling
- [x] Fix objcopy.exp failures (12/116 → 44/116): byte-copy fast path, `-O verilog`/`srec`/`ihex` output, `--set-section-alignment`, `--set-section-flags`, `--rename-section`, `--strip-symbol` reloc check, `--keep-global-symbol` vs `--globalize-symbol` conflict, archive support in `strip`, `STT_NOTYPE` symbol handling, `-I/-N/-G/-p` flag parsing

- [x] Fix objdump.exp failures (19→22/33): in-place ELF `sh_addr`/`e_entry` patching for `--change-section-address`, `--adjust-vma`, `--set-start`, `--adjust-start` (preserves all other ELF structure exactly)

- [x] Fix readelf.exp failure (25→26/39): `-j`/`--display-section` on REL/RELA sections now emits a relocation table instead of a hex dump (GNU-compatible format; refactored `readelf_relocs` to share per-section printing via new `readelf_dump_reloc_section`)

- [x] Fix strip on executables (104→106 objcopy.exp passes): in-place ELF section-table edit for ET_EXEC/ET_DYN preserves `sh_addr`, program-header layout, and PROGBITS/NOBITS distinction (slow `object::write::Object` path was producing unrunnable binaries by zeroing addresses); `-K` rewrites `.symtab`/`.strtab` instead of dropping them. Fixes `run stripped executable` and `run stripped executable with saving a symbol`.

## Next steps

- [ ] Add `gas/testsuite/` and `ld/testsuite/` DejaGnu integration for `as`/`ld`
- [ ] Add compressed section support for `compress.exp` tests