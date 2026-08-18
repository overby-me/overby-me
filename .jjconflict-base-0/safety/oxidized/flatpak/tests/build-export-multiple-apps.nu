#!/usr/bin/env nu

let app1 = $env.WORK | path join app1
let app2 = $env.WORK | path join app2
let repo = $env.WORK | path join multirepo

# App 1
^$env.FLATPAK build-init $app1 org.test.App1 org.test.Sdk org.test.Platform
mkdir ($app1 | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app1 | path join files bin app1)
^chmod +x ($app1 | path join files bin app1)
^$env.FLATPAK build-finish $app1 --command app1

# App 2
^$env.FLATPAK build-init $app2 org.test.App2 org.test.Sdk org.test.Platform
mkdir ($app2 | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app2 | path join files bin app2)
^chmod +x ($app2 | path join files bin app2)
^$env.FLATPAK build-finish $app2 --command app2

# Export both to same repo
try { ^$env.FLATPAK build-export $repo $app1 }
try { ^$env.FLATPAK build-export $repo $app2 }

# Both should exist in the repo
if (($repo | path join app org.test.App1) | path type) != "dir" {
  print "FAIL: App1 not in repo"
  try { ^ls -R $repo }
  exit 1
}

if (($repo | path join app org.test.App2) | path type) != "dir" {
  print "FAIL: App2 not in repo"
  try { ^ls -R $repo }
  exit 1
}

print "PASS: build-export-multiple-apps"
