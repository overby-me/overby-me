#!/usr/bin/env nu

let r = (do { ^$env.FLATPAK --user create-usb } | complete)
if $r.exit_code == 0 {
  print "FAIL: expected non-zero exit"
  exit 1
}

print "PASS: create-usb-missing-args"
