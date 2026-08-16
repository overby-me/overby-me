#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user list } | complete)
let output = $r.stdout + $r.stderr

# Check it doesn't crash - if we got here, it didn't
# Check output contains header
if not (($output | str downcase) | str contains "name") {
  print $"FAIL: list output does not contain 'Name' header: ($output)"
  exit 1
}

print "PASS: list-empty"
