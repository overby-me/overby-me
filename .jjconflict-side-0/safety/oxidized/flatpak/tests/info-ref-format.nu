#!/usr/bin/env nu

let app = $env.WORK | path join refinfo
^$env.FLATPAK build-init $app org.test.RefFmt org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app
^$env.FLATPAK --user install $app
let r = (do { ^$env.FLATPAK --user info org.test.RefFmt } | complete)
let output = ($r.stdout + $r.stderr)
if not ($output | str contains "Ref") { exit 1 }
if not ($output | str contains "app/") { exit 1 }
if not ($output | str contains "org.test.RefFmt") { exit 1 }

print "PASS: info-ref-format"
