#!/usr/bin/env nu
# Asserts the B4 false-positive result: the hashbrown workload, with its
# SwissTable unsafe instrumented and FecAlloc tracking allocations, ran to
# completion with no fe-c violation — while the injected checks did fire (a
# check that never runs proves nothing).

def main [log: path, exit_code: int] {
  let lines = (open $log | lines)

  # Must not have trapped.
  if $exit_code != 0 {
    error make {msg: $"workload exited ($exit_code); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str starts-with "fe-c-violation"}) {
    error make {msg: $"false positive: instrumentation trapped legitimate code; log: ($lines)"}
  }
  if not ($lines | any {|l| $l | str contains "FP_OK"}) {
    error make {msg: $"workload did not complete; log: ($lines)"}
  }

  # The checks must actually have executed (proving instrumentation was live).
  let report = ($lines | where {|l| $l | str contains "deref checks executed"} | first)
  if ($report | is-empty) {
    error make {msg: "no checks executed — instrumentation was not active"}
  }
  let n = ($report | parse --regex 'fe-c: (?<n>\d+) deref checks' | get n.0 | into int)
  if $n < 10000 {
    error make {msg: $"only ($n) checks executed; expected the workload to fire many"}
  }

  print $"false-positive OK: ($n) checks fired on instrumented hashbrown, no false trap"
}
