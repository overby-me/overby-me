#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user uninstall org.test.DoesNotExist } | complete)
if $r.exit_code == 0 { print "FAIL: expected non-zero exit"; exit 1 }

print "PASS: uninstall-nonexistent"
