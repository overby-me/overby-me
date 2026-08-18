#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user override --socket wayland } | complete)
if $r.exit_code == 0 { print "FAIL: expected non-zero exit"; exit 1 }
let output = $r.stdout + $r.stderr
if not (($output | str downcase) =~ 'no application|specified') { print "FAIL: no hint"; exit 1 }

print "PASS: override-missing-app"
