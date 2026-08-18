# Rules considered and rejected

## rust-test-without-assertion

Flags a `#[test]` whose body contains no assertion. Written, measured, and
dropped: it fired 14 times on this tree and was wrong all 14.

Eight were robustness tests whose whole point is that something does not
panic — `every_saver_survives_being_poked`, `fuzz_parse_steps_malformed_yaml`.
The other six assert through a helper: `fix16.rs` tests call `close(value,
expected, tolerance)`, and the `assert!` lives inside `close`.

That second class is the fatal one. Deciding whether a test asserts means
following calls into helpers, which is dataflow, not syntax, and ast-grep has
no name resolution. Narrowing the rule to tests that make no calls at all
would leave it matching almost nothing.

Worth noting where the real instance was caught instead. This tree did ship a
test that could not fail — `test_clamp_does_not_overflow` asserted `r <= 255`
on a `u8` — and clippy found it, but only sideways, through
`absurd_extreme_comparisons` complaining about the comparison rather than
about the test. Neither tool has a rule for the thing itself.
