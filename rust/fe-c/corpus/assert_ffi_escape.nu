#!/usr/bin/env nu
# Asserts the B5 / I9 cross-FFI escape result (trace F6): the reproducer hands
# a stack borrow out to C, the frame returns, C re-enters Rust through the
# trampoline, and the dereference of the now-dead stack local aborts with a
# UseAfterScopeExit naming the escaped local's own address (not whatever now
# occupies the reused stack).

def main [log: path, exit_code: int] {
  let lines = (open $log | lines)

  if $exit_code == 0 {
    error make {msg: $"reproducer did not abort (exit ($exit_code)); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: "reached NO_ABORT: the cross-FFI stale access was not trapped"}
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

  print $"ffi-escape OK: aborted UseAfterScopeExit naming the dead stack scope ($sl) reached across FFI"
}
