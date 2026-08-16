#!/usr/bin/env nu

# Test: flatpak --user config prints something useful

mkdir ($env.HOME | path join .local share flatpak)

let r = (do { ^$env.FLATPAK --user config } | complete)
let output = ($r.stdout + $r.stderr)

if ($output | is-empty) {
  print "FAIL: flatpak --user config produced no output"
  exit 1
}

print $"Output: ($output)"
print "PASS: flatpak --user config produced output"
