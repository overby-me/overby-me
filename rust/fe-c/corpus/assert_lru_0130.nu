#!/usr/bin/env nu
# Asserts RUSTSEC-2021-0130 against real lru 0.6.6. The iterator yields a
# reference into a node; the loop pop()s (frees) that node, then reads the
# value through the dangling reference. Under FEC_MODE=through the read is a
# checked dereference that resolves the freed heap allocation (kept findable in
# quarantine) and aborts UseAfterFree. Fe-C catching a real use-after-free CVE
# in a real, unmodified crate.

def main [log: path, exit_code: int] {
  let lines = (open $log | lines)

  if $exit_code == 0 {
    error make {msg: $"through mode did not abort the UAF (exit ($exit_code)); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: "reached NO_ABORT: the freed-node read was not caught"}
  }

  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=UseAfterFree") {
    error make {msg: $"expected UseAfterFree, got: ($viol)"}
  }

  # The report must name the freed heap allocation (base + id).
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"report did not name the freed allocation; got: ($viol)"}
  }
  let id = ($viol | parse --regex 'alloc_id=(?<i>[0-9]+)' | get i.0?)
  if ($id | is-empty) {
    error make {msg: $"report did not name the freed allocation id; got: ($viol)"}
  }

  # The faulting pointer is the reference that was read (into the freed node).
  let vp = (
    $lines | where {|l| $l | str starts-with "VALUE_PTR="} | first
    | default "VALUE_PTR=" | str replace "VALUE_PTR=" "" | str trim
  )
  let fault = ($viol | parse --regex 'fault=(?<f>0x[0-9a-f]+)' | get f.0?)
  if (not ($vp | is-empty)) and ($fault != $vp) {
    error make {msg: $"fault ($fault) is not the read reference ($vp): ($viol)"}
  }

  print $"lru-0130 OK: through aborts UseAfterFree on the freed node \(base ($base), id ($id)\)"
}
