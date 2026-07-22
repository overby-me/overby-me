#!/usr/bin/env nu
# Asserts RUSTSEC-2021-0130 against real lru 0.6.6, in BOTH modes. The iterator
# yields a reference into a node; the loop pop()s (frees) that node, then reads
# the value through the dangling reference. `through` checks every dereference;
# `case` elides safe derefs but re-checks the dealloc-reachable one (it follows
# the pop() call) — the point-4 / I6 re-check. So both modes resolve the freed
# heap allocation (kept findable in quarantine) and abort UseAfterFree. Fe-C
# catching a real use-after-free CVE in a real, unmodified crate, both modes.

# `expect_read`: the case-mode dealloc-reachable re-check is injected right at
# the dangling dereference, so its report names the read site (`read_at=`).
# `through` catches it at the same deref via the generic rooted check, which
# carries no read line, so it does not.
def check-mode [label: string, log: path, exit_code: int, expect_read: bool] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the UAF (exit ($exit_code)); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the freed-node read was not caught"}
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
  if $expect_read {
    let rl = ($viol | parse --regex 'read_at=(?<n>[0-9]+)' | get n.0?)
    if ($rl | is-empty) {
      error make {msg: $"($label) mode: report did not name the dangling-read site \(read_at=\); got: ($viol)"}
    }
    if (($rl | into int) <= 0) {
      error make {msg: $"($label) mode: read_at is not a valid source line: ($rl)"}
    }
  }
  $base
}

def main [through_log: path, through_exit: int, case_log: path, case_exit: int] {
  let tb = (check-mode "through" $through_log $through_exit false)
  let cb = (check-mode "case" $case_log $case_exit true)
  print $"lru-0130 OK: both modes abort UseAfterFree on the freed node \(through base ($tb), case base ($cb)\); case caught it via the dealloc-reachable re-check and named the dangling-read site"
}
