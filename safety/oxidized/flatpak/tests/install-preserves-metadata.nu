#!/usr/bin/env nu

let app = $env.WORK | path join preserve
^$env.FLATPAK build-init $app org.test.Preserve org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app --share network --socket wayland
^$env.FLATPAK --user install $app

# Find the installed metadata
let meta = (do { ^find ($env.HOME | path join .local share flatpak) -path "*/org.test.Preserve/*/metadata" } | complete | get stdout | lines | get 0? | default "")
if ($meta | is-empty) {
  print "FAIL: metadata not found in installation"
  exit 1
}
^grep -q "name=org.test.Preserve" $meta
^grep -q "command=app" $meta
^grep -q "network" $meta
^grep -q "wayland" $meta

print "PASS: install-preserves-metadata"
