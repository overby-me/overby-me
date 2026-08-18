# Rules considered and rejected

## mojo-bare-except

Flags `except:` with no exception type, which swallows everything. Measured
before writing: 11 sites in this tree, and all 11 are correct.

Seven are `except: pass` inside `__del__`. A Mojo destructor cannot propagate,
so swallowing there is the only option the language leaves. Three more are a
try-next-candidate loop in `_lib.mojo`, which tries each `-L` path in
`NIX_LDFLAGS` and raises a real error after the loop if none held. The last
returns a sentinel from a WASM host function whose C caller reads the return
value rather than an exception.

A rule that is wrong 11 times out of 11 is the deslop failure mode this layer
exists to avoid, so it was not written.

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

## Rejections from the 2026-08 mining pass

Recorded with their measurements in ../CANDIDATES.md ("Triage verdicts"):
fixed-/tmp literals (63 sandbox-internal fps), Vec::remove(0)-in-loop (7/7
bounded LRU), flush-in-loop (6/6 decoder DPB.flush - a domain method, the
name-matching mistake deslop made), equality-or chains (no regex
backreferences + zero surface), bare always-true closures (11/11 Dioxus
predicates), general allocation arithmetic (45 benign graphics sites),
commented-out code (the deepcomment pass owns it), detached tokio::spawn
(rustc must_use covers it), CString-temporary as_ptr (rustc
dangling_pointers_from_temporaries covers it - verified by compiling).
