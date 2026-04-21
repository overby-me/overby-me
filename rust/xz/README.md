# rust-xz

A pure-Rust `xz(1)` CLI built on top of the `liblzma` crate (which
itself bundles upstream `xz-utils` 5.8.x). Aims to be drop-in
compatible with the upstream binary on the surface area exercised by
xz's own test suite. Passes **117/117** Nix checks built from the
upstream test artifacts plus four rust-xz–specific checks.

## Building

```sh
nix build .#rust-xz
./result/bin/xz --help
```

A debug build is also available as `.#rust-xz-dev` for quick
iteration. The standard symlinks (`unxz`, `xzcat`, `lzma`,
`unlzma`, `lzcat`) are installed alongside the main `xz` binary.

## Running the test suite

Tests run inside a Nix sandbox. The upstream test artifacts are
extracted from `pkgs.xz.src` (xz-utils 5.8.1) and pointed at our
`rust-xz-dev` binary. Each logical test is a separate Nix check
derivation so failures are isolated.

```sh
# Full suite (117 derivations).
nix flake check

# A single per-file decode check.
nix build .#checks.x86_64-linux.\"rust-xz-good-1-arm64-lzma2-1.xz\"

# An upstream shell script.
nix build .#checks.x86_64-linux.rust-xz-test-files

# An upstream C unit test (built from xz's own makefile).
nix build .#checks.x86_64-linux.rust-xz-test-vli

# View a failing log.
nix log .#checks.x86_64-linux.rust-xz-fuzz
```

In-tree `cargo` tests (81 total: 75 unit + 1 fuzz harness + 5
proptest properties) run without Nix:

```sh
cd rust/xz
cargo test
cargo bench --bench roundtrip   # criterion throughput suite
```

## Test inventory (117 Nix checks)

| Category | Count | Notes |
|-|-|-|
| Per-file decode (`good-*` / `bad-*` / `unsupported-*`) | 95 | Every file in `xz-5.8.1/tests/files/`. `good-*` must decode; `bad-*` and `unsupported-*` must be rejected. |
| Upstream shell scripts | 6 | `test_files.sh`, `test_suffix.sh`, `test_compress_generated_{abc,random,text}`, `test_scripts.sh` (skipped: needs `xzdiff`/`xzgrep` we don't ship) |
| Upstream C unit tests | 12 | `test_check`, `test_hardware`, `test_stream_flags`, `test_filter_flags`, `test_filter_str`, `test_block_header`, `test_index`, `test_index_hash`, `test_bcj_exact_size`, `test_memlimit`, `test_lzip_decoder`, `test_vli` — built via xz's own `Makefile` against upstream liblzma. |
| `rust-xz-roundtrip` | 1 | Random-input compress/decompress at every preset 0-9. |
| `rust-xz-list` | 1 | End-to-end smoke test of `xz -l` output. |
| `rust-xz-filters` | 1 | End-to-end BCJ + LZMA2 filter chain via `--x86`/`--filters=`. |
| `rust-xz-fuzz` | 1 | Decoder-stability harness: feeds every corpus file plus prefix-truncated and bit-flipped mutations through the decoder, asserting no panics. |

The `script` helper in `testsuite.nix` treats `exit 77` (skip) as a
**failure** by default. Only `test_scripts.sh` is allowed to skip
(`allowSkip = true`), because it needs the `xzdiff` and `xzgrep`
shell wrappers we don't ship. This keeps a misconfigured test from
silently flagging the matrix green — see the Round-2 false-pass
correction in `CHANGELOG.md`.

## Supported features

Everything exercised by the upstream test surface, plus a few
rust-xz-only conveniences:

### Container formats

- `.xz` (stream encoder + decoder, single + concatenated streams).
- `.lzma` (LZMA-alone encode + decode).
- `.lz` (lzip decode-only — `liblzma`'s lzip decoder; encoding not
  implemented).
- `--format=auto`/`xz`/`lzma`/`alone`/`lzip`/`raw` (`-F`/`-Fraw`).
- Raw filter-chain mode for both encode and decode (no container,
  no integrity check, no magic bytes).

### Filter chains

- `--lzma1=preset=N`, `--lzma2=preset=N` (the only `--lzmaN=` value
  upstream tests exercise; the `e` "extreme" suffix is silently
  stripped).
- `--filters="x86 lzma2:preset=4"` and `--filters=arm64--lzma2`
  (both space- and `--`-separated tokens accepted, matching upstream).
- Per-filter short flags: `--x86`, `--arm`, `--arm64`, `--armthumb`,
  `--powerpc`, `--ia64`, `--sparc`, `--riscv`, `--delta`.
- LZMA1/LZMA2 must be the final filter; chain validation rejects
  malformed orderings with a clear `InvalidInput`.

### CLI surface

- Mode flags: `-z`/`--compress` (default), `-d`/`--decompress`,
  `-t`/`--test`, `-l`/`--list`.
- Argv[0] dispatch: `unxz` → decompress, `xzcat` → decompress + stdout,
  `lzma` → compress with lzma format, `unlzma` → decompress,
  `lzcat` → decompress + stdout.
- Levels: `-0`..`-9`, `--fast`, `--best`.
- I/O: `-c`/`--stdout`, `-k`/`--keep`, `-f`/`--force`,
  `-v`/`--verbose`, `-q`/`--quiet`, `-Q`/`--no-warn`.
- Suffix: `-S`/`--suffix=.SUF` (replaces `.xz`/`.lzma` on output;
  required in `-F raw` mode for file outputs).
- Input lists: `--files=FILE` (newline-sep), `--files0=FILE`
  (NUL-sep), with auto-skip of empty entries.
- Threads: `-T`/`--threads=N` accepted-and-ignored (single-threaded
  for now).
- Memlimit: `--memlimit-compress=`, `--memlimit-decompress=`,
  `--no-adjust` accepted-and-ignored.
- `--`-terminated positionals.

### `-l`/`--list`

Mimics the upstream non-verbose output:

```text
Strms  Blocks   Compressed Uncompressed  Ratio  Check   Filename
    1       1         80 B         24 B  3.333  CRC64   h.xz
```

Counts streams by scanning for the xz header magic, decodes through
`io::sink()` to compute the uncompressed size, and reads the
integrity-check ID from the stream-flags byte. With `-lv` and
multiple files, a totals line is appended.

### `-dfc` passthrough

When `-d -f -c` are all set and the input does not look like a
recognised compressed stream (no `.xz` magic, no `LZIP` magic), the
bytes are copied verbatim to stdout — matching upstream xz's
"force-decode-cat passes plain text through" behaviour. Triggered
on `Format::Auto` after a 6-byte peek.

## Layout

```text
rust/xz/
  Cargo.toml
  default.nix         # Nix package + per-test flake checks
  testsuite.nix       # Per-test Nix sandbox runners (script/file/cTest/...)
  CHANGELOG.md
  README.md
  benches/
    roundtrip.rs      # Criterion throughput benches
  src/
    main.rs           # Thin shim: parse args, dispatch
    lib.rs            # Re-exports the modules so benches/tests/fuzz share code
    cli.rs            # Argv → Options
    codec.rs          # liblzma streaming encoders / decoders
    list.rs           # `xz -l` summary table
    options.rs        # Mode / Format / FilterChain / FilterKind types
    processor.rs      # Per-file driver (open, route, suffix dance, cleanup)
    suffix.rs         # Suffix mapping for compress / decompress
    bin/
      rust-xz-fuzz.rs # Standalone decoder-stability fuzz harness
  tests/
    fuzz_corpus.rs    # Local mirror of the Nix `rust-xz-fuzz` check
    proptest_roundtrip.rs # Property-based round-trip tests
```

## Performance

Quick smoke benchmark on a development laptop (256 KiB synthetic
payload, criterion `--quick`):

| Operation | Preset 0 | Preset 3 | Preset 6 | Preset 9 |
|-|-|-|-|-|
| Decompress | ~870 MiB/s | ~1.0 GiB/s | ~980 MiB/s | ~600 MiB/s |
| Compress | (varies) | (varies) | (varies) | (varies) |

Run `cargo bench --bench roundtrip` to reproduce. The numbers are
in the same order as system `xz`, since both ultimately drive the
same liblzma C code.

## Known limitations / future work

- Lzip compression (currently decode-only).
- Per-block detail in `--list -v` (`-vv` upstream walks the index
  records; we report one block per stream as a coarse approximation).
- A `--robot` machine-readable list output.
- True libFuzzer integration — the in-tree `rust-xz-fuzz` derivation
  only does corpus-replay + prefix-truncation + 1-byte-flip mutation.
- A CI bench-tracking job that runs `cargo bench` on each commit.
- Real `--memlimit-*` enforcement (currently accept-and-ignore).
