#!/usr/bin/env nu
# Asserts the B5 stack scope hook result (I8): the instrumented stack-UAF
# reproducer aborts with a UseAfterScopeExit violation naming the dead stack
# scope (the escaped local's address), rather than resolving the reused
# stack memory as valid.

def main [log: path, exit_code: int] {
  let lines = (open $log | lines)

  if $exit_code == 0 {
    error make {msg: $"reproducer did not abort \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: "reached NO_ABORT: the stale stack access was not trapped"}
  }

  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=UseAfterScopeExit") {
    error make {msg: $"expected UseAfterScopeExit, got: ($viol)"}
  }

  # The named allocation must be the stack local's own address.
  let sl = (
    $lines | where {|l| $l | str starts-with "STACK_LOCAL="} | first
    | str replace "STACK_LOCAL=" "" | str trim
  )
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0)
  if $base != $sl {
    error make {msg: $"report named ($base), expected the escaped stack local ($sl)"}
  }

  # I9 / F7: the report names where the address escaped the frame.
  let esc = ($viol | parse --regex 'escaped_at=(?<e>[0-9]+)' | get e.0?)
  if ($esc | is-empty) or (($esc | into int) == 0) {
    error make {msg: $"report did not name the escape site (escaped_at); got: ($viol)"}
  }

  print $"stack-uaf OK: aborted UseAfterScopeExit naming the dead stack scope ($sl), escaped_at line ($esc)"
}
