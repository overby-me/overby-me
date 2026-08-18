#!/usr/bin/env nu
# Asserts the B5 / I9 heap-escape result: a raw pointer to a stack local is
# captured by-move into a boxed closure kept past the frame; when the closure
# is invoked later, the dereference of the now-dead local aborts with a
# UseAfterScopeExit naming the escaped local's own address and the capture site
# (escaped_at) — the RUSTSEC-2021-0128 closure shape.

def main [log: path, exit_code: int] {
  let lines = (open $log | lines)

  if $exit_code == 0 {
    error make {msg: $"reproducer did not abort \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: "reached NO_ABORT: the closure's stale access was not trapped"}
  }

  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=UseAfterScopeExit") {
    error make {msg: $"expected UseAfterScopeExit, got: ($viol)"}
  }

  let sl = (
    $lines | where {|l| $l | str starts-with "STACK_LOCAL="} | first
    | str replace "STACK_LOCAL=" "" | str trim
  )
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0)
  if $base != $sl {
    error make {msg: $"report named ($base), expected the escaped stack local ($sl)"}
  }

  # F7: the report must name the capture site the pointer escaped at.
  let esc = ($viol | parse --regex 'escaped_at=(?<e>[0-9]+)' | get e.0?)
  if ($esc | is-empty) or (($esc | into int) == 0) {
    error make {msg: $"report did not name the escape site (escaped_at); got: ($viol)"}
  }

  print $"closure-escape OK: aborted UseAfterScopeExit naming the dead stack scope ($sl), escaped_at line ($esc)"
}
