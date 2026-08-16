#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user run org.test.NonExistent } | complete)

if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit for missing app"
  print (($r.stdout + $r.stderr) | str trim -r -c "\n")
  exit 1
}

print "PASS: run-missing-app"
