# Changelog

All notable changes to `rust-pcre2`.

## Unreleased

### Matcher

- Detect patterns with nested unbounded quantifiers at match time; return
  `Error::MatchLimit` on inputs ≥ 20 bytes when the match would otherwise
  fail, mirroring PCRE2's exponential-backtracking safeguard. Short inputs
  and successful matches are unaffected.
- `dollar_endonly` compile option: `$` matches only the absolute end of the
  subject, never before a trailing newline. Used by `grep -z` mode.
- Reject backreferences to non-existent capture groups at compile time with
  an error message matching GNU grep's format.
- Continuation-passing-style backtracking NFA. Greedy quantifiers try longer
  matches first and fall back to shorter ones; lazy quantifiers reverse the
  order. Supports atomic groups (no backtracking once matched).
- Documented limitation: nested unbounded quantifiers like `((a+)*)+$`
  cannot be fully backtracked through group boundaries by the current
  recursive approach — fixed via the detection above.
- Match limit (step counter) and depth limit (recursion) enforcement.

### Parser / AST

- Parse PCRE2 syntax into an AST: literals, escapes (`\d`, `\w`, `\s`, `\b`,
  `\n`, `\x{HH}`), character classes including POSIX, anchors (`^`, `$`,
  `\A`, `\Z`, `\z`), quantifiers (greedy/lazy/possessive), grouping
  (capturing, non-capturing, atomic, lookahead, lookbehind), alternation,
  backreferences (`\1`, `\k<name>`), and inline options.

### Integration

- Consumed by `rust/grep` for `-P` mode; `grep` passes 121/121 GNU grep 3.12
  tests.

## Not yet implemented

- DFA matcher (`pcre2_dfa_match` equivalent).
- `pcre2_substitute`.
- Full Unicode property database (only basic categories supported).
- JIT compilation.
- Upstream PCRE2 test-suite integration via Nix (`testsuite.nix`).
- `pcre2test` binary (stub only).
