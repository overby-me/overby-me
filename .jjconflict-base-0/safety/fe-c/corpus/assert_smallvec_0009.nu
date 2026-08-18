#!/usr/bin/env nu
# Asserts RUSTSEC-2019-0009 against real smallvec 0.6.9, in BOTH modes. Calling
# SmallVec::grow(current_capacity) on a spilled (heap) SmallVec frees the data
# and leaves the SmallVec pointing at freed memory; the subsequent read of the
# freed buffer aborts UseAfterFree. `through` checks the read directly; `case`
# re-checks it because it is dealloc-reachable (it follows the grow call). Note:
# escape `\(exit ...\)` — nushell runs `(exit ...)` as a subexpression (see
# MEMORY: fe-c-nushell-assert-exit-footgun).

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the UAF \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the freed-buffer read was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=UseAfterFree") {
    error make {msg: $"($label) mode: expected UseAfterFree, got: ($viol)"}
  }
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the freed allocation; got: ($viol)"}
  }
  $base
}

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tb = (check-mode "through" $through_log $through_exit)
  let cb = (check-mode "case" $case_log $case_exit)
  print $"smallvec-0009 OK: both modes abort UseAfterFree on the freed buffer \(through base ($tb), case base ($cb)\) after SmallVec::grow"
}
