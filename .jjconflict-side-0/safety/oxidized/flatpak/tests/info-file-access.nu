#!/usr/bin/env nu

let app = $env.WORK | path join fa-app
^$env.FLATPAK build-init $app org.test.FAInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app --filesystem home
^$env.FLATPAK --user install $app

# home should be read-write (granted via --filesystem home)
let r = (do { ^$env.FLATPAK --user info --file-access=home org.test.FAInfo } | complete)
let output = ($r.stdout + $r.stderr | str trim -r -c "\n")
if $output != "read-write" {
  print $"FAIL: expected 'read-write' for home, got '($output)'"
  exit 1
}

# /usr should be hidden (not granted)
let r = (do { ^$env.FLATPAK --user info --file-access=/usr org.test.FAInfo } | complete)
let output = ($r.stdout + $r.stderr | str trim -r -c "\n")
if $output != "hidden" {
  print $"FAIL: expected 'hidden' for /usr, got '($output)'"
  exit 1
}

print "PASS: info-file-access"
