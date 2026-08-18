#!/usr/bin/env nu

let app = $env.WORK | path join metainfo
^$env.FLATPAK build-init $app org.test.MetaInfo org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info --show-metadata org.test.MetaInfo } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output | str contains "[Application]") {
  print "FAIL: output does not contain [Application]"
  print $output
  exit 1
}

if not ($output | str contains "name=org.test.MetaInfo") {
  print "FAIL: output does not contain name=org.test.MetaInfo"
  print $output
  exit 1
}

print "PASS: info-show-metadata"
