#!/usr/bin/env nu
# Asserts the real RUSTSEC-2023-0016 catch. partial_sort 0.1.1 validates its
# `last` argument with a `debug_assert!`, elided in optimized/no-debug-assert
# builds, so `partial_sort(v, last, ..)` with `last > v.len()` walks its
# get_unchecked reads past the buffer (a read-only out-of-bounds, per the
# advisory, until the library's own bounds-checked write panics).
#
# This is a through-catches / case-elides entry — the spatial-OOB analog of
# through-safe-ref:
#   * `through` checks the safe-reference reads and aborts OutOfBounds at the
#     first out-of-bounds element, naming the owning Vec buffer (I10).
#   * `case` elides those safe derefs (the documented both-modes elision), so
#     fe-c does not catch the read-only over-read; execution proceeds until
#     partial_sort's own bounds-checked `v.swap` panics (the advisory's limiting
#     behavior). No fe-c-violation is emitted.

# through: must abort OutOfBounds naming the owning buffer.
def check-through [log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"through mode did not abort the partial_sort OOB \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"through mode reached NO_ABORT: the out-of-bounds read was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"through mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"through mode: expected OutOfBounds, got: ($viol)"}
  }
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"through mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

# case: the documented elision — fe-c must NOT emit a violation (case elides the
# safe-reference reads); the process still crashes, via partial_sort's own
# bounds-checked write, not via fe-c.
def check-case [log: path, exit_code: int] {
  let lines = (open $log | lines)
  if ($lines | any {|l| $l | str starts-with "fe-c-violation"}) {
    error make {msg: $"case mode emitted a fe-c-violation; the safe-deref elision no longer holds, so update this differential entry. log: ($lines)"}
  }
}

def main [
  through_log: path, through_exit: int,
  case_log: path, case_exit: int,
] {
  let base = (check-through $through_log $through_exit)
  check-case $case_log $case_exit
  print $"partial-sort-0016 OK: through aborted OutOfBounds naming the owning buffer ($base); case elided the safe-reference reads, emitting no fe-c-violation"
}
