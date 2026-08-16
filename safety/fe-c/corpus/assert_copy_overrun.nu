#!/usr/bin/env nu
# Asserts the write-intrinsic extent check (point 0, write path) in BOTH modes.
# A ptr::copy_nonoverlapping copies 32 bytes into an 8-byte buffer; the
# destination base is in bounds but the write extent overruns, so the extent
# check aborts OutOfBounds naming the owning buffer. A single-address
# destination check would pass. Write checks are not mode-gated, so both modes
# abort. Note: escape `\(exit ...\)` in messages — nushell runs `(exit ...)` as
# a subexpression (see MEMORY: fe-c-nushell-assert-exit-footgun).

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the overrun \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the copy overrun was not caught"}
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
  print $"copy-overrun OK: both modes abort OutOfBounds on the write overrun \(through base ($tb), case base ($cb)\) via the write-extent check"
}
