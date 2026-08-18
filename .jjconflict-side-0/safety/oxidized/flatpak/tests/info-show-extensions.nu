#!/usr/bin/env nu

let app = $env.WORK | path join ext-app
^$env.FLATPAK build-init $app org.test.ExtInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info --show-extensions org.test.ExtInfo } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output =~ '(?i)No extensions') {
  print "FAIL: expected 'No extensions' for app without extension points"
  print $"Got: ($output)"
  exit 1
}

print "PASS: info-show-extensions"
