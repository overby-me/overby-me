# rust-bash

A Bash-compatible shell written in Rust, targeting full compatibility with GNU Bash 5.3.

## Status

**77/77 nix integration tests passing.** The shell passes the full Bash 5.3 test suite used in the nix harness, covering aliases, arithmetic, arrays (indexed and associative), brace expansion, builtins, case statements, command substitution, conditionals, coprocesses, extglob, functions, globbing, heredocs, here-strings, IFS handling, mapfile, namerefs, parameter expansion, pattern matching, pipelines, printf, process substitution, quoting, redirections, traps, and more.

See [`CHANGELOG.md`](CHANGELOG.md) for the full fix history (300+ fixes across 117 phases).

## Features

- POSIX-compatible command execution with Bash extensions
- Indexed and associative arrays
- Nameref variables (`declare -n`)
- Arithmetic evaluation (`(( ))`, `$(( ))`, `let`)
- Extended globbing (`extglob`, `globstar`)
- Process substitution (`<()`, `>()`)
- Coprocesses (`coproc`)
- Here documents and here strings
- Brace expansion
- Command substitution (`` `...` `` and `$(...)`)
- Variable redirections (`{varname}>file`)
- `set -e`, `set -x`, pipefail, lastpipe
- Builtins: `declare`, `local`, `export`, `unset`, `read`, `printf`, `mapfile`, `trap`, `type`, `command`, `hash`, `enable`, `shopt`, `set`, `getopts`, `select`, and more
- Built-in help data extracted from Bash 5.3 `.def` files

## Building

Requires a Rust toolchain (edition 2024).

```bash
cargo build           # debug build
cargo build --release # optimized build with LTO
```

The binary is named `bash` and also symlinks to `sh` when installed via Nix.

### With Nix

```bash
nix build .#rust-bash          # release build
nix build .#rust-bash-dev      # debug build (faster compile)
```

## Testing

### Nix test harness (77 tests)

```bash
# Run a single test
nix build .#checks.x86_64-linux.rust-bash-test-NAME

# Run all tests (continue on failure)
nix build --keep-going .#checks.x86_64-linux.rust-bash-test-{alias,appendop,arith,array,...}

# View failure diff
nix log .#checks.x86_64-linux.rust-bash-test-NAME
```

### Local testing

```bash
cd /tmp/bash-5.3/tests
export THIS_SH=/path/to/target/debug/bash

# Single test diff against reference bash
diff <("$THIS_SH" ./NAME.tests 2>&1) <(bash ./NAME.tests 2>&1)

# Run all 83 local tests
for test in $(ls *.tests | sed 's/.tests$//' | sort); do
  diff_lines=$(timeout 60 diff <("$THIS_SH" ./${test}.tests 2>&1) \
    <(bash ./${test}.tests 2>&1) 2>&1 | wc -l)
  [ "$diff_lines" -gt 0 ] && echo "DIFF($diff_lines): $test" || echo "OK: $test"
done
```

**Note:** Some tests show spurious diffs when run in parallel due to race conditions on shared temp files. Run sequentially for accurate results. The `ifs-posix` test takes ~4 minutes (6856 subtests) — use `timeout 300`.

## Architecture

| Directory / File | Description |
|---|---|
| `src/main.rs` | Entry point, SIGPIPE handling, interactive/script/stdin modes |
| `src/ast.rs` | AST types (`WordPart`, `Command`, `ForClause`, etc.) |
| `src/lexer/` | Tokenizer — heredocs, `${}` parsing, `$(...)` comsub, compound arrays |
| `src/parser.rs` | Recursive-descent parser, array elements, heredoc body resolution |
| `src/expand/` | Word expansion, parameter expansion (`${...}` operators), IFS splitting, pattern matching, arithmetic expansion, process substitution |
| `src/interpreter/` | Shell state, command execution, pipelines, redirections, arithmetic eval, variable/nameref resolution |
| `src/builtins/` | Built-in commands — `declare`, `read`, `printf`, `trap`, `type`, `set`, `shopt`, etc. |
| `default.nix` | Nix package definitions (release and dev builds) |
| `testsuite.nix` | Nix test harness with path/PID normalization |

## Dependencies

- [`nix`](https://crates.io/crates/nix) — Unix syscalls (process, signal, fd, poll)
- [`glob`](https://crates.io/crates/glob) — Filename globbing
- [`libc`](https://crates.io/crates/libc) — Low-level C bindings
- [`regex-lite`](https://crates.io/crates/regex-lite) — Lightweight regex (for `=~` operator)

## License

MIT