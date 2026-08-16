#!/usr/bin/env nu

let app = $env.WORK | path join locinfo
^$env.FLATPAK build-init $app org.test.LocInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info org.test.LocInfo } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output =~ '(?i)Location') {
  print "FAIL: info output does not contain 'Location'"
  print $output
  exit 1
}

print "PASS: info-location"
