#!/usr/bin/env nu

let app = $env.WORK | path join instinfo
^$env.FLATPAK build-init $app org.test.InstInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info org.test.InstInfo } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output | str contains "user") {
  print "FAIL: info output does not contain 'user' installation"
  print $output
  exit 1
}

print "PASS: info-installation"
