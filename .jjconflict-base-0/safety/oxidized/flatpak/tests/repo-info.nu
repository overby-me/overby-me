#!/usr/bin/env nu

let repoapp = $env.WORK | path join repoapp
^$env.FLATPAK build-init $repoapp org.test.Repo org.test.Sdk org.test.Platform
mkdir ($repoapp | path join files bin)
"#!/bin/sh\n" | save -f --raw ($repoapp | path join files bin app)
^chmod +x ($repoapp | path join files bin app)
^$env.FLATPAK build-finish $repoapp --command app
^$env.FLATPAK build-export ($env.WORK | path join inforepo) $repoapp

let _output = (do { ^$env.FLATPAK repo ($env.WORK | path join inforepo) } | complete)

# Check it doesn't crash (segfault=139, abort=134, illegal=136)
let rc = (do { ^$env.FLATPAK repo ($env.WORK | path join inforepo) } | complete).exit_code
if $rc in [139 134 136] {
  print $"FAIL: repo command crashed with signal \(exit code ($rc)\)"
  exit 1
}

print "PASS: repo-info"
