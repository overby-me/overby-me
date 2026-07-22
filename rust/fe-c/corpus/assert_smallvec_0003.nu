#!/usr/bin/env nu
# Asserts the corpus-smallvec-0003 acceptance (Task B3): the instrumented
# RUSTSEC-2021-0003 reproducer aborts naming the SmallVec allocation (the
# derivation root of the overflowing write) and NOT the neighbouring String
# — the I10 / F10 provenance canary — while the patched 1.6.1 control runs
# clean.

# Parses "0x..." hex from a "KEY=0x..." line.
def hex-of [lines: list, key: string] {
  let line = ($lines | where {|l| $l | str starts-with $"($key)="} | first)
  $line | str replace $"($key)=" "" | str trim
}

def main [repro_log: path, repro_exit: int, control_log: path, control_exit: int] {
  let r = (open $repro_log | lines)

  # 1. The reproducer must have aborted (SIGABRT = 134), not run to the end.
  if $repro_exit == 0 {
    error make {msg: $"reproducer did not abort \(exit ($repro_exit)\); log: ($r)"}
  }
  if not ($r | any {|l| $l | str contains "NO_ABORT"} | into bool | ($in == false)) {
    error make {msg: "reproducer reached NO_ABORT: the overflow was not trapped"}
  }

  # 2. It must have emitted a fe-c out-of-bounds violation.
  let viol = ($r | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"no fe-c-violation line; log: ($r)"}
  }

  # 3. The named allocation must be the SmallVec buffer, NOT the String.
  let sv = (hex-of $r "SMALLVEC_BASE")
  let st = (hex-of $r "STRING_BASE")
  let alloc_base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0)
  if $alloc_base != $sv {
    error make {msg: $"report named ($alloc_base), expected the SmallVec allocation ($sv)"}
  }
  if $alloc_base == $st {
    error make {msg: $"I10/F10 regression: report named the neighbouring String ($st)"}
  }

  # 4. The patched control must run clean.
  let c = (open $control_log | lines)
  if $control_exit != 0 {
    error make {msg: $"patched control aborted \(exit ($control_exit)\); log: ($c)"}
  }
  if not ($c | any {|l| $l | str contains "CONTROL_OK"}) {
    error make {msg: $"control did not complete cleanly; log: ($c)"}
  }

  print $"corpus-smallvec-0003 OK: aborted naming the SmallVec allocation ($sv), not the String ($st); control clean"
}
