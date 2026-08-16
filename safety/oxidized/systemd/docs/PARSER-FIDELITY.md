# Parser fidelity: where the unit parser diverges from C

The unit-file parser reads root-trusted input, so its behaviour on malformed or
edge-case input is a fidelity concern in its own right, separate from the
concurrency model in [ARCHITECTURE.md](ARCHITECTURE.md) and the per-test gaps in
[TEST-OVERRIDES.md](TEST-OVERRIDES.md). This file records the known divergences
from C as documented debt, each carrying its consequence and the decision or work
it is waiting on.

## Strict error model: a bad directive value drops the whole unit

**Status:** characterised, decision pending (task #31). Not yet implemented.

**C is lenient.** For a known key with an unparseable value, C's fragment parser
(`src/core/load-fragment.c`) logs `Failed to parse <key>, ignoring: <value>` at
`LOG_WARNING` and returns 0, leaving the setting at its default. The unit still
loads with the rest of its configuration. This is pervasive: roughly 42
`log_syntax(... "Failed to parse ... ignoring")` sites cover the numeric, enum,
and structured value parsers.

**Rust is strict.** The unit parsers under
`crates/libsystemd/src/units/unit_parsing/` return a `ParsingErrorReason` from
the offending value parse and propagate it with `?` (for example the
`LimitNOFILE` and `TasksMax` `.map_err(...)?` sites in `service_unit.rs`, plus
`SettingTooManyValues` and `UnknownSetting`). The top-level loader then treats
any parse error as fatal to the whole file:

```rust
// crates/libsystemd/src/units/loading/mod.rs
let parsed_file = match parse_file(&raw) {
    Ok(pf) => pf,
    Err(e) => {
        warn!("Skipping unit {:?}: could not parse file: {:?}", entry_path, e);
        continue;
    }
};
```

**Consequence.** A unit that C loads (with a warning, the bad directive
defaulted) is *entirely absent* under rust. One typo such as
`LimitNOFILE=infinityy` removes the whole unit rather than one directive. The
real-world hit is small because malformed units are rare, but it is a genuine
robustness and fidelity gap.

**Why it is not a quick fix.** Two reasons keep it off the incremental path:

1. It is a design decision. Lenient (match C, silently default garbage) versus
   strict (fail fast, surface config errors) is a real trade-off. The port's
   "follow upstream" method rule argues for lenient, but the change flips PID 1's
   core parser error model, so the maintainer should own it.
2. Done faithfully it is unbounded for a single change. Leniency means every
   value-parse `?` site must instead warn and fall back to that directive's C
   default, across all the section parsers, with a high blast radius.

**Recommended path** (for a human decision or a dedicated arc, not a drive-by):
decide lenient, then restructure the parsers to collect per-directive warnings
and skip only the offending directive while keeping the rest, instead of
propagating to a whole-unit skip. Add a differential test that feeds a unit with
one bad directive to both implementations and asserts both still load it.

## Related

- **[TEST-OVERRIDES.md](TEST-OVERRIDES.md)** — per-test functional gaps.
- Task #32 (`RestrictFileSystems=` parsed and reported but not enforced) is a
  different shape of fidelity debt: the value is accepted but the behaviour is a
  no-op, rather than the input being rejected.
