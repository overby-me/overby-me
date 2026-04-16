# rust-pcre2: Plan for a Pure Rust PCRE2 Implementation

## Goal

Implement PCRE2 (Perl Compatible Regular Expressions 2) as a pure Rust library
crate, usable as a drop-in replacement for the C PCRE2 library. The primary
consumer is `rust/grep` which needs PCRE2 for its `-P` (Perl regex) mode,
currently served by `fancy-regex` which lacks true PCRE2 semantics (no
backtracking limits, no JIT, different match behavior).

## Reference Implementation

- **Upstream:** https://github.com/PhilipHazel/pcre2 (v10.46)
- **Source size:** ~64,500 lines of C across 15 core modules
- **Test suite:** 31 test input files with ~9,000 patterns and expected outputs
- **API surface:** ~84 exported functions

## Architecture

### Crate Structure

```
rust/pcre2/
  Cargo.toml          # Library crate: rust-pcre2
  Cargo.lock
  default.nix         # Nix package + test checks
  testsuite.nix       # Test runner against upstream pcre2test output
  PLAN.md
  .deslop.toml
  src/
    lib.rs            # Public API (Regex, Match, CompileOptions, etc.)
    compile.rs        # Pattern compilation (pcre2_compile equivalent)
    parse.rs          # Regex parser → AST
    ast.rs            # Abstract syntax tree for regex patterns
    match_nfa.rs      # Backtracking NFA matcher (pcre2_match equivalent)
    match_dfa.rs      # DFA matcher (pcre2_dfa_match equivalent)
    class.rs          # Character classes, Unicode properties
    unicode.rs        # Unicode tables, case folding, scripts
    substitute.rs     # pcre2_substitute equivalent
    error.rs          # Error types and messages
    options.rs        # Compile/match options and flags
```

### Public API Design

```rust
pub struct Regex { ... }
pub struct Match<'a> { ... }
pub struct CompileContext { ... }
pub struct MatchContext { ... }

impl Regex {
    pub fn new(pattern: &str) -> Result<Regex, Error>;
    pub fn with_options(pattern: &str, options: u32) -> Result<Regex, Error>;
    pub fn is_match(&self, subject: &[u8]) -> Result<bool, Error>;
    pub fn find(&self, subject: &[u8]) -> Result<Option<Match>, Error>;
    pub fn find_at(&self, subject: &[u8], offset: usize) -> Result<Option<Match>, Error>;
    pub fn find_iter<'r, 's>(&'r self, subject: &'s [u8]) -> FindIter<'r, 's>;
    pub fn captures(&self, subject: &[u8]) -> Result<Option<Captures>, Error>;
    pub fn replace(&self, subject: &[u8], replacement: &[u8]) -> Result<Vec<u8>, Error>;
}

impl Match<'_> {
    pub fn start(&self) -> usize;
    pub fn end(&self) -> usize;
    pub fn as_bytes(&self) -> &[u8];
}

impl MatchContext {
    pub fn set_match_limit(&mut self, limit: u32);
    pub fn set_depth_limit(&mut self, limit: u32);
}
```

The API works on `&[u8]` (not `&str`) to match PCRE2's byte-oriented semantics
and handle non-UTF-8 data correctly.

## Test Strategy

### Upstream Test Integration (via Nix)

Tests compare `rust-pcre2` output against the reference C `pcre2test` output,
similar to how `rust/awk` tests against GNU gawk and `rust/grep` tests against
GNU grep.

**Test runner (`testsuite.nix`):**
1. Extract PCRE2 source from `pkgs.pcre2.src`
2. Build a `pcre2test`-compatible test driver from `rust-pcre2`
3. Run each `testinput{N}` through both C `pcre2test` and Rust version
4. Normalize output (strip timing info, normalize paths)
5. Compare with `diff`

**Test suites (31 files, ~9,000 patterns):**

| Test | Patterns | Description |
|------|----------|-------------|
| testinput1 | 1,368 | Basic non-UTF patterns |
| testinput2 | 2,171 | API, errors, internals |
| testinput3 | 26 | Locale-specific |
| testinput4 | 650 | UTF-8 patterns |
| testinput5 | 807 | Unicode properties |
| testinput6 | 967 | DFA matching (pcre2_dfa_match) |
| testinput7 | 547 | UTF with DFA |
| testinput8 | 78 | POSIX interface |
| testinput9 | 33 | Unicode properties (extended) |
| testinput10 | 181 | Unicode scripts |
| testinput11 | 83 | Features requiring 16/32-bit |
| testinput12 | 192 | JIT-specific |
| testinput13 | 5 | JIT limits |
| testinput14 | 18 | JIT-specific features |
| testinput15 | 58 | JIT non-default |
| testinput17-27 | ~1,900 | Extended features, edge cases |

### Test Commands

```
nix build .#checks.x86_64-linux.rust-pcre2-test-{N}
nix log .#checks.x86_64-linux.rust-pcre2-test-{N}
```

## Implementation Plan

### Phase 1: Parser and AST (~1,500 lines)

Build a PCRE2 pattern parser that produces an AST.

**Syntax to support:**
- Literals, escapes (`\d`, `\w`, `\s`, `\b`, `\n`, `\x{HH}`, etc.)
- Character classes (`[...]`, `[^...]`, POSIX classes)
- Anchors (`^`, `$`, `\A`, `\Z`, `\z`)
- Quantifiers (`*`, `+`, `?`, `{n}`, `{n,}`, `{n,m}`, lazy/possessive variants)
- Grouping (`(...)`, `(?:...)`, `(?<name>...)`, `(?=...)`, `(?!...)`, `(?<=...)`, `(?<!...)`)
- Alternation (`|`)
- Backreferences (`\1`, `\k<name>`)
- Atomic groups (`(?>...)`)
- Conditional patterns (`(?(cond)yes|no)`)
- Unicode properties (`\p{L}`, `\P{Lu}`, `\p{Script=Greek}`)
- Comments (`(?#...)`, extended mode)
- Options (`(?i)`, `(?m)`, `(?s)`, `(?x)`, inline and scoped)

**Target tests:** testinput1, testinput2 (compilation, error detection)

### Phase 2: Backtracking NFA Matcher (~2,000 lines)

Implement `pcre2_match` equivalent — the core backtracking matcher.

**Features:**
- Recursive/iterative backtracking with configurable limits
- Capturing groups and backreferences
- Lookahead/lookbehind (positive and negative)
- Atomic groups (no backtracking once matched)
- `match_limit` and `depth_limit` enforcement
- Partial matching support
- Start offset support (`find_at`)

**Target tests:** testinput1 (basic matching), testinput4 (UTF matching)

### Phase 3: Character Classes and Unicode (~1,500 lines)

**Features:**
- POSIX character classes (`[:alpha:]`, etc.)
- Unicode general categories (`\p{L}`, `\p{Lu}`, `\p{Nd}`)
- Unicode scripts (`\p{Greek}`, `\p{Han}`)
- Unicode case folding (case-insensitive matching)
- UTF-8 decoding/validation
- Unicode character database tables

**Target tests:** testinput5, testinput9, testinput10 (Unicode properties)

### Phase 4: DFA Matcher (~1,000 lines)

Implement `pcre2_dfa_match` equivalent — alternative matching algorithm.

**Features:**
- Finds all possible matches (longest and shortest)
- No backreference support (by design)
- No capture groups (only overall match)
- Workspace-based state management

**Target tests:** testinput6, testinput7 (DFA-specific tests)

### Phase 5: Substitution (~500 lines)

Implement `pcre2_substitute` for search-and-replace.

**Features:**
- Replacement strings with `$1`, `${name}` backreferences
- Global and single replacement modes
- Extended replacement syntax

### Phase 6: Integration with rust/grep (~200 lines)

Replace `fancy-regex` with `rust-pcre2` in `rust/grep/src/matcher.rs`:

```rust
// In matcher.rs
MatcherInner::Pcre2(rust_pcre2::Regex)
```

**Changes needed in rust/grep:**
- Add `rust-pcre2` as a dependency (path = "../pcre2")
- Add `Pcre2` variant to `MatcherInner`
- Update `build_matcher()` to use `rust-pcre2` for `-P` mode
- Update `raw_is_match()` and `raw_find_matches()` with Pcre2 arms
- Handle match limit errors (exit 2 with "backtracking limit" message)
- Remove `fancy-regex` dependency

**Expected result:** `pcre-abort` test passes (119/119 = 100%)

### Phase 7: JIT Compilation (optional, future)

A Cranelift-based JIT compiler for regex patterns. This is a major effort
and can be deferred — the NFA matcher is correct and sufficient for most use
cases.

## PCRE2 Feature Priority

### Must-have (for grep -P)

- [x] Pattern compilation with error reporting
- [ ] Backtracking match with limits
- [ ] Capturing groups and backreferences
- [ ] Lookahead/lookbehind
- [ ] Unicode UTF-8 support
- [ ] Character class handling (\d, \w, \s, POSIX, Unicode properties)
- [ ] Case-insensitive matching
- [ ] Multiline mode (^ and $ match line boundaries)
- [ ] Dotall mode (. matches newline)
- [ ] find_at() for offset-based matching
- [ ] Match limit / depth limit enforcement

### Nice-to-have

- [ ] DFA matching (pcre2_dfa_match)
- [ ] Substitution (pcre2_substitute)
- [ ] Named captures
- [ ] Atomic groups
- [ ] Conditional patterns
- [ ] Callout support
- [ ] Partial matching
- [ ] POSIX API compatibility layer

### Out of scope (initially)

- JIT compilation
- 16-bit and 32-bit character modes
- EBCDIC support
- BSR/newline convention options

## Estimated Effort

| Phase | Lines | Effort |
|-------|-------|--------|
| Phase 1: Parser/AST | ~1,500 | High |
| Phase 2: NFA Matcher | ~2,000 | Very High |
| Phase 3: Unicode | ~1,500 | Medium |
| Phase 4: DFA | ~1,000 | Medium |
| Phase 5: Substitution | ~500 | Low |
| Phase 6: grep integration | ~200 | Low |
| **Total** | **~6,700** | |

## Nix Integration

### default.nix

```nix
{
  packages.rust-pcre2 = { lib, rustPlatform }:
    rustPlatform.buildRustPackage {
      pname = "rust-pcre2";
      version = "0.1.0";
      src = lib.fileset.toSource { ... };
      cargoLock.lockFile = ./Cargo.lock;
      meta = {
        description = "A pure Rust implementation of PCRE2";
        license = lib.licenses.mit;
      };
    };

  checks = let
    testNames = [ "1" "2" "3" "4" "5" "6" "7" "8" "9" "10" ... ];
  in
    builtins.listToAttrs (map (name: {
      name = "rust-pcre2-test-${name}";
      value = pkgs: import ./testsuite.nix { inherit pkgs name; };
    }) testNames);
}
```

### testsuite.nix

```nix
{ pkgs, name }:
pkgs.runCommand "rust-pcre2-test-${name}" {
  nativeBuildInputs = [ pkgs.rust-pcre2 pkgs.pcre2 ... ];
  pcre2Src = pkgs.pcre2.src;
} ''
  tar xf $pcre2Src
  PCRE2_SRC=$(echo pcre2-*)

  # Run reference pcre2test
  ${pkgs.pcre2}/bin/pcre2test $PCRE2_SRC/testdata/testinput${name} > expected 2>&1

  # Run rust-pcre2 test driver
  ${pkgs.rust-pcre2}/bin/pcre2test $PCRE2_SRC/testdata/testinput${name} > actual 2>&1

  # Normalize and compare
  diff expected actual && touch $out
''
```

## References

- [PCRE2 documentation](https://www.pcre.org/current/doc/html/)
- [PCRE2 pattern syntax](https://www.pcre.org/current/doc/html/pcre2pattern.html)
- [PCRE2 API reference](https://www.pcre.org/current/doc/html/pcre2api.html)
- [PCRE2 source (GitHub)](https://github.com/PhilipHazel/pcre2)
- [fancy-regex (current Rust PCRE-like)](https://github.com/fancy-regex/fancy-regex) — what we're replacing
