#!/usr/bin/env nu

let app = $env.WORK | path join cmdinfo
^$env.FLATPAK build-init $app org.test.CmdInfo org.test.Sdk org.test.Platform stable
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin mycommand)
^chmod +x ($app | path join files bin mycommand)
^$env.FLATPAK build-finish $app --command mycommand
^$env.FLATPAK --user install $app

let r = (do { ^$env.FLATPAK --user info org.test.CmdInfo } | complete)
let output = ($r.stdout + $r.stderr)

if not ($output | str contains "mycommand") {
  print "FAIL: info output does not contain 'mycommand'"
  print $output
  exit 1
}

print "PASS: info-command"
