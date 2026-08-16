#!/usr/bin/env nu

# Verify `flatpak update --bundle=PATH` rejects nonexistent bundles.
let r = (do { ^$env.FLATPAK --user update --bundle=/nonexistent.flatpak } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"
if not ($output =~ "bundle not found") {
  print "FAIL: expected 'bundle not found' in stderr"
  print $"Got: ($output)"
  exit 1
}
print "PASS: update-bundle-missing"
