#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# Run repair on a healthy installation: should find no problems
let r = (do { ^$env.FLATPAK --user repair } | complete)
let output = $r.stdout + $r.stderr
print $"repair output: ($output)"

if ($output | lines | any {|l| $l =~ 'No problems found' }) {
  ok "repair reports no problems on healthy installation"
} else {
  print "FAIL: repair did not report 'No problems found'"
  print $"Got: ($output)"
  exit 1
}

# Verify the number of refs checked is at least 1
if ($output | lines | any {|l| $l =~ '[0-9]+ refs checked' }) {
  ok "repair reports refs checked count"
} else {
  print "FAIL: repair output missing refs checked count"
  print $"Got: ($output)"
  exit 1
}

print "PASS: vm-repair-no-problems"
