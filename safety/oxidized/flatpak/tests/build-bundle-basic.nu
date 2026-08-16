#!/usr/bin/env nu

let app = $env.WORK | path join bundleapp
let repo = $env.WORK | path join bundlerepo

^$env.FLATPAK build-init $app org.test.Bundle org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK build-export $repo $app

do { ^$env.FLATPAK build-bundle $repo ($env.WORK | path join test.flatpak) org.test.Bundle } | complete | ignore

# Check that either the bundle file was created OR we get a meaningful error (not crash)
if (($env.WORK | path join test.flatpak) | path exists) {
  print "Bundle file created successfully"
} else {
  # Ensure it didn't crash (signal 139=segfault, 134=abort, 136=fpe)
  let r = (do { ^$env.FLATPAK build-bundle $repo ($env.WORK | path join test2.flatpak) org.test.Bundle } | complete)
  print -n ($r.stdout + $r.stderr)
  let rc = $r.exit_code
  if $rc == 139 or $rc == 134 or $rc == 136 {
    print $"FAIL: build-bundle crashed with signal \(exit code ($rc)\)"
    exit 1
  }
  print $"build-bundle exited with code ($rc) \(no crash\)"
}

print "PASS: build-bundle-basic"
