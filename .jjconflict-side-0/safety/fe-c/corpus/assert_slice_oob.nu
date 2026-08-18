#!/usr/bin/env nu
# Asserts the slice-constructor extent check (point 1, the case-mode
# slice-reborrow gap) in BOTH modes. A `&[u64]` is built with
# `slice::from_raw_parts(buf.as_ptr(), 1000)` over a four-element buffer, then
# indexed at 500 — in bounds of the slice's *claimed* length but far past the
# real allocation. Vetting the slice's extent at the `from_raw_parts` mint (the
# raw->safe cast for a slice) catches the length lie at construction, so both
# `case` (which elides the slice's later derefs) and `through` abort OutOfBounds
# naming the owning buffer — never the off-the-end faulting address (I10).

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the slice OOB \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the lying slice was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"($label) mode: expected OutOfBounds, got: ($viol)"}
  }
  # The report resolves the owning Vec buffer (derivation root, I10), not the
  # off-the-end element address the faulting index lands in.
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

# One mode's run: asserts it aborted OutOfBounds naming the owning buffer.
# Called once per mode (through, case) from the check.
def main [
  through_log: path, through_exit: int,
  case_log: path, case_exit: int,
] {
  let tbase = (check-mode "through" $through_log $through_exit)
  let cbase = (check-mode "case" $case_log $case_exit)
  print $"slice-oob OK: both modes aborted OutOfBounds naming the owning buffer \(through=($tbase) case=($cbase)\)"
}
