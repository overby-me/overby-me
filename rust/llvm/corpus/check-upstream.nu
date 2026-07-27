#!/usr/bin/env nu
# Runs an upstream LLVM test suite through our `opt` and reports how much of
# it we agree with.
#
#   nu check-upstream.nu <path-to-opt> <suite-directory> <ratchet>
#
# Each test says what it expects in its own RUN lines: a suite file whose RUN
# line invokes `not llvm-as` or `not opt` is a module upstream rejects, and
# everything else is one upstream accepts. We agree with a file when our
# accept-or-reject verdict matches, so both halves of the suite count. A file
# whose RUN lines need a tool or a flag we do not have is skipped and counted
# separately rather than being scored as agreement.
#
# The ratchet is the number of files we have to agree with. It only ever goes
# up: a change that drops it fails the check, and a change that raises it is
# expected to record the new number in the same commit.

# What a test's RUN lines say should happen.
def expectation [text: string]: nothing -> string {
  let runs = ($text | lines | where ($it | str contains "RUN:"))
  if ($runs | is-empty) {
    return "skip"
  }
  # lit skips a test whose REQUIRES line the build does not satisfy, and the
  # ones here want an assertions-enabled LLVM. Scoring them would measure the
  # oracle's build configuration rather than our agreement with it.
  if ($text | lines | any {|line| $line | str contains "REQUIRES:"}) {
    return "skip"
  }
  # Tools and modes this tier does not have. A test that needs one of them
  # says nothing about whether we read the module correctly.
  let unsupported = [
    "llvm-bcanalyzer" "llc " "llvm-link" "llvm-extract" "llvm-lto" "opt -passes"
    "--passes=" "-passes=" "split-file" "llvm-dwarfdump" "verify-uselistorder"
    "llvm-diff" "llvm-objdump" "%python" "sed " "grep " "count " "FileCheck --check-prefix"
  ]
  for pattern in $unsupported {
    if ($runs | any {|line| $line | str contains $pattern}) {
      return "skip"
    }
  }
  if ($runs | any {|line| ($line | str contains "not llvm-as") or ($line | str contains "not opt")}) {
    "reject"
  } else if ($runs | any {|line| ($line | str contains "llvm-as") or ($line | str contains "llvm-dis")}) {
    "accept"
  } else {
    "skip"
  }
}

def main [opt: path, suite: path, ratchet: int] {
  let files = (glob ([$suite "*.ll"] | path join) | sort)
  if ($files | is-empty) {
    error make {msg: $"no .ll files in ($suite)"}
  }

  mut agreed = 0
  mut skipped = 0
  mut disagreements = []
  for file in $files {
    let text = (open --raw $file)
    let want = (expectation $text)
    if $want == "skip" {
      $skipped = $skipped + 1
      continue
    }
    let result = (do { ^$opt -S -passes=verify $file -o /dev/null } | complete)
    let accepted = $result.exit_code == 0
    let matched = if $want == "accept" { $accepted } else { not $accepted }
    if $matched {
      $agreed = $agreed + 1
    } else {
      let reason = if $accepted {
        "we accept a module upstream rejects"
      } else {
        ($result.stderr | lines | first 1 | str join "" | str trim)
      }
      $disagreements = ($disagreements | append $"($file | path basename): ($reason)")
    }
  }

  let considered = $agreed + ($disagreements | length)
  print $"suite:    ($suite | path basename)"
  print $"agreed:   ($agreed) of ($considered) considered; ($skipped) skipped; ($files | length) total"
  print $"ratchet:  ($ratchet)"
  if ($disagreements | is-not-empty) {
    print ""
    print "disagreements:"
    print ($disagreements | first 40 | str join "\n")
    if (($disagreements | length) > 40) {
      print $"... and (($disagreements | length) - 40) more"
    }
  }

  if $agreed < $ratchet {
    error make {msg: $"agreement fell from ($ratchet) to ($agreed); the ratchet only moves up"}
  }
  if $agreed > $ratchet {
    print ""
    print $"agreement rose to ($agreed); raise the ratchet in default.nix to hold it"
  }
}
