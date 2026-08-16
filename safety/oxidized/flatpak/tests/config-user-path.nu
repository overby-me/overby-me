#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user config } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output | str contains $env.HOME) {
  print $"FAIL: config output does not contain HOME path '($env.HOME)'"
  print $output
  exit 1
}

print "PASS: config-user-path"
