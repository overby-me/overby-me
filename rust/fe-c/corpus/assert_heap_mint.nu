#!/usr/bin/env nu
# Asserts heap use-after-free with mint-site naming (trace -0130 debuggability).
# A Box is freed while a field reference minted into it is held; reading through
# the dangling reference aborts UseAfterFree in both modes. Because the mint is
# in the instrumented binary, both reports name `minted_at` (where the reference
# was born, recorded by the point-1 ensure); `case` additionally names `read_at`
# (its dealloc-reachable re-check sits right at the dangling read).

def check-mode [label: string, log: path, exit_code: int, expect_read: bool] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the UAF (exit ($exit_code)); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the freed-Box read was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=UseAfterFree") {
    error make {msg: $"($label) mode: expected UseAfterFree, got: ($viol)"}
  }
  let mint = ($viol | parse --regex 'minted_at=(?<n>[0-9]+)' | get n.0?)
  if ($mint | is-empty) {
    error make {msg: $"($label) mode: report did not name the mint site \(minted_at=\); got: ($viol)"}
  }
  if (($mint | into int) <= 0) {
    error make {msg: $"($label) mode: minted_at is not a valid source line: ($mint)"}
  }
  if $expect_read {
    let rl = ($viol | parse --regex 'read_at=(?<n>[0-9]+)' | get n.0?)
    if ($rl | is-empty) {
      error make {msg: $"($label) mode: expected read_at in case mode; got: ($viol)"}
    }
  }
  $mint
}

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tm = (check-mode "through" $through_log $through_exit false)
  let cm = (check-mode "case" $case_log $case_exit true)
  print $"heap-mint OK: both modes abort UseAfterFree naming the mint site \(through minted_at=($tm), case minted_at=($cm)\); case also named the dangling-read site"
}
