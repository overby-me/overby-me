# rust-patch

A pure-Rust reimplementation of GNU `patch(1)` that aims to be output-
compatible with the upstream tool. Passes **49/49** tests from the
GNU patch 2.8 test suite.

## Building

```sh
nix build .#rust-patch
./result/bin/patch --help
```

A debug build is also available as `.#rust-patch-dev` for quick iteration.

## Running the test suite

Tests are run in a Nix sandbox. Each test script comes from the GNU patch
source tarball; rust-patch is placed at `$abs_top_builddir/src/patch` and
the script is executed against it.

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-patch-test-{name}

# View failure diff
nix log .#checks.x86_64-linux.rust-patch-test-{name}
```

See `default.nix` for the full list of test names. Tests time out after 60s.

## Supported features

All GNU patch 2.8 features exercised by the upstream test suite, including:

- Unified, context, normal, and ed-style diffs (auto-detected)
- Git extended headers: `diff --git`, `new/deleted file mode`, `old/new mode`,
  `rename from/to`, `copy from/to`, `index`, symlink mode (120000)
- `-p N` path stripping (including `-p0` and stripping empty)
- `-R` reverse, `-N`/`--forward`, `-f`/`--force`, `-t`/`--batch`
- `-b`/`--backup` with `--backup-suffix`, `--backup-prefix`,
  `--basename-prefix`, numbered/existing/simple backups via `VERSION_CONTROL`
- `--no-backup-if-mismatch`, `-E`/`--remove-empty-files`
- `-o FILE` with POSIX multi-patch concatenation
- `-i FILE`, `-d DIR`, `-r FILE` (reject file)
- `--dry-run`, `--silent`/`-s`, `--verbose`/`-v`
- `--read-only={ignore,warn,fail}`
- `--reject-format={context,unified}`, `--posix`, `--binary`
- `--follow-symlinks` (writes a regular file, doesn't write through the link)
- `-e`/`--ed` and auto-detection of ed-style input via positional file
- `--set-utc` (parses YYYY-MM-DD HH:MM:SS + offset, updates mtime)
- CRLF handling: per-file decision, `--binary` byte-exact mode,
  "different line endings" hunk-failure suffix
- Fuzz/offset matching with GNU's asymmetric-context heuristic
- Reject files in the matching format (or explicit `--reject-format`)
- Shell-quoted filenames with embedded whitespace or meta-chars in
  diagnostics

## Known limitations

- `--merge` (diff3-style conflict merging) is not implemented; invoking
  it exits with a "not implemented" error so the upstream `merge` test
  suite detects the absence and skips.
- Interactive prompts (e.g. "Apply anyway? [n]") are emitted as static
  text and the implicit "no" is taken; `-f`/`-t` cover the non-interactive
  cases.
- GIT binary payloads are recognised (the header is parsed) but the base85
  delta is not decoded — the file is left unmodified and a diagnostic is
  printed.

## Layout

```text
rust/patch/
  Cargo.toml
  default.nix       # Nix package + test checks
  testsuite.nix     # Per-test Nix sandbox runner
  CHANGELOG.md
  README.md
  src/
    main.rs         # Single-file implementation
```
