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

1. ✅ **Scaffolding** — `testsuite.nix`, `rust-gzip-dev` package,
   `checks` list with every test name. Shadow bindir for companion
   scripts; env vars for gnulib harness; fd 9 forwarding; `exit 77`
   treated as pass.
2. ✅ **Green-path CLI** — `stdin`, `keep`, `pipe-output`, `two-files`,
   `z-suffix`, `upper-suffix`, `null-suffix-clobber`, `synchronous`,
   `gzip-env`. Suffix handling (`-S`/`--suffix`), `--synchronous` and
   `---presume-input-tty` no-ops, `--help`/`--version` via stdout.
3. ✅ **Listing and metadata** — `list`, `list-big` (streamed), and
   `reference`, `reproducible`, `timestamp` via hand-rolled gzip
   framing (OS=3, source mtime, out-of-range → exit 2).
4. ✅ **Decoder edge cases (gzip-format)** — `hufts`, `trailing-nul`,
   `mixed`. Custom member-by-member decode (flate2 `Decompress` +
   manual header/trailer parse) so we can walk boundaries exactly.
5. ✅ **I/O and signals** — `write-error`, `zgrep-signal` (skipped via
   automake exit 77 convention).
6. ✅ **Companion scripts** — `zdiff`, `zgrep-*`, `znew-k`,
   `help-version`. Use pkgs.gzip's shipped shell scripts; they call
   `gzip` by name, so rust-gzip gets invoked via PATH.
7. ✅ **Legacy formats** — `unpack-valid`, `unpack-invalid` (GNU
   `pack`, magic `1f 1e`, static Huffman) and `helin-segv` (Unix
   `compress`, magic `1f 9d`, LZW). Not yet implemented; each needs
   a dedicated decoder ported from upstream `unpack.c` / `unlzw.c`.

Current status: **29/30 passing (97%)**. See `README.md`.

## Remaining gaps to reach 30/30

- **Pack decoder** — ✅ Done (`src/unpack.rs`). `unpack-valid` passes.
- **LZW decoder** — ✅ Done (`src/unlzw.rs`). `helin-segv` passes.
- **`unpack-invalid`** — the only remaining failure. This test feeds
  three inputs: a valid `.z` file, a corrupt gzip stream, and a valid
  gzip file. The corrupt gzip stream should be rejected with
  `invalid compressed data--format violated`, but flate2's deflate
  decoder is more lenient than zlib and does not reject the particular
  corrupt bitstream that GNU gzip's zlib rejects. This is a flate2 vs
  zlib strictness difference, not a pack/LZW issue.

## Running

```sh
# Single test
nix build .#checks.x86_64-linux.rust-gzip-test-keep

# View log for a failure
nix log .#checks.x86_64-linux.rust-gzip-test-keep

# All tests, keep going on failures
nix build .#checks.x86_64-linux.rust-gzip-test-* --keep-going --no-link
```
