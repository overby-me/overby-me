# Changelog

All notable changes to rust-patchelf.

## [Unreleased]

### Test suite compatibility

Passes 46/46 of the upstream NixOS/patchelf 0.15.2 test suite as
Nix checks. Up from 14/46 at the start of tracked development.

### Source layout

- Refactored single 639-line `src/main.rs` into eight modules:
  `args`, `elf`, `grow`, `commands/{print, interpreter, rpath,
  soname, needed, debug}`. `main.rs` itself is ~150 lines of glue +
  the `parse_with_workarounds` helper.

### Nix integration

- `pkgs.rust-patchelf` (release, LTO + strip) and
  `pkgs.rust-patchelf-dev` (debug, fast compile).
- `rust/patchelf/fixtures.nix` builds the upstream `tests/`
  artefacts once per test session via `make check TESTS=`. Sets
  `dontPatchELF`, `dontStrip`, `noAuditTmpdir`, `noBrokenSymlinks`,
  and stubs `fixupOutputHooks=()` so `libbar.so` keeps its
  `/build/.../no-such-path` runpath and the rest of the fixtures
  reach the tests intact. Pre-creates the per-arch
  `no-rpath-${arch}.sh` symlinks (the upstream Makefile only does
  this during `make check`).
- `rust/patchelf/testsuite.nix` runs each upstream `tests/*.sh` in
  a sandbox with `rust-patchelf-dev` symlinked at the expected
  `../src/patchelf` path; exports `STRIP`/`OBJDUMP`/`READELF`/
  `OBJCOPY`/`PATCHELF_DEBUG=1`/`srcdir=.`.
- `rust/patchelf/default.nix` declares 46 per-test checks (32
  `src_TESTS` + 14 `no_rpath_arch_TESTS`).

### Argument parser

- All upstream flags accepted: `--print-interpreter`,
  `--set-interpreter`, `--print-rpath`, `--set-rpath`,
  `--add-rpath`, `--shrink-rpath`, `--allowed-rpath-prefixes`,
  `--remove-rpath`, `--force-rpath`, `--print-soname`,
  `--set-soname`, `--print-needed`, `--add-needed`,
  `--remove-needed`, `--replace-needed`, `--add-debug-tag`,
  `--no-default-lib`, `--clear-execstack`, `--set-execstack`,
  `--print-execstack`, `--debug`, `--rename-dynamic-symbols`,
  `--clear-symbol-version`, `--output`, `--page-size`, `--version`,
  `--help`/`-h`.
- `@FILE` argument expansion in any string-typed flag with
  upstream's `getting info about FILE` error message on missing
  file.

### Read operations

- `--print-interpreter` reads `PT_INTERP`.
- `--print-rpath` prefers `DT_RUNPATH`, falls back to `DT_RPATH`,
  and prints empty when neither exists.
- `--print-soname` prints `DT_SONAME` or errors with
  `no DT_SONAME found`.
- `--print-needed` lists every `DT_NEEDED`.

### In-place modify operations (no growth required)

- `--set-interpreter PATH` overwrites `PT_INTERP` when the new path
  fits.
- `--set-rpath`/`--add-rpath`/`--set-soname`/`--replace-needed`
  overwrite the dynstr entry in place when the new value fits the
  available slack at the original string offset.
- `--shrink-rpath` filters entries by needed-lib presence; satisfies
  each lib by the *first* matching entry (matches upstream).
  `--allowed-rpath-prefixes ARG` drops entries that don't start with
  any of the colon-delimited prefixes.
- `--remove-rpath` re-tags `DT_RPATH`/`DT_RUNPATH` to `DT_DEBUG`
  (cannot use `DT_NULL` because that terminates the dynamic array).
- `--remove-needed` zeros the matching `DT_NEEDED` entry (tag and
  value).
- `--add-debug-tag` overwrites the first `DT_NULL` slot in
  `.dynamic` with `DT_DEBUG` (no-op when `DT_DEBUG` is already
  present).

### Growth engine (`src/grow.rs`)

- `ElfBits` abstracts the per-field byte arithmetic for `Elf_Dyn`
  (8 vs 16 bytes), `Elf_Phdr` (32 vs 56 bytes — different field
  order between 32 and 64-bit), `Elf_Shdr` (40 vs 64 bytes), and
  LE/BE word reads/writes. Same algorithms work for all four
  width/endian combinations.
- `append_to_dynstr(elf, data, extra)` returns the new dynstr vaddr
  and the offset of the first appended byte. Updates `DT_STRTAB`,
  `DT_STRSZ`, the `.dynstr` section header, and LOAD #1
  `filesz`/`memsz`. Strings are referenced by offset into strtab,
  not by VA, so existing offsets remain valid after relocation.
- `add_dynamic_entry(elf, data, d_tag, d_val)` overwrites the first
  `DT_NULL` slot in `.dynamic`.
- Two placement strategies, tried in order:
  - **LOAD #1 slack** — park in the 2-3 KB of zero padding between
    the end of LOAD #1 `filesz` and the next page boundary; grow
    LOAD #1 `filesz`/`memsz` to cover.
  - **GNU_STACK / PT_NULL → PT_LOAD** — append at file EOF
    (page-aligned), rewrite the otherwise-useless `PT_GNU_STACK`
    (or `PT_NULL` on MIPS) phdr into a fresh read-only `PT_LOAD`
    covering the new region.
- PT_INTERP growth uses the same LOAD-slack strategy first and then
  falls back to plain file-EOF append (the kernel reads PT_INTERP
  via `p_offset` without LOAD coverage).

### Modify operations using growth

- `--set-rpath` adds a new `DT_RPATH`/`DT_RUNPATH` entry via the
  first `DT_NULL` slot when none exists.
- `--add-rpath` falls through to `--set-rpath` when no rpath exists.
- `--set-soname` adds a new `DT_SONAME` entry the same way.
- `--add-needed` always uses growth (lib name in `.dynstr` + new
  `DT_NEEDED` slot).
- `--replace-needed` falls through to growth when the new name is
  longer than the original string's slack.
- `--set-interpreter` parks the new path in LOAD slack or appends at
  file EOF.

### Multi-action invocations

- `main.rs` re-parses the (possibly mutated) buffer at the start of
  each action via a per-iteration snapshot. Multi-action commands
  see the post-previous-grow layout for each subsequent action.

### `--force-rpath`

- Plumbed through `Options.force_rpath` into `set_rpath` and
  `add_rpath`. When set, new entries use `DT_RPATH` instead of
  `DT_RUNPATH`, and an existing `DT_RUNPATH` tag is flipped to
  `DT_RPATH` in place.

### Output handling

- `--output FILE` writes to a different path while preserving the
  input file's mode (so the rewritten file is still executable).

### Compatibility workarounds

- `parse_with_workarounds`: when goblin 0.9 rejects a binary because
  `DT_GNU_HASH` points at zero buckets (`strip --remove-section=
  .gnu.hash` produces this), walk the program-header table by hand
  to locate `PT_DYNAMIC`, then rewrite the `d_tag` of any
  `DT_GNU_HASH` entry to `DT_DEBUG` in a *parsing copy* of the
  buffer. The on-disk buffer keeps the original entry so the round
  trip preserves it; goblin only sees the patched copy.
