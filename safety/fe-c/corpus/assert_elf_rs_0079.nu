#!/usr/bin/env nu
# Asserts the real RUSTSEC-2022-0079 catch in BOTH modes. elf_rs 0.2.0 builds
# `from_raw_parts(content.as_ptr().add(sh_off), sh_num)` for the section header
# table with sh_off/sh_num read straight from the (attacker-controlled) ELF
# header. A crafted header with a huge section count yields a slice far past the
# input buffer. The slice-constructor extent check vets the claimed extent at
# the from_raw_parts mint (resolving the derivation root, I10), so both `through`
# and `case` abort OutOfBounds naming the owning buffer — not the off-the-end
# element address. The element type is generic (`ET::SectionHeader`), so this
# also exercises the injected `size_of::<T>()` that monomorphizes.

def check-mode [label: string, log: path, exit_code: int] {
  let lines = (open $log | lines)
  if $exit_code == 0 {
    error make {msg: $"($label) mode did not abort the elf_rs OOB \(exit ($exit_code)\); log: ($lines)"}
  }
  if ($lines | any {|l| $l | str contains "NO_ABORT"}) {
    error make {msg: $"($label) mode reached NO_ABORT: the unvalidated section count was not caught"}
  }
  let viol = ($lines | where {|l| $l | str starts-with "fe-c-violation"} | first)
  if ($viol | is-empty) {
    error make {msg: $"($label) mode: no fe-c-violation line; log: ($lines)"}
  }
  if not ($viol | str contains "kind=OutOfBounds") {
    error make {msg: $"($label) mode: expected OutOfBounds, got: ($viol)"}
  }
  # The report resolves the owning ELF buffer (derivation root), not the
  # off-the-end section-table address.
  let base = ($viol | parse --regex 'alloc_base=(?<b>0x[0-9a-f]+)' | get b.0?)
  if ($base | is-empty) {
    error make {msg: $"($label) mode: report did not name the owning allocation; got: ($viol)"}
  }
  $base
}

# Asserts both modes aborted OutOfBounds naming the owning ELF buffer.
def main [
  through_log: path, through_exit: int,
  case_log: path, case_exit: int,
] {
  let tbase = (check-mode "through" $through_log $through_exit)
  let cbase = (check-mode "case" $case_log $case_exit)
  print $"elf-rs-0079 OK: both modes aborted OutOfBounds naming the owning buffer \(through=($tbase) case=($cbase)\)"
}
