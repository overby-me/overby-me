#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --help } | complete)
let output = ($r.stdout + $r.stderr)

for cmd in [install update uninstall list info run override search remotes] {
  if not ($output | str downcase | str contains $cmd) {
    print $"FAIL: --help output does not mention '($cmd)'"
    print $output
    exit 1
  }
}

print "PASS: help-usage-format"
