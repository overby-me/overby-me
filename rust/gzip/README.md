# rust-gzip

A GNU gzip-compatible compression tool written in Rust.

## Status

**27/30 tests passing (90%)** — upstream GNU gzip 1.14 test suite
(`tests/TESTS`). Each test runs rust-gzip in a sandbox against the
upstream shell script unchanged, using pre-built zdiff / zgrep / znew
from `pkgs.gzip` for the companion tools (those scripts call `gzip` by
name on PATH, so they pick up rust-gzip).

### Passing

`gzip-env`, `help-version`, `hufts`, `keep`, `list`, `list-big`,
`memcpy-abuse`, `mixed`, `null-suffix-clobber`, `pipe-output`,
`reference`, `reproducible`, `stdin`, `synchronous`, `timestamp`,
`trailing-nul`, `two-files`, `upper-suffix`, `write-error`, `z-suffix`,
`zdiff`, `zgrep-abuse`, `zgrep-binary`, `zgrep-context`, `zgrep-f`,
`zgrep-signal`, `znew-k`.

### Remaining

- `unpack-valid`, `unpack-invalid` — exercise GNU `pack` format
  (magic `1f 1e`, static Huffman). Needs a Huffman decoder port.
- `helin-segv` — exercises Unix `compress` / `.Z` format (magic
  `1f 9d`, LZW). Needs an LZW decoder port.

## Usage

```sh
# Single test
nix build .#checks.x86_64-linux.rust-gzip-test-keep

# View a failing test's log
nix log .#checks.x86_64-linux.rust-gzip-test-keep

# All tests, keep going on failures
nix build .#checks.x86_64-linux.rust-gzip-test-* --keep-going --no-link
```

The binary is `gzip` from `pkgs.rust-gzip` (release) or
`pkgs.rust-gzip-dev` (debug, faster compile). `gunzip` and `zcat` are
installed as symlinks.

## Architecture

Single-file `src/main.rs`:

- `parse_args` — GNU-style option parsing with `--long`, clustered
  short options (`-dckv`), program-name dispatch (`gunzip`/`zcat`).
- `compress_stream` — hand-rolled gzip framing: writes the 10-byte
  header with OS=3 (Unix) and either a recorded source mtime or 0,
  delegates deflate to `flate2::write::DeflateEncoder`, and emits the
  CRC32 + ISIZE trailer. Control over the exact header bytes is what
  gets `reference` / `reproducible` byte-for-byte against upstream.
- `decompress_stream` — buffers the full input, then walks member
  boundaries manually using `flate2::Decompress` plus a small
  `parse_gzip_header` for flag-aware header parsing. This handles
  multi-member archives, trailing NUL padding (tape convention), and
  `-cdf` cat-style pass-through for non-gzip content.
- `CountingReader` — thin adapter over `Read` that tallies bytes for
  `-l` on stdin without buffering the whole stream.
- Error-path normalization via `canonical_decode_error` maps
  flate2's wording to GNU gzip's
  `invalid compressed data--format violated` / `not in gzip format`
  / `unexpected end of file` so stderr comparisons pass.

## Features

- Compress/decompress/test/list with stdin, single file, multiple
  files, and directory recursion (`-r`).
- Suffix handling: `-S <suffix>` / `--suffix`, default `.gz`, canonical
  alternates (`.tgz`, `.z`, `.Z`, `-gz`, `-z`, `_z`). Empty suffix is
  rejected with `invalid suffix ''`.
- Exit-code semantics: 0 on success, 2 for warnings (out-of-range
  source mtime), 1 on hard error. Multi-file runs keep the worst code.
- `--help` / `--version` write to stdout and exit 1 if the write fails
  (e.g. `/dev/full`), matching coreutils conventions.
- `--synchronous`, `---presume-input-tty` accepted as no-ops (the
  test harness relies on both being recognized).
- Multi-member gzip streams fully supported; trailing NUL padding
  after a valid stream is silently tolerated.
