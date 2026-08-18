#!/usr/bin/env nu

let app1 = $env.WORK | path join multi1
^$env.FLATPAK build-init $app1 org.test.Multi1 org.test.Sdk org.test.Platform
mkdir ($app1 | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app1 | path join files bin app)
^chmod +x ($app1 | path join files bin app)
^$env.FLATPAK build-finish $app1 --command app

let app2 = $env.WORK | path join multi2
^$env.FLATPAK build-init $app2 org.test.Multi2 org.test.Sdk org.test.Platform
mkdir ($app2 | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app2 | path join files bin app)
^chmod +x ($app2 | path join files bin app)
^$env.FLATPAK build-finish $app2 --command app

^$env.FLATPAK --user install $app1
^$env.FLATPAK --user install $app2

let list_out = $env.WORK | path join list_out
^$env.FLATPAK --user list o+e> $list_out

if (do { ^grep -q "org.test.Multi1" $list_out } | complete).exit_code != 0 {
  print "FAIL: org.test.Multi1 not found in list output"
  ^cat $list_out
  exit 1
}

if (do { ^grep -q "org.test.Multi2" $list_out } | complete).exit_code != 0 {
  print "FAIL: org.test.Multi2 not found in list output"
  ^cat $list_out
  exit 1
}

print "PASS: install-multiple-apps"
