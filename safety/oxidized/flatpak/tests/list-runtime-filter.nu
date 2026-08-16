#!/usr/bin/env nu

^$env.FLATPAK build-init ($env.WORK | path join rtfilt) org.test.RTFilt org.test.Sdk org.test.Platform
mkdir ($env.WORK | path join rtfilt files bin)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join rtfilt files bin app)
^$env.FLATPAK build-finish ($env.WORK | path join rtfilt) --command app
^$env.FLATPAK --user install ($env.WORK | path join rtfilt)
^$env.FLATPAK --user list --runtime o> ($env.WORK | path join rtout)
if (do { ^grep -q "org.test.RTFilt" ($env.WORK | path join rtout) } | complete).exit_code == 0 {
  print "FAIL: app should not appear in --runtime list"
  exit 1
}
^$env.FLATPAK --user list --app o> ($env.WORK | path join appout)
^grep -q "org.test.RTFilt" ($env.WORK | path join appout)

print "PASS: list-runtime-filter"
