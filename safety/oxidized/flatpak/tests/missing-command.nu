#!/usr/bin/env nu

# Test: running flatpak with no args exits non-zero and shows usage
let r = (do { ^$env.FLATPAK } | complete)
let output = $r.stdout + $r.stderr
let rc = $r.exit_code

if $rc == 0 {
  print "FAIL: expected non-zero exit code, got 0"
  exit 1
}

if not (($output | str downcase) | str contains "usage") {
  print "FAIL: output does not contain 'Usage'"
  print $"Got: ($output)"
  exit 1
}

print $"PASS: missing command prints usage and exits non-zero \(rc=($rc)\)"
