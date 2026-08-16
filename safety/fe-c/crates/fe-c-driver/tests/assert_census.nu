#!/usr/bin/env nu
# Asserts a fe-c-driver census JSON meets the hand-audited minimums for
# tests/fixtures/census_fixture.rs (Task A5 spot-check). Counts are lower
# bounds: MIR lowering only ever adds temporaries, so the census may see
# more than the source shows, never fewer — under-counting would break I1.

def main [census: path] {
  let c = (open $census)

  let mins = {
    bodies: 6           # five fns + main
    raw_ptr_locals: 5   # one per raw-pointer parameter, at least
    raw_derefs: 3       # raw_read, raw_write, reborrow's &*p
    raw_to_safe_casts: 1 # the &*p reborrow
    ptr_int_casts: 1    # `p as usize`
    ffi_calls: 1        # the extern "C" abs() call
  }

  for field in ($mins | columns) {
    let got = ($c | get $field)
    let want = ($mins | get $field)
    if $got < $want {
      error make {msg: $"census ($field) = ($got), expected >= ($want)"}
    }
  }

  # Completeness (I1): every body must be visited, none skipped.
  if ($c | get skipped_bodies) != 0 {
    error make {msg: $"census skipped ($c.skipped_bodies) bodies; must be 0 for I1"}
  }

  print $"census OK for crate ($c.crate): ($c | reject crate | to json --raw)"
}
