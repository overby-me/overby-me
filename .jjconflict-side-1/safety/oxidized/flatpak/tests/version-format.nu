#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --version } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"

if not ($output =~ 'Flatpak \d+\.\d+\.\d+') {
  print $"FAIL: version output does not match 'Flatpak X.Y.Z' pattern: ($output)"
  exit 1
}

print "PASS: version-format"
