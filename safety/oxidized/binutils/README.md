# Oxidized Binutils

<!-- publish:begin -->
> Part of the [overby.me monorepo](https://tangled.org/overby.me/overby.me), where this lives in
> [`safety/oxidized/binutils`](https://tangled.org/overby.me/overby.me/tree/main/safety/oxidized/binutils) and where all development happens.
>
> It is also published on its own, as
> [tangled.org/overby.me/oxidized-binutils](https://tangled.org/overby.me/oxidized-binutils) and
> [github.com/overby-me/oxidized-binutils](https://github.com/overby-me/oxidized-binutils). Both
> are read-only mirrors, rebuilt from the monorepo with
> [josh](https://github.com/josh-project/josh): a commit made to either is
> overwritten by the next sync, so please open issues and pull requests on the
> monorepo.
<!-- publish:end -->

A from-scratch Rust reimplementation of GNU `binutils` (ar, nm, objdump,
readelf, objcopy, strings, size, addr2line, c++filt, strip, elfedit) aimed
at passing the upstream GNU binutils test suite.

## Status

- **138/138 custom comparison tests passing (100%)**
- **329/329 upstream DejaGnu tests passing (100%)**

A multicall binary dispatches on `argv[0]` (or `argv[1]` for `cargo run
--`) so that the same Rust program serves all the symlinked tool names
under `bin/`.

## Architecture

### 1. Custom comparison tests (`testsuite.nix`)

A Nix derivation that runs a single named test, comparing `oxidized-binutils`
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
tool paths pointed at `oxidized-binutils` symlinks.

Key design decisions:

- **Real upstream tests**: runs the actual `.exp` files from
  `binutils/testsuite/binutils-all/`, not hand-written approximations
- **Generated `site.exp`**: configures tool paths (`NM`, `AR`,
  `OBJDUMP`, etc.) to oxidized-binutils, and `AS_FOR_TARGET`/`CC_FOR_TARGET`
  to GNU as/gcc for assembling test fixtures
- **Threshold-based pass/fail**: each check has `minPass` (minimum
  expected passes) and `maxFail` (maximum allowed failures) so
  regressions are caught but known failures don't block CI
- **Per-file and aggregate checks**: individual `.exp` files get
  separate Nix checks (e.g., `oxidized-binutils-dejagnu-size`), plus a
  `oxidized-binutils-dejagnu-all` informational check
- **Subdir tests**: `binutils-all/x86-64/x86-64.exp` and
  `binutils-all/i386/i386.exp` are run via a separate `runtest` call
  (basename-only dispatch can't resolve subdir paths)
- **Skipped tests**: `dlltool.exp` (Windows PE/COFF) and
  `debuginfod.exp` (server dependency) are skipped

### 3. `default.nix`

- `oxidized-binutils` package — release multicall binary with symlinks for
  each tool name in `postInstall`
- `oxidized-binutils-dev` package — debug build for fast iteration
- `checks` attribute set wiring per-tool custom tests, per-`.exp`
  DejaGnu checks, and the aggregate DejaGnu check

## Running the tests

```sh
# Run a single custom comparison test
nix build .#checks.x86_64-linux.oxidized-binutils-test-strings-basic

# Run a single upstream DejaGnu test file
nix build .#checks.x86_64-linux.oxidized-binutils-dejagnu-cxxfilt

# Run all upstream tests (informational, always passes)
nix build .#checks.x86_64-linux.oxidized-binutils-dejagnu-all -o result
cat result/test-output.log    # full DejaGnu output
cat result/summary.log        # PASS/FAIL lines only
cat result/results.txt        # machine-readable counts

# Run all checks
nix flake check
```

## Test results

### Custom comparison tests (138/138 passing)

| Tool | Tests |
|---|---|
| `ar` | `create-list`, `extract`, `offsets`, `print` |
| `addr2line` | `addresses`, `addresses-functions-pretty`, `basenames`, `basic`, `demangle`, `functions`, `inlines`, `multiple`, `out-of-section`, `pretty`, `pretty-functions` |
| `c++filt` | `basic`, `multiple`, `nested`, `no-params`, `no-recurse-limit`, `no-strip-leading`, `strip-underscore`, `style`, `types` |
| `nm` | `all-with-prefix`, `archive`, `basic`, `debug-syms`, `defined-only`, `extern-only`, `extern-only-posix`, `format-bsd`, `just-symbols`, `just-symbols-with-prefix`, `last-wins-sort`, `line-numbers`, `multiple-files`, `no-demangle`, `no-recurse-limit`, `no-sort`, `no-weak`, `numeric-extern`, `numeric-sort`, `posix`, `posix-decimal`, `posix-octal`, `print-armap`, `print-file-name`, `print-file-name-posix`, `print-size`, `quiet`, `radix-decimal`, `radix-hex`, `radix-hex-print-file-name`, `radix-octal`, `reverse-numeric`, `reverse-sort`, `size-sort`, `special-syms`, `target-noop`, `undefined-only`, `undefined-only-posix` |
| `objdump` | `archive`, `disassemble`, `disassemble-section`, `disassemble-with-syms`, `disassemble-zeroes`, `file-headers`, `headers`, `headers-section-filter`, `headers-syms`, `intel-mnemonic`, `intel-syntax`, `intel-syntax-bundled`, `multiple-files`, `no-show-raw-insn`, `relocs`, `section-filter`, `section-filter-bundled`, `section-headers-wide`, `start-address`, `syms`, `syms-section-filter` |
| `readelf` | `arch-specific`, `archive-index-not-archive`, `dynamic`, `dynamic-arch`, `dynamic-program-headers`, `file-header`, `groups`, `groups-wide`, `header-notes`, `header-program`, `header-sections`, `headers-all`, `headers-combined`, `hex-dump`, `hex-dump-data`, `hex-dump-text`, `histogram`, `multiple-files`, `notes`, `notes-symbols`, `notes-wide`, `notes-with-sections`, `program-headers`, `program-wide`, `relocs`, `sections`, `sections-arch-wide`, `string-dump`, `string-dump-data`, `string-dump-missing`, `string-dump-text`, `symbols`, `unwind`, `use-dynamic`, `version-info`, `wide` |
| `size` | `basic`, `common`, `decimal`, `hex`, `multiple-files`, `octal`, `sysv`, `sysv-hex`, `sysv-octal`, `totals` |
| `strings` | `basic`, `min-length`, `multiple-files`, `object`, `print-file-name`, `radix-decimal`, `radix-hex`, `radix-octal` |

### Upstream DejaGnu tests (329/329 passing)

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
| `x86-64/x86-64.exp` | 35 | 0 | 35 | (informational; only the binutils-all-x86-64 subset) |
| `i386/i386.exp` | 8 | 0 | 8 | (informational; binutils-all-i386 subdir) |
| **Total** | **329** | **0** | **329** | |

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
   .../oxidized-binutils-dejagnu-all/test-output.log`).
2. Reproduce locally:
   `nix build .#checks.x86_64-linux.oxidized-binutils-dejagnu-<file>`.
3. Read the failure under `nix log <drv>` and compare against the
   expected `.r` / `.dump` file in the binutils source tree.
4. Fix the code, rebuild, re-run the threshold-gated check.
5. Commit and push the `binutils-test` bookmark.