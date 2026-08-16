#!/usr/bin/env nu

let app = $env.WORK | path join datadir
^$env.FLATPAK build-init $app org.test.DataDirs org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

# The installation should have the app directory structure
let inst = $env.HOME | path join .local share flatpak
let app_dir = (do { ^find ($inst | path join app) -maxdepth 1 -name "org.test.DataDirs" -type d } | complete | get stdout | str trim)
if ($app_dir | is-empty) {
  print "FAIL: app directory not created"
  exit 1
}

print "PASS: install-creates-data-dirs"
