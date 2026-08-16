#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user remote-info } | complete)
if $r.exit_code == 0 { print "FAIL: expected non-zero exit"; exit 1 }

print "PASS: remote-info-missing-args"
