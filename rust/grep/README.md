# rust-grep

A pure-Rust reimplementation of GNU `grep(1)` that aims to be output-compatible
with the upstream tool. Passes **121/121** tests from the GNU grep 3.12 test suite.

Uses `rust-pcre2` (pure Rust) for `-P` (Perl-compatible) mode, `fancy-regex` for
BRE/ERE backreferences, and the `regex` crate otherwise.

## Building

```sh
nix build .#rust-grep
./result/bin/grep --help
```

The package installs `grep`, `egrep`, and `fgrep` (symlinks) into `$out/bin`.

## Running the test suite

Tests are run in a Nix sandbox. Each test script comes from the GNU grep source
tarball; rust-grep is placed first on `PATH` (as `grep`/`egrep`/`fgrep`) and the
script is executed against it.

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-grep-test-{name}

# View failure diff
nix log .#checks.x86_64-linux.rust-grep-test-{name}
```

See `default.nix` for the full list of test names. Tests time out after 120s.

## Supported features

All GNU grep 3.12 features exercised by the upstream test suite, including:

- Basic, extended, Perl (`-G`/`-E`/`-P`), and fixed-string (`-F`) matching
- Multiple patterns via repeated `-e` or `-f FILE`
- `-i`, `-v`, `-w`, `-x`, `-c`, `-l`/`-L`, `-m`, `-n`, `-b`, `-o`, `-H`/`-h`, `-q`, `-s`
- `-A`/`-B`/`-C NUM` and `-NUM` context
- `-r`/`-R` recursive search with `--include`, `--exclude`, `--exclude-dir`
- `-z` NUL-delimited I/O
- `--color=always/never/auto` with `GREP_COLORS` support
- Binary file detection, device-file skip (`-D skip`), `-a`/`--binary-files=text`
- `-T`/`--initial-tab`, `--label`, `--line-buffered`

## Known limitation

Patterns with nested unbounded quantifiers (e.g. `((a+)*)+`) trigger a
`MatchLimit`-style error in `-P` mode on non-trivial inputs, mirroring PCRE2's
exponential-backtracking safeguard. Short inputs and successful matches are
unaffected.

## Layout

```text
rust/grep/
  Cargo.toml
  default.nix       # Nix package + test checks
  testsuite.nix     # Per-test Nix sandbox runner
  CHANGELOG.md
  README.md
  src/
    main.rs         # CLI entrypoint and exit codes
    args.rs         # Argument parsing
    files.rs        # File and directory walking
    grep.rs         # Per-file matching, context, output formatting
    matcher.rs      # Pattern compilation and matching dispatch
    pattern.rs      # BRE-to-ERE conversion and pattern validation
```
