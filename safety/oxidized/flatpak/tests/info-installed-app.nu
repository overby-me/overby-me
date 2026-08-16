#!/usr/bin/env nu

let app = $env.WORK | path join infoapp
^$env.FLATPAK build-init $app org.test.Info org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info org.test.Info } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output | str contains "org.test.Info") {
  print "FAIL: info output does not contain 'org.test.Info'"
  print $output
  exit 1
}

print "PASS: info-installed-app"
