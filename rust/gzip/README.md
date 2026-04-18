# rust-gzip

A GNU gzip-compatible compression tool written in Rust.

Passes **30/30** tests from the upstream GNU gzip 1.14 test suite, run
unmodified against the rust-gzip binary in a Nix sandbox.

## Usage

The binary is `gzip` from `pkgs.rust-gzip` (release) or
`pkgs.rust-gzip-dev` (debug, faster compile). `gunzip` and `zcat` are
installed as symlinks.

```sh
# Compress / decompress
echo "hello" | gzip | gunzip

# From a Nix shell
nix run .#rust-gzip -- -d file.gz
```

## Features

- Compress / decompress / test / list with stdin, single file, multiple
  files, and directory recursion (`-r`).
- Suffix handling: `-S <suffix>` / `--suffix`, default `.gz`, canonical
  alternates (`.tgz`, `.z`, `.Z`, `-gz`, `-z`, `_z`).
- Exit-code semantics: 0 success, 2 warning (out-of-range mtime),
  1 hard error. Multi-file runs keep the worst code.
- `--help` / `--version` write to stdout; exit 1 on write failure
  (e.g. `/dev/full`), matching coreutils conventions.
- `--synchronous`, `---presume-input-tty` accepted as no-ops.
- Multi-member gzip streams; trailing NUL padding silently tolerated.
- Legacy format decompression:
  - **Pack** (magic `1f 1e`) — static Huffman, ported from `unpack.c`.
  - **LZW / compress** (magic `1f 9d`) — variable-width codes, ported
    from `unlzw.c`.

## Architecture

Multi-module `src/` layout:

| Module | Purpose |
|--------|---------|
| `main.rs` | Entry point |
| `cli.rs` | Argument parsing, `Mode` / `Options`, `--help` / `--version` |
| `compress.rs` | Gzip compression with hand-rolled framing (OS=3, exact mtime) |
| `decompress.rs` | Multi-format decompression (gzip, pack, LZW) with CRC32 + ISIZE trailer verification |
| `ops.rs` | File operations — compress / decompress / test / list for files and stdio |
| `unpack.rs` | Pack format decoder (magic `1f 1e`, static Huffman) |
| `unlzw.rs` | LZW / compress decoder (magic `1f 9d`, variable-width codes) |
| `util.rs` | `CountingReader`, suffix handling, output file creation |

### Implementation notes

- **zlib-rs backend** — flate2 is configured with the `zlib-rs` feature
  (pure Rust zlib rewrite) instead of the default miniz_oxide. This
  gives deflate validation strictness matching GNU gzip's own inflate
  (e.g. rejecting corrupt bitstreams that miniz_oxide silently accepts)
  while keeping the dependency tree C-free.
- **Hand-rolled gzip framing** — `compress_stream` writes the 10-byte
  header directly (OS=3, source mtime or 0), delegates deflate to
  `flate2::write::DeflateEncoder`, then appends the CRC32 + ISIZE
  trailer. This byte-exact control is what passes the `reference` and
  `reproducible` tests.
- **Member-by-member decoding** — `decompress_stream` buffers the full
  input and walks member boundaries using `flate2::Decompress` with a
  flag-aware header parser, rather than using `MultiGzDecoder`. This
  handles multi-member archives, trailing NUL padding, legacy format
  dispatch, and `-cdf` cat-style pass-through.
- **Deflate no-progress → format violated** — when zlib-rs consumes all
  input without reaching `StreamEnd`, the error is reported as
  `invalid compressed data--format violated` to match GNU gzip's own
  `inflate.c` behavior (structural Huffman errors, not premature EOF).

## Testing

Each of the 30 upstream test scripts runs as its own Nix check:

```sh
# Single test
nix build .#checks.x86_64-linux.rust-gzip-test-keep

# View a failure log
nix log .#checks.x86_64-linux.rust-gzip-test-keep

# All tests (keep going on failures)
nix build .#checks.x86_64-linux.rust-gzip-test-* --keep-going --no-link
```

### Test harness

The harness (`testsuite.nix`) extracts `pkgs.gzip.src`, builds a shadow
`$PATH` with `gzip` / `gunzip` / `zcat` pointing at `rust-gzip-dev` and
companion scripts (`zdiff`, `zgrep`, `znew`, …) from the upstream tarball,
exports the environment variables the gnulib `init.sh` framework expects
(`LC_ALL=C`, `srcdir`, `VERSION`, etc.), then runs each test script and
propagates its exit code. Exit 77 (automake "skip") is treated as pass.
