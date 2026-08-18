#!/usr/bin/env nu
# Asserts the B1 capability-propagation dataflow (I10) traced each
# `insert_many_like` write back to its `as_mut_ptr` derivation root, in both
# the direct-deref and ptr::write forms. Reads the driver's stdout, which
# carries `fe-c-prov fn=… write_roots=[…]` lines under FEC_PROV_FN.

def main [log: path] {
  let lines = (open $log | lines | where {|l| $l | str starts-with "fe-c-prov"})

  for fn in ["insert_many_like_direct" "insert_many_like_write"] {
    let hit = (
      $lines
      | where {|l| ($l | str contains $fn) and ($l | str contains 'write_roots=["as_mut_ptr"]')}
    )
    if ($hit | is-empty) {
      error make {msg: $"provenance: ($fn) write not rooted at as_mut_ptr; got: ($lines)"}
    }
  }

  print "provenance OK: both write forms rooted at as_mut_ptr (I10)"
}
