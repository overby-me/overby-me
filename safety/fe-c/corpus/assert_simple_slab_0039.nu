#!/usr/bin/env nu
# Asserts RUSTSEC-2020-0039 / CVE-2020-35892 against real simple-slab 0.3.2, in
# BOTH modes. Slab::index() is `&*(mem.offset(i))` with no bounds check, so
# indexing past the end reads out of bounds. The backing buffer is a
# libc::malloc allocation registered by cementite's interpose tier (A4); the
# opaque-origin root fix roots the malloc'd pointer; and the instrumented
# reborrow aborts OutOfBounds. Interposition + root fix + instrumentation on a
# real CVE. Note: escape `\(exit ...\)` and `\(A4\)` — nushell runs `(...)` as a
# subexpression (MEMORY: fe-c-nushell-assert-exit-footgun).

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the OOB read \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the out-of-bounds index read was not caught"}
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
  print $"simple-slab-0039 OK: both modes abort OutOfBounds on the unchecked index read \(through base ($tb), case base ($cb)\); the interposed libc::malloc buffer was resolved via the opaque-origin root fix \(A4\)"
}
