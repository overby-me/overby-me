#!/usr/bin/env nu

^$env.FLATPAK build-init ($env.WORK | path join listapp) org.test.Listed org.test.Sdk org.test.Platform
mkdir ($env.WORK | path join listapp files bin)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join listapp files bin app)
chmod +x ($env.WORK | path join listapp files bin app)
^$env.FLATPAK build-finish ($env.WORK | path join listapp) --command app
^$env.FLATPAK --user install ($env.WORK | path join listapp)

let r = (do { ^$env.FLATPAK --user list } | complete)
let output = $r.stdout + $r.stderr

if not ($output | str contains "org.test.Listed") {
  print "FAIL: list output does not contain 'org.test.Listed'"
  print $output
  exit 1
}

print "PASS: list-installed-app"
