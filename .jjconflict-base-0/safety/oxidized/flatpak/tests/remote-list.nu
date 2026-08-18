#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user remote-add myremote https://example.com/repo

let r = (do { ^$env.FLATPAK --user remotes } | complete)
let output = ($r.stdout + $r.stderr) | str trim -r -c "\n"

if not ($output =~ "myremote") {
  print "FAIL: remotes output does not contain 'myremote'"
  print $"Output was: ($output)"
  exit 1
}

print "PASS: remote-list"
