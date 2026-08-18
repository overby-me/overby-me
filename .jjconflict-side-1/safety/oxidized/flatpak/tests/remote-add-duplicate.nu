#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user remote-add dup-remote https://example.com/repo

let r = (do { ^$env.FLATPAK --user remote-add dup-remote https://example.com/other } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"

if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit when adding duplicate remote"
  exit 1
}

if not ($output =~ '(?i)already exists') {
  print $"FAIL: output does not mention 'already exists': ($output)"
  exit 1
}

print "PASS: remote-add-duplicate"
