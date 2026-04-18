# Upstream Test Suite Plan

Goal: run the GNU gzip upstream test suite unmodified against rust-gzip
and pass every test. This mirrors the approach used for `rust/awk` — each
upstream test becomes its own Nix check, wired through `default.nix` and
driven by a single `testsuite.nix` template.

## Upstream layout

- Source: `pkgs.gzip.src` (GNU gzip tarball).
- Tests live in `tests/` in the extracted source tree.
- Each test is a POSIX `/bin/sh` script with **no extension** (e.g.
  `keep`, `hufts`, `zdiff`, `help-version`).
- Drivers:
  - `tests/init.sh` — gnulib helper, provides `compare`, `returns_`,
    `path_prepend_`, `fail_`, `skip_`, `Exit`, `framework_failure_`,
    per-test tempdir setup.
  - `tests/init.cfg` — project-specific `init.sh` extensions.
- Only binary fixture: `tests/hufts-segv.gz` (used by `hufts`).
- Canonical `TESTS =` list comes from `tests/Makefile.am`.

Tests are self-checking: they assert via `compare expected actual` and
`returns_ N cmd`, and signal pass/fail via exit code. Unlike awk we do
**not** need to diff rust-gzip output against reference gzip output — we
just run the upstream script and propagate its exit code.

## Test inventory (≈31 scripts)

`gzip-env`, `helin-segv`, `help-version`, `hufts`, `keep`, `list`,
`list-big`, `memcpy-abuse`, `mixed`, `null-suffix-clobber`,
`pipe-output`, `reference`, `reproducible`, `stdin`, `synchronous`,
`timestamp`, `trailing-nul`, `two-files`, `unpack-invalid`,
`unpack-valid`, `unzip-valid`, `upper-suffix`, `write-error`,
`z-suffix`, `zdiff`, `zgrep-abuse`, `zgrep-binary`, `zgrep-context`,
`zgrep-f`, `zgrep-signal`, `znew-k`.

Authoritative names come from `tests/Makefile.am` in the pinned
`pkgs.gzip.src`; regenerate the Nix list from that file when bumping.

## Harness design

### `testsuite.nix`

Mirrors `rust/awk/testsuite.nix` but simpler (no output diff). Per-test
`pkgs.runCommand` that:

1. Extracts `pkgs.gzip.src` and `cd`s into `gzip-*/tests`.
2. Builds a shadow `$PATH` dir containing:
   - `gzip` → rust-gzip's binary.
   - `gunzip`, `zcat` → already symlinks created by rust-gzip's
     `postInstall`.
   - `zdiff`, `zcmp`, `zgrep`, `zegrep`, `zfgrep`, `zforce`, `zmore`,
     `zless`, `znew`, `gzexe` → copied from the upstream source tarball
     (they are POSIX shell scripts that shell out to `gzip`). They live
     at the top level of the extracted source, not in `tests/`.
3. Exports the environment the gnulib harness expects:
   - `LC_ALL=C`
   - `srcdir=.`
   - `VERSION=<pkgs.gzip.version>`
   - `PACKAGE_VERSION=<pkgs.gzip.version>`
   - `PACKAGE_BUGREPORT=bug-gzip@gnu.org`
   - `TERM=dumb`, `PAGER` unset
   - `EXEEXT=""`
   - `abs_top_builddir=$PWD/..`
4. Runs `timeout 60 sh ./${name}` and propagates the exit code
   (`touch $out` on success, `exit 1` otherwise).

Native build inputs: `rust-gzip-dev`, `gzip` (for `.src` and fallback
scripts), `coreutils`, `diffutils`, `gnused`, `gnugrep`, `bash`,
`findutils`, `gawk`.

### `default.nix`

Add a `rust-gzip-dev` package alongside `rust-gzip` (debug build, faster
iteration — same pattern as `rust-awk-dev`). Wire the `checks` attrset by
listing every test name and generating `rust-gzip-test-${name}` via
`testsuite.nix`.

## Milestones

1. **Scaffolding** — `testsuite.nix`, `rust-gzip-dev` package, `checks`
   list with every test name. One test target builds and runs (even if
   failing). No changes to `src/main.rs` yet.
2. **Green-path tests** — `help-version`, `stdin`, `keep`,
   `pipe-output`, `two-files`, `z-suffix`, `upper-suffix`. These
   exercise the happy path and surface the first round of CLI/exit-code
   gaps.
3. **Listing and metadata** — `list`, `list-big`, `reference`,
   `reproducible`, `timestamp`. Likely requires matching GNU gzip's
   exact `-l` column layout and honoring `SOURCE_DATE_EPOCH` /
   `--no-name` mtime rules.
4. **Archive-format edge cases** — `hufts` (corrupted deflate stream),
   `helin-segv`, `memcpy-abuse`, `unpack-invalid`, `unpack-valid`,
   `unzip-valid`, `trailing-nul`, `mixed`, `null-suffix-clobber`.
   Expect to harden the decoder and error messages.
5. **I/O and signals** — `write-error`, `synchronous`, `pipe-output`
   (re-check), `zgrep-signal`. SIGPIPE handling, fsync semantics.
6. **Companion scripts** — `zdiff`, `zgrep-*`, `znew-k`, `gzip-env`.
   These drive the shell scripts shipped by upstream gzip; our job is to
   make `rust-gzip` behave well enough as a drop-in that the scripts
   pass. If a script itself has bugs only fixed in a newer gzip, pin the
   script from the matching `pkgs.gzip.src`.

Track progress in `README.md` (like `rust/awk/README.md`): a single
"N/31 passing" line and a short architecture summary.

## Known likely gaps in `src/main.rs`

Reading these off the current source so we can attack them in order once
the harness is wired:

- `-l` output format almost certainly doesn't match GNU's exact columns
  (header, spacing, `crc`/`method`/`date` columns with `-v`).
- `compress_stream` always stamps the current time as mtime; `--no-name`
  must also zero the stored name and mtime. Conversely `-N`/default
  should preserve source mtime, not use `SystemTime::now()`.
- `--suffix`/`-S` is accepted but ignored (hard-coded `.gz`). Several
  tests (`z-suffix`, `upper-suffix`) will exercise it.
- `strip_gz_suffix` hard-codes a suffix list; real gzip honors
  `--suffix` and rejects unknown suffixes differently.
- Error message wording (`gzip: FILE: …`) must match upstream byte-for
  -byte for the tests that `compare` stderr.
- Exit codes: upstream uses 0 / 1 / 2 distinctly (warning vs error);
  our current code collapses everything into 0/1.
- `--test` on stdin currently writes nothing on success; upstream is
  silent too, so that's fine, but verbose/quiet interactions need
  auditing.
- `-r` directory recursion order and error reporting may differ.
- No `gzip-env` parsing of the `GZIP` env var (deprecated but tested).

Each milestone will likely land a small batch of fixes plus any
normalizations the harness needs.

## Running

```sh
# Single test
nix build .#checks.x86_64-linux.rust-gzip-test-keep

# View log for a failure
nix log .#checks.x86_64-linux.rust-gzip-test-keep

# All tests, keep going on failures
nix build .#checks.x86_64-linux.rust-gzip-test-* --keep-going --no-link
```
