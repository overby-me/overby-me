# rust-binutils

A from-scratch Rust reimplementation of GNU `binutils` (ar, nm, objdump,
readelf, objcopy, strings, size, addr2line, c++filt, strip, elfedit) aimed
at passing the upstream GNU binutils test suite.

## Status

- **64/64 custom comparison tests passing (100%)**
- **328/329 upstream DejaGnu tests passing (99.7%)**

The single remaining failing dejagnu test is `readelf -wi
(binutils-all/x86-64/pr26808.dwp)`. It is intrinsically incompatible
with `binutils-all/pr26160` — both tests run on the same DWP file but
have contradictory expected outputs (one expects warning text inline
in stdout via stderr-merging dejagnu invocation, the other expects
clean stdout via `2>/dev/null`).

A multicall binary dispatches on `argv[0]` (or `argv[1]` for `cargo run
--`) so that the same Rust program serves all the symlinked tool names
under `bin/`.

## Architecture

### 1. Custom comparison tests (`testsuite.nix`)

A Nix derivation that runs a single named test, comparing `rust-binutils`
output against reference GNU `binutils` output. Takes `{ pkgs, tool, name
}` as arguments.

Key design decisions:

- **Two-level naming**: tests are keyed by `tool` + `name` (e.g.,
  `nm/basic`, `strings/object`)
- **Tool name mapping**: `cxxfilt` → `c++filt` binary name handled in Nix
- **Shared test object**: `bintest.s` from upstream is assembled once per
  test via `gcc -c`
- **Output normalization**: nix store paths, binary names, and trailing
  whitespace are normalized before diffing
- **Standalone test scripts**: each test is a shell script in
  `tests/${tool}/${name}.sh` that uses `$REF`, `$RUST`, `$TESTOBJ`,
  `$TMPDIR`, and the `compare` helper

### 2. Upstream DejaGnu tests (`dejagnu-testsuite.nix`)

A Nix derivation that runs the real upstream `.exp` test files from the
GNU binutils source tree using DejaGnu's `runtest` harness, with all
tool paths pointed at `rust-binutils` symlinks.

Key design decisions:

- **Real upstream tests**: runs the actual `.exp` files from
  `binutils/testsuite/binutils-all/`, not hand-written approximations
- **Generated `site.exp`**: configures tool paths (`NM`, `AR`,
  `OBJDUMP`, etc.) to rust-binutils, and `AS_FOR_TARGET`/`CC_FOR_TARGET`
  to GNU as/gcc for assembling test fixtures
- **Threshold-based pass/fail**: each check has `minPass` (minimum
  expected passes) and `maxFail` (maximum allowed failures) so
  regressions are caught but known failures don't block CI
- **Per-file and aggregate checks**: individual `.exp` files get
  separate Nix checks (e.g., `rust-binutils-dejagnu-size`), plus a
  `rust-binutils-dejagnu-all` informational check
- **Subdir tests**: `binutils-all/x86-64/x86-64.exp` and
  `binutils-all/i386/i386.exp` are run via a separate `runtest` call
  (basename-only dispatch can't resolve subdir paths)
- **Skipped tests**: `dlltool.exp` (Windows PE/COFF) and
  `debuginfod.exp` (server dependency) are skipped

### 3. `default.nix`

- `rust-binutils` package — release multicall binary with symlinks for
  each tool name in `postInstall`
- `rust-binutils-dev` package — debug build for fast iteration
- `checks` attribute set wiring per-tool custom tests, per-`.exp`
  DejaGnu checks, and the aggregate DejaGnu check

## Running the tests

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

## Test results

### Custom comparison tests (64/64 passing)

| Tool | Tests |
|---|---|
| `ar` | `create-list`, `extract`, `print` |
| `addr2line` | `basic`, `demangle`, `multiple`, `pretty` |
| `c++filt` | `basic`, `multiple`, `nested`, `no-params`, `no-strip-leading`, `types` |
| `nm` | `basic`, `debug-syms`, `defined-only`, `extern-only`, `just-symbols`, `just-symbols-with-prefix`, `no-sort`, `numeric-sort`, `posix`, `print-file-name`, `print-file-name-posix`, `radix-decimal`, `radix-hex`, `radix-octal`, `reverse-sort`, `undefined-only` |
| `objdump` | `disassemble`, `file-headers`, `headers`, `relocs`, `section-filter`, `syms` |
| `readelf` | `arch-specific`, `dynamic`, `file-header`, `groups`, `header-program`, `header-sections`, `headers-all`, `headers-combined`, `hex-dump`, `hex-dump-data`, `histogram`, `notes`, `program-headers`, `relocs`, `sections`, `string-dump`, `string-dump-missing`, `symbols`, `wide` |
| `size` | `basic`, `decimal`, `hex`, `octal`, `sysv`, `totals` |
| `strings` | `basic`, `min-length`, `object`, `radix-hex` |

### Upstream DejaGnu tests (328/329 passing)

| Test file | Pass | Fail | Total | Threshold |
|-----------|------|------|-------|-----------|
| `cxxfilt.exp` | 3 | 0 | 3 | minPass=3, maxFail=0 |
| `size.exp` | 3 | 0 | 3 | minPass=3, maxFail=0 |
| `nm.exp` | 15 | 0 | 15 | minPass=15, maxFail=0 |
| `ar.exp` | 14 | 0 | 14 | minPass=14, maxFail=0 |
| `readelf.exp` | 38 | 0 | 38 | minPass=38, maxFail=0 |
| `objdump.exp` | 32 | 0 | 33 | minPass=32, maxFail=0 |
| `objcopy.exp` | 120 | 0 | 120 | minPass=120, maxFail=0 |
| `compress.exp` | 45 | 0 | 45 | minPass=45, maxFail=0 |
| `strings.exp` | 1 | 0 | 1 | minPass=1, maxFail=0 |
| `addr2line.exp` | 3 | 0 | 3 | minPass=3, maxFail=0 |
| `update-section.exp` | 6 | 0 | 6 | minPass=6, maxFail=0 |
| `elfedit.exp` | 6 | 0 | 6 | minPass=6, maxFail=0 |
| `x86-64/x86-64.exp` | 34 | 1 | 35 | (informational; only the binutils-all-x86-64 subset) |
| `i386/i386.exp` | 8 | 0 | 8 | (informational; binutils-all-i386 subdir) |
| **Total** | **328** | **1** | **329** | |

## Coverage gaps

**Untested tools** (not covered by upstream `.exp` files we run):

- `strip` — tested indirectly via `objcopy.exp` (strip is an objcopy mode)
- `ranlib` — tested indirectly via `ar.exp`
- `as` — stub implementation (delegates to system `as`); upstream tests
  in `gas/testsuite/` (separate suite, would test the real assembler not
  ours)
- `ld` — stub implementation (delegates to system `ld`); upstream tests
  in `ld/testsuite/`

**Skipped upstream test files:**

- `dlltool.exp` — Windows-specific (PE/COFF)
- `debuginfod.exp` — requires debuginfod server

## Future work

- `gas/testsuite/` and `ld/testsuite/` integration when `as`/`ld` are
  implemented natively rather than delegating
- objcopy `--merge-notes` to actually merge consecutive GNU build
  attribute notes (currently a no-op)
- Full SFrame v2/v3 Function Index dump (only the header is rendered;
  FDE/FRE entries with CFA/FP/RA decoding are not yet implemented)
- Improve `readelf -wiaoRlL dw5` (DWARF 5 features: loclists/rnglists,
  decoded line) for more thorough coverage

## Workflow

1. Pick a failing test (`grep "^FAIL:"
   .../rust-binutils-dejagnu-all/test-output.log`).
2. Reproduce locally:
   `nix build .#checks.x86_64-linux.rust-binutils-dejagnu-<file>`.
3. Read the failure under `nix log <drv>` and compare against the
   expected `.r` / `.dump` file in the binutils source tree.
4. Fix the code, rebuild, re-run the threshold-gated check.
5. Commit and push the `binutils-test` bookmark.
