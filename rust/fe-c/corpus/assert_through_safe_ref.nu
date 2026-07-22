#!/usr/bin/env nu
# Asserts the C1 through/case mode distinction on a safe-reference
# use-after-scope: a closure captures a safe &u64 to a stack local and reads it
# back after the frame dies. `through` mode CHECKS the safe dereference and
# aborts; `case`-like mode (FEC_MODE unset) ELIDES it and runs clean. The one
# bolded row of the both-modes table, made executable.

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tlines = (open $through_log | lines)

  # --- through mode: must abort UseAfterScopeExit naming the escape site ---
  if $through_exit == 0 {
    error make {msg: $"through mode did not abort (exit ($through_exit)); log: ($tlines)"}
  }
  if ($tlines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: "through mode reached NO_ABORT: the safe-deref UAF was not checked"}
  }
  let viol = ($tlines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"through mode: no fe-c-violation line; log: ($tlines)"}
  }
  if not ($viol | str contains "kind=UseAfterScopeExit") {
    error make {msg: $"through mode: expected UseAfterScopeExit, got: ($viol)"}
  }
  let sl = (
    $tlines | where {|l| $l | str starts-with "STACK_LOCAL="} | first
    | str replace "STACK_LOCAL=" "" | str trim
  )
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0)
  if $base != $sl {
    error make {msg: $"through mode named ($base), expected the escaped local ($sl)"}
  }
  let esc = ($viol | parse --regex 'escaped_at=(?<e>[0-9]+)' | get e.0?)
  if ($esc | is-empty) or (($esc | into int) == 0) {
    error make {msg: $"through mode did not name the escape site; got: ($viol)"}
  }

  # --- case-like mode: must run clean (the safe deref is elided) ---
  let clines = (open $case_log | lines)
  if $case_exit != 0 {
    error make {msg: $"case-like mode aborted (exit ($case_exit)); it must elide the safe deref; log: ($clines)"}
  }
  if not ($clines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"case-like mode did not reach NO_ABORT; log: ($clines)"}
  }

  print $"through-safe-ref OK: through aborts UseAfterScopeExit \(escaped_at ($esc)\), case-like elides the safe deref — the mode distinction"
}
