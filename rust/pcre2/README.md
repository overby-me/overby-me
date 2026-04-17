# rust-pcre2

A pure-Rust PCRE2 (Perl Compatible Regular Expressions 2) library.

Used by `rust/grep` for `-P` mode. Replaces `fancy-regex`, which lacks real
PCRE2 semantics (no backtrack limits, different match behavior).

## Scope

A practical subset of PCRE2 covering what `grep -P` needs — not a drop-in
replacement for all of PCRE2. See `CHANGELOG.md` for what's implemented; `bin/pcre2test.rs` is a stub.

## API

```rust
use rust_pcre2::{Regex, CompileOptions};

let re = Regex::new(r"\bword\b")?;
assert!(re.is_match(b"a word here")?);

let mut opts = CompileOptions::default();
opts.caseless = true;
let re = Regex::with_options(r"hello", opts)?;
```

Bytes, not `&str`: the API works on `&[u8]` to match PCRE2's byte-oriented
semantics and handle non-UTF-8 input. Set `match_limit` / `depth_limit` via
`Regex::set_match_limit` / `set_depth_limit` (defaults: 1M / 1000).

## Architecture

- **Parser** (`parse.rs`) — character-level recursive descent that emits an
  AST (`ast.rs`).
- **Matcher** (`matcher.rs`) — continuation-passing-style backtracking NFA
  operating directly on the AST; supports captures, backreferences,
  lookahead/lookbehind, atomic groups, and the PCRE2 anchors.
- **`lib.rs`** — `Regex`, `Match`, `CompileOptions`, `MatchContext`, and the
  find/iter surface.

### What's intentionally omitted

- JIT compilation
- 16-bit / 32-bit character modes
- EBCDIC support
- DFA matcher (`pcre2_dfa_match`)
- `pcre2_substitute`
- Full Unicode property database (only basic categories are supported —
  letters, decimal digits, whitespace, etc.)

### Catastrophic backtracking

The matcher doesn't do true exponential backtracking for classic pathological
patterns like `((a+)*)+`. Instead, it detects nested unbounded quantifiers at
compile time and returns `Error::MatchLimit` when such a pattern fails to
match on a non-trivial input (≥ 20 bytes). This mirrors PCRE2's observable
behaviour for `grep -P`.

## Building and testing

```sh
nix build .#rust-pcre2
cd rust/pcre2 && cargo test
```

The checked-in `default.nix` exposes the library package. The `pcre2test`
binary is a placeholder; there is no upstream-test integration yet.
