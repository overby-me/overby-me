#!/usr/bin/env nu

^$env.FLATPAK build-init ($env.WORK | path join filterapp) org.test.Listed org.test.Sdk org.test.Platform
mkdir ($env.WORK | path join filterapp files bin)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join filterapp files bin app)
chmod +x ($env.WORK | path join filterapp files bin app)
^$env.FLATPAK build-finish ($env.WORK | path join filterapp) --command app
^$env.FLATPAK --user install ($env.WORK | path join filterapp)

^$env.FLATPAK --user list --app o> ($env.WORK | path join applist)
^$env.FLATPAK --user list --runtime o> ($env.WORK | path join rtlist)

let applist = ($env.WORK | path join applist)
let rtlist = ($env.WORK | path join rtlist)

if (do { ^grep -q "org.test.Listed" $applist } | complete).exit_code != 0 {
  print "FAIL: --app list does not contain org.test.Listed"
  ^cat $applist
  exit 1
}

if (do { ^grep -q "org.test.Listed" $rtlist } | complete).exit_code == 0 {
  print "FAIL: --runtime list should not contain org.test.Listed"
  ^cat $rtlist
  exit 1
}

print "PASS: list-filter-app"
