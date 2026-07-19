#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user pin } | complete)
if $r.exit_code == 0 { print "FAIL: expected non-zero exit"; exit 1 }

print "PASS: pin-missing-args"
