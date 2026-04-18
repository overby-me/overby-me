# rust-bzip2 Testing Plan

## Goal

Validate the Rust bzip2 rewrite against the upstream bzip2 test suite using Nix
checks, following the same differential-testing pattern established by rust/awk.

## Upstream Test Suite

The upstream bzip2 1.0.8 source (pkgs.bzip2.src) ships six reference files:

| File | Description |
|------|-------------|
| sample1.ref | Uncompressed reference data for test 1 |
| sample2.ref | Uncompressed reference data for test 2 |
| sample3.ref | Uncompressed reference data for test 3 |
| sample1.bz2 | Known-good compressed output (level -1) |
| sample2.bz2 | Known-good compressed output (level -2) |
| sample3.bz2 | Known-good compressed output (level -3) |

The upstream make test performs six operations:

1. Compress each sampleN.ref at level -N to sampleN.rb2
2. Decompress each sampleN.bz2 to sampleN.tst
3. Compare compressed output against known-good: cmp sampleN.bz2 sampleN.rb2
4. Compare decompressed output against original: cmp sampleN.tst sampleN.ref

## Testing Strategy

### Phase 1: Upstream Sample Tests (Nix Checks)

Create a testsuite.nix that mirrors the awk pattern:

- Use pkgs.bzip2.src to get the upstream source tarball (no vendored test data)
- Extract the sample/reference files from it
- Run each test as a separate pkgs.runCommand derivation for granular caching

Individual test checks:

| Check name | What it does |
|------------|--------------|
| rust-bzip2-test-compress-1 | Compress sample1.ref at -1, cmp against sample1.bz2 |
| rust-bzip2-test-compress-2 | Compress sample2.ref at -2, cmp against sample2.bz2 |
| rust-bzip2-test-compress-3 | Compress sample3.ref at -3, cmp against sample3.bz2 |
| rust-bzip2-test-decompress-1 | Decompress sample1.bz2, cmp against sample1.ref |
| rust-bzip2-test-decompress-2 | Decompress sample2.bz2, cmp against sample2.ref |
| rust-bzip2-test-decompress-3 | Decompress sample3.bz2 (with -s), cmp against sample3.ref |

### Phase 2: Differential Tests Against Reference bzip2

Beyond the static sample files, add differential tests that compare rust-bzip2
against the reference pkgs.bzip2 on various inputs:

| Check name | What it does |
|------------|--------------|
| rust-bzip2-test-roundtrip-text | Compress then decompress text data, verify identity |
| rust-bzip2-test-roundtrip-binary | Compress then decompress binary data, verify identity |
| rust-bzip2-test-stdin-stdout | Pipe through stdin/stdout, compare with reference bzip2 |
| rust-bzip2-test-integrity | bzip2 -t on known-good .bz2 files passes |
| rust-bzip2-test-symlinks | bunzip2 and bzcat symlink behavior matches reference |
| rust-bzip2-test-flags | Verify -k, -f, -v, -q flag behavior |

### Phase 3: Edge Cases and Robustness

| Check name | What it does |
|------------|--------------|
| rust-bzip2-test-empty | Compress/decompress empty input |
| rust-bzip2-test-large | Handle files larger than one bzip2 block (900kB default) |
| rust-bzip2-test-all-levels | Compress at every level (1-9), decompress, verify identity |
| rust-bzip2-test-force-overwrite | -f overwrites existing files |
| rust-bzip2-test-keep | -k preserves input files |
| rust-bzip2-test-bad-input | Graceful error on corrupt/non-bz2 input |

## Code Refactoring

Before implementing tests, refactor main.rs into modules:

| Module | Contents |
|--------|----------|
| main.rs | Entry point, CLI dispatch |
| cli.rs | Config struct and parse_args() |
| compress.rs | compress() function |
| decompress.rs | decompress() function |
| process.rs | process_file() and stdin/stdout handling |

This enables unit testing of individual components and keeps the codebase
maintainable as features are added.

## Nix Integration

### File: default.nix

Add a rust-bzip2-dev package (debug build for fast compile) and a checks
attrset mapping test names to derivations, same pattern as rust/awk/default.nix.

### File: testsuite.nix

A pkgs.runCommand derivation parameterized by test name. Extracts
pkgs.bzip2.src, runs the appropriate test, and uses cmp or diff to
verify results.

## Running Tests

Single test:
  nix build .#checks.x86_64-linux.rust-bzip2-test-compress-1

All bzip2 tests:
  nix build .#checks.x86_64-linux.rust-bzip2-test-{compress,decompress}-{1,2,3}
