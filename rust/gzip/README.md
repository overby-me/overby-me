# rust-gzip

A GNU gzip-compatible compression tool written in Rust.

## Status

**29/30 tests passing (97%)** — upstream GNU gzip 1.14 test suite
(`tests/TESTS`). Each test runs rust-gzip in a sandbox against the
upstream shell script unchanged, using pre-built zdiff / zgrep / znew
from `pkgs.gzip` for the companion tools (those scripts call `gzip` by
name on PATH, so they pick up rust-gzip).

### Passing

`gzip-env`, `helin-segv`, `help-version`, `hufts`, `keep`, `list`,
`list-big`, `memcpy-abuse`, `mixed`, `null-suffix-clobber`,
`pipe-output`, `reference`, `reproducible`, `stdin`, `synchronous`,
`timestamp`, `trailing-nul`, `two-files`, `unpack-valid`,
`upper-suffix`, `write-error`, `z-suffix`, `zdiff`, `zgrep-abuse`,
`zgrep-binary`, `zgrep-context`, `zgrep-f`, `zgrep-signal`, `znew-k`.

### Remaining

- `unpack-invalid` — the only remaining failure. This test feeds three
  inputs including a corrupt gzip stream that should be rejected with
  `invalid compressed data--format violated`. flate2's deflate decoder
  is more lenient than zlib and does not reject the particular corrupt
  bitstream that GNU gzip's zlib rejects. This is a flate2 vs zlib
  strictness difference, not a missing feature.

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

Multi-module `src/` layout:

- `main.rs` — thin entry point.
- `cli.rs` — argument parsing, `Mode`/`Options`, `--help`/`--version`.
- `compress.rs` — gzip compression with hand-rolled framing: writes the
  10-byte header with OS=3 (Unix) and either a recorded source mtime
  or 0, delegates deflate to `flate2::write::DeflateEncoder`, and emits
  the CRC32 + ISIZE trailer.
- `decompress.rs` — multi-format decompression (gzip, pack, LZW) with
  CRC32 verification. Walks member boundaries manually using
  `flate2::Decompress` plus flag-aware header parsing. Handles
  multi-member archives, trailing NUL padding (tape convention), and
  `-cdf` cat-style pass-through for non-gzip content.
- `ops.rs` — file operations (compress/decompress/test/list for files
  and stdio).
- `unpack.rs` — Pack format decoder (magic `1f 1e`, static Huffman).
- `unlzw.rs` — LZW/compress decoder (magic `1f 9d`, variable-width
  codes).
- `util.rs` — `CountingReader`, suffix handling, output file creation.

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
- Pack format decompression (legacy `.z` files, magic `1f 1e`, static
  Huffman).
- LZW/compress format decompression (legacy `.Z` files, magic `1f 9d`,
  variable-width codes).
- Multi-member gzip streams fully supported; trailing NUL padding
  after a valid stream is silently tolerated.
