#!/usr/bin/env nu

# Test config --unset removes a key
^$env.FLATPAK --user config --set languages "en;de"
let r = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output = ($r.stdout + $r.stderr | str trim -r -c "\n")
if $output != "en;de" {
  print $"FAIL: expected 'en;de', got '($output)'"
  exit 1
}

^$env.FLATPAK --user config --unset languages
let rc = (do { ^$env.FLATPAK --user config --get languages } | complete).exit_code
if $rc == 0 {
  print "FAIL: config --get should fail after --unset"
  exit 1
}

# Unset of non-existent key should not fail
^$env.FLATPAK --user config --unset nonexistent-key

print "PASS: config-unset"
