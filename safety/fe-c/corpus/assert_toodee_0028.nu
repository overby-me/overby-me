#!/usr/bin/env nu
# Asserts RUSTSEC-2021-0028 / CVE-2021-28028 against real toodee 0.2.0, in BOTH
# modes. `insert_row` reserves space based on the iterator's len() but writes the
# actual items yielded; a Liar iterator (claims 2, yields 100) drives a
# `ptr::write` past the reserved Vec buffer. The overrunning write is caught by
# the write-call extent check (not mode-gated), resolved from the as_mut_ptr
# root, so both modes abort OutOfBounds. Note: escape `\(exit ...\)` — nushell
# runs `(exit ...)` as a subexpression (MEMORY: fe-c-nushell-assert-exit-footgun).

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the OOB write \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the insert_row overrun was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"($label) mode: expected OutOfBounds, got: ($viol)"}
  }
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tb = (check-mode "through" $through_log $through_exit)
  let cb = (check-mode "case" $case_log $case_exit)
  print $"toodee-0028 OK: both modes abort OutOfBounds on the insert_row overrun \(through base ($tb), case base ($cb)\); resolved from the as_mut_ptr root"
}
