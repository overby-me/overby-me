#!/usr/bin/env nu
# Runs an upstream LLVM test suite through our `opt` and through real
# `llvm-as`, and reports how often the two agree about whether a file is a
# module at all.
#
#   nu check-upstream.nu <path-to-opt> <path-to-llvm-as> <suite-directory> <ratchet>
#
# The oracle is llvm-as's own verdict, not the test's RUN lines. Reading the
# RUN lines was the first attempt and it was wrong often enough to matter: a
# test that pipes llvm-as's stderr into FileCheck is expecting a diagnostic,
# and there are 286 of those in `Verifier` alone against the 74 that spell it
# `not llvm-as`. Scoring those as "upstream accepts this" made our own wrong
# acceptances look like agreement. Asking the tool is both simpler and true.
#
# Nothing is skipped. Every `.ll` file in the suite is a module llvm-as either
# reads or refuses, so every file has an answer to agree or disagree with.
#
# There are two bounds, because agreement alone can be gamed. Refusing a
# module for a rule that does not exist scores as agreement whenever upstream
# refuses it for a different reason, so deleting that wrong rule *lowers* the
# agreement count while making the parser more correct. The second bound is
# the number of modules we refuse that llvm-as reads, which is the failure
# that actually matters, and it may only fall. Together they say: agree more,
# and never buy agreement by refusing valid input.

def main [opt: path, llvm_as: path, suite: path, ratchet: int, refusals: int] {
  let files = (glob ([$suite "*.ll"] | path join) | sort)
  if ($files | is-empty) {
    error make {msg: $"no .ll files in ($suite)"}
  }

  mut agreed = 0
  mut we_accept = []
  mut we_reject = []
  for file in $files {
    let upstream = (do { ^$llvm_as $file -o /dev/null } | complete)
    let ours = (do { ^$opt -S -passes=verify $file -o /dev/null } | complete)
    let accepted = $ours.exit_code == 0
    if ($upstream.exit_code == 0) == $accepted {
      $agreed = $agreed + 1
    } else if $accepted {
      $we_accept = ($we_accept | append ($file | path basename))
    } else {
      let reason = ($ours.stderr | lines | last 1 | str join "" | str trim)
      $we_reject = ($we_reject | append $"($file | path basename): ($reason)")
    }
  }

  print $"suite:    ($suite | path basename)"
  print $"agreed:   ($agreed) of ($files | length)"
  print $"ratchet:  ($ratchet) agreed, at most ($refusals) refused"
  if ($we_reject | is-not-empty) {
    print ""
    print $"we refuse ($we_reject | length) modules llvm-as reads:"
    print ($we_reject | first 40 | str join "\n")
    if (($we_reject | length) > 40) {
      print $"... and (($we_reject | length) - 40) more"
    }
  }
  if ($we_accept | is-not-empty) {
    print ""
    print $"we read ($we_accept | length) modules llvm-as refuses:"
    print ($we_accept | first 40 | str join " ")
    if (($we_accept | length) > 40) {
      print $"... and (($we_accept | length) - 40) more"
    }
  }

  if ($we_reject | length) > $refusals {
    error make {msg: $"we now refuse ($we_reject | length) modules llvm-as reads, up from ($refusals); that ceiling only moves down"}
  }
  if $agreed < $ratchet {
    error make {msg: $"agreement fell from ($ratchet) to ($agreed); the ratchet only moves up"}
  }
  if $agreed > $ratchet or ($we_reject | length) < $refusals {
    print ""
    print $"now ($agreed) agreed and ($we_reject | length) refused; record both in default.nix"
  }
}
