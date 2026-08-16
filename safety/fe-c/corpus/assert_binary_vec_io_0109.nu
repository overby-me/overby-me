#!/usr/bin/env nu
# Asserts the real RUSTSEC-2025-0109 catch in BOTH modes. binary_vec_io 0.1.12's
# safe `binary_write_from_ref<T>(f, p: &T, n)` builds
# `from_raw_parts(p as *const u8, n * size_of::<T>())` from a single referent, so
# n > 1 yields a slice past the one-element allocation. The slice-constructor
# extent check vets the claimed extent at the from_raw_parts mint (resolving the
# derivation root, I10) — before write_all reads the slice — so both `through`
# and `case` abort OutOfBounds naming the owning heap allocation.

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the binary_vec_io OOB \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the oversized slice was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"($label) mode: expected OutOfBounds, got: ($viol)"}
  }
  # The report resolves the owning one-element allocation (derivation root), not
  # the off-the-end slice address.
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

# Asserts both modes aborted OutOfBounds naming the owning heap allocation.
def main [
  through_log: path, through_exit: int,
  case_log: path, case_exit: int,
] {
  let tbase = (check-mode "through" $through_log $through_exit)
  let cbase = (check-mode "case" $case_log $case_exit)
  print $"binary-vec-io-0109 OK: both modes aborted OutOfBounds naming the owning allocation \(through=($tbase) case=($cbase)\)"
}
