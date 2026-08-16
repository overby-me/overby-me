#!/usr/bin/env nu

^$env.FLATPAK build-init ($env.WORK | path join emptygrp) org.test.EmptyGrp org.test.Sdk org.test.Platform

# Add an empty Context group
"\n[Context]\n\n" | save -a --raw ($env.WORK | path join emptygrp metadata)

# build-finish should still work with the empty group present
^$env.FLATPAK build-finish ($env.WORK | path join emptygrp) --command test --socket x11

if (do { ^grep -q "x11" ($env.WORK | path join emptygrp metadata) } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain 'x11' after build-finish with empty group"
  ^cat ($env.WORK | path join emptygrp metadata)
  exit 1
}

print "PASS: metadata-empty-groups"
