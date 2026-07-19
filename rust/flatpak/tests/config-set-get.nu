#!/usr/bin/env nu

# Test config --set / --get round-trip
^$env.FLATPAK --user config --set languages "en;de"
let r = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output = ($r.stdout + $r.stderr | str trim -r -c "\n")
if $output != "en;de" {
  print $"FAIL: expected 'en;de', got '($output)'"
  exit 1
}

# Overwrite
^$env.FLATPAK --user config --set languages "en;fr"
let r = (do { ^$env.FLATPAK --user config --get languages } | complete)
let output = ($r.stdout + $r.stderr | str trim -r -c "\n")
if $output != "en;fr" {
  print $"FAIL: expected 'en;fr', got '($output)'"
  exit 1
}

print "PASS: config-set-get"
