#!/usr/bin/env nu

let app = $env.WORK | path join rtinfo
^$env.FLATPAK build-init $app org.test.RTInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info org.test.RTInfo } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output =~ '(?i)Runtime') {
  print "FAIL: info output does not contain 'Runtime'"
  print $output
  exit 1
}

if not ($output | str contains "org.test.Platform") {
  print "FAIL: info output does not contain 'org.test.Platform'"
  print $output
  exit 1
}

print "PASS: info-runtime"
