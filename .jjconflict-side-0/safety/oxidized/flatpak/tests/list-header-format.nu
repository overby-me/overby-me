#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user list } | complete)
let output = $r.stdout + $r.stderr

let first_line = ($output | lines | get 0? | default "")
if not ($first_line | str contains "Name") {
  print "FAIL: list header does not contain 'Name'"
  print $output
  exit 1
}

print "PASS: list-header-format"
