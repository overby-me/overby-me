#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --version } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
if not ($output =~ '(?i)flatpak') {
  print $"FAIL: version output does not contain 'flatpak': ($output)"
  exit 1
}
print "PASS: version output contains 'flatpak'"
