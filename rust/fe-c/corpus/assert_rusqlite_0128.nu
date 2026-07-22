#!/usr/bin/env nu
# Asserts RUSTSEC-2021-0128 against real rusqlite 0.25.3 + bundled SQLite. A
# closure captures a stack borrow, is registered with SQLite, outlives the
# frame, and is invoked by SQLite (C) — reading the dropped local through a
# safe reference. `through` mode CHECKS that safe dereference and aborts,
# naming the dead scope and the registration site; `case`-like mode (FEC_MODE
# unset) ELIDES it and runs clean. Fe-C catching a real CVE in a real,
# unmodified third-party crate + its real C dependency.

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tlines = (open $through_log | lines)

  # --- through mode: must abort UseAfterScopeExit naming the escape site ---
  if $through_exit == 0 {
    error make {msg: $"through mode did not abort the CVE (exit ($through_exit)); log: ($tlines)"}
  }
  if ($tlines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: "through mode reached NO_ABORT: the CVE was not caught"}
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
  # F7: name the create_scalar_function registration site.
  let esc = ($viol | parse --regex 'escaped_at=(?<e>[0-9]+)' | get e.0?)
  if ($esc | is-empty) or (($esc | into int) == 0) {
    error make {msg: $"through mode did not name the registration site; got: ($viol)"}
  }

  # --- case-like mode: the safe deref is elided; the program runs clean ---
  let clines = (open $case_log | lines)
  if $case_exit != 0 {
    error make {msg: $"case-like mode aborted (exit ($case_exit)); it must elide the safe deref; log: ($clines)"}
  }
  if not ($clines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"case-like mode did not reach NO_ABORT; log: ($clines)"}
  }

  print $"rusqlite-0128 OK: through aborts UseAfterScopeExit on the real CVE \(escaped_at ($esc), the registration site\); case-like elides the safe deref"
}
