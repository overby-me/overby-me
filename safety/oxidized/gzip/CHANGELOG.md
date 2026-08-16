# Changelog

## 0.1.0

**30/30 upstream GNU gzip 1.14 tests passing.**

### Legacy format decoders

- Pack format decompressor (`unpack.rs`) — static Huffman decoder
  ported from GNU gzip's `unpack.c`. Passes `unpack-valid` and
  `unpack-invalid`.
- LZW / compress decompressor (`unlzw.rs`) — variable-width code
  decoder ported from GNU gzip's `unlzw.c`. Passes `helin-segv`.

### Deflate validation

- Switched flate2 backend from miniz_oxide to zlib-rs (pure Rust zlib
  rewrite) for stricter deflate validation matching GNU gzip.
- Added CRC32 + ISIZE trailer verification in the gzip member decoder.
- Deflate no-progress condition now reports "format violated" instead
  of "unexpected end of file", matching GNU gzip's own `inflate.c`.

### Compression

- Hand-rolled gzip framing with byte-exact header control (OS=3, source
  mtime, XFL). Passes `reference` and `reproducible` byte-for-byte.

### Decompression

- Member-by-member gzip decoding via `flate2::Decompress` with manual
  header/trailer parsing.
- Multi-member archive support.
- Trailing NUL padding tolerance (tape archive convention).
- `-cdf` cat-style pass-through for non-gzip content.
- Legacy format dispatch on magic bytes (`1f 1e` → pack, `1f 9d` → LZW).

### CLI

- GNU-style option parsing: `--long`, clustered short (`-dckv`),
  program-name dispatch (`gunzip` / `zcat`).
- Suffix handling (`-S` / `--suffix`), all canonical alternates.
- Compression levels `-1` through `-9`.
- `--help` / `--version` with stdout-write-failure exit code.
- `--synchronous`, `---presume-input-tty` accepted as no-ops.
- Directory recursion (`-r`), multi-file processing, `-` for stdin.

### Error compatibility

- `canonical_decode_error` maps flate2 / zlib-rs error strings to GNU
  gzip's canonical wording so upstream tests can compare stderr
  byte-for-byte.

### Testing

- Nix-based test harness (`testsuite.nix`) running each upstream test
  as an isolated `pkgs.runCommand` check.
- Companion shell scripts (`zdiff`, `zgrep`, `znew`, …) sourced from
  `pkgs.gzip` and invoked via PATH so they pick up rust-gzip.
