#!/usr/bin/env nu
# Differential gate (Task C3, I4). `through` is the oracle: it checks every
# dereference, so it catches every memory-safety violation in the corpus.
# `case` elides safe-pointer derefs (vetted at the cast) but re-checks the
# dealloc-reachable ones. So `case` may miss a violation `through` catches
# ONLY where the both-modes table documents an elision — the safe-pointer-deref
# row (a stack-scope use-after-scope read through a safe reference). Any other
# through-catch that case misses is a bug.
#
# This harness runs three contrasting reproducers in both modes and asserts:
#   - raw-pointer UAF (closure-escape): BOTH catch (raw deref, checked always)
#   - safe-ref stack UAF (through-safe-ref): through catches, case ELIDES
#     (the one documented elision)
#   - heap UAF (lru-0130): BOTH catch (case via the dealloc-reachable re-check)
# Exit code 0 = caught/aborted, non-zero-from-abort we pass in as booleans.

# aborted: did the run abort (non-zero exit)?
def main [
  ce_through: int, ce_case: int,      # closure-escape: raw UAF
  tsr_through: int, tsr_case: int,    # through-safe-ref: safe-ref stack UAF
  lru_through: int, lru_case: int,    # lru-0130: heap UAF
] {
  # Oracle: through must catch all three.
  for pair in [["closure-escape" $ce_through] ["through-safe-ref" $tsr_through] ["lru-0130" $lru_through]] {
    if ($pair.1 == 0) {
      error make {msg: $"oracle violated: through did not catch ($pair.0)"}
    }
  }

  # Agreement: case must catch the raw and heap UAFs.
  if ($ce_case == 0) {
    error make {msg: "case missed the raw-pointer UAF (closure-escape) — raw derefs are checked in both modes"}
  }
  if ($lru_case == 0) {
    error make {msg: "case missed the heap UAF (lru-0130) — the dealloc-reachable re-check should catch it"}
  }

  # Documented elision: case must ELIDE the safe-ref stack UAF.
  if ($tsr_case != 0) {
    error make {msg: "case unexpectedly caught the safe-ref stack UAF (through-safe-ref); the both-modes table says case elides safe-pointer derefs"}
  }

  print "differential OK: through caught all three; case agreed on the raw + heap UAFs and elided only the documented safe-pointer-deref (stack scope) — no undocumented gap"
}
