#!/usr/bin/env nu

let app = $env.WORK | path join urepo-app
let repo = $env.WORK | path join urepo
^$env.FLATPAK build-init $app org.test.URepo org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK build-export $repo $app

let r = (do { ^$env.FLATPAK build-update-repo $repo } | complete)
let rc = $r.exit_code

if $rc == 139 or $rc == 134 or $rc == 136 {
  print $"FAIL: build-update-repo crashed with signal \(rc=($rc)\)"
  exit 1
}

print "PASS: build-update-repo"
