#!/usr/bin/env nu
# Asserts the raw->safe cast ensure (point 1, §3.1) in BOTH modes. A raw
# pointer past the end of a Vec buffer is cast to `&u64`; the cast ensure
# resolves the derivation root, sees the referent's extent is out of bounds,
# and aborts OutOfBounds. Both `case` (which elides the later derefs) and
# `through` (whose deref check resolves the off-the-end faulting address and so
# relies on the cast ensure) must abort.

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the OOB cast (exit ($exit_code)); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the OOB raw->safe cast was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"($label) mode: expected OutOfBounds, got: ($viol)"}
  }
  # The report resolves the owning Vec buffer, not the off-the-end address.
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tb = (check-mode "through" $through_log $through_exit)
  let cb = (check-mode "case" $case_log $case_exit)
  print $"cast-oob OK: both modes abort OutOfBounds on the raw->safe cast \(through base ($tb), case base ($cb)\) via the cast ensure"
}
