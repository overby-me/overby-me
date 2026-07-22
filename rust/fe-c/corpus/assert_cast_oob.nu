#!/usr/bin/env nu
# Asserts the raw->safe cast ensure (point 1, §3.1) in BOTH modes. A raw
# pointer past the end of a Vec buffer is cast to `&u64`; the cast ensure
# resolves the derivation root, sees the referent's extent is out of bounds,
# and aborts OutOfBounds. Both `case` (which elides the later derefs) and
# `through` (whose deref check resolves the off-the-end faulting address and so
# relies on the cast ensure) must abort.

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the OOB cast (exit ($exit_code)); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the OOB raw->safe cast was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"($label) mode: expected OutOfBounds, got: ($viol)"}
  }
  # The report resolves the owning Vec buffer, not the off-the-end address.
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

def main [
  tw_log: path, tw_exit: int, # through, whole-object &*bad
  cw_log: path, cw_exit: int, # case,    whole-object &*bad
  tf_log: path, tf_exit: int, # through, field reborrow &(*p).b
  cf_log: path, cf_exit: int, # case,    field reborrow &(*p).b
] {
  let tw = (check-mode "through/whole-object" $tw_log $tw_exit)
  let cw = (check-mode "case/whole-object" $cw_log $cw_exit)
  let tf = (check-mode "through/field" $tf_log $tf_exit)
  let cf = (check-mode "case/field" $cf_log $cf_exit)
  print $"cast-oob OK: both modes abort OutOfBounds on the raw->safe cast — whole-object \(through ($tw), case ($cw)\) and field reborrow \(through ($tf), case ($cf)\) — resolved at the owning buffer via the cast ensure"
}
