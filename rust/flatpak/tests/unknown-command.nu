#!/usr/bin/env nu

# Test: unknown command produces error with "unknown command"

let r = (do { ^$env.FLATPAK badcmd123 } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"

if ($output | is-empty) {
  print "FAIL: no output from unknown command"
  exit 1
}

if $output =~ '(?i)unknown command' {
  print "PASS: unknown command error message found"
} else {
  print "FAIL: expected 'unknown command' in output, got:"
  print $output
  exit 1
}

# Also verify it exits non-zero
if (do { ^$env.FLATPAK badcmd123 } | complete).exit_code == 0 {
  print "FAIL: expected non-zero exit code for unknown command"
  exit 1
}

print "PASS: unknown-command"
