#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK search } | complete)
if $r.exit_code == 0 { print "FAIL: expected non-zero exit"; exit 1 }

print "PASS: search-missing-args"
