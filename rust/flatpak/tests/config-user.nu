#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user config } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output =~ '(?i)user|path') {
  print "FAIL: config output does not contain 'user' or 'path'"
  print $output
  exit 1
}

print "PASS: config-user"
