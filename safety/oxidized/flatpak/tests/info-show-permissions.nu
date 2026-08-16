#!/usr/bin/env nu

let app = $env.WORK | path join perminfo
^$env.FLATPAK build-init $app org.test.PermInfo org.test.Sdk org.test.Platform
mkdir ($app | path join files bin)
"#!/bin/sh\n" | save -f --raw ($app | path join files bin app)
^chmod +x ($app | path join files bin app)
^$env.FLATPAK build-finish $app --command app --share network --socket x11
^$env.FLATPAK --user install $app

let perm_out = $env.WORK | path join perm_out
^$env.FLATPAK --user info --show-permissions org.test.PermInfo o> $perm_out

if (do { ^grep -q "network" $perm_out } | complete).exit_code != 0 {
  print "FAIL: --show-permissions output does not contain 'network'"
  ^cat $perm_out
  exit 1
}

if (do { ^grep -q "x11" $perm_out } | complete).exit_code != 0 {
  print "FAIL: --show-permissions output does not contain 'x11'"
  ^cat $perm_out
  exit 1
}

print "PASS: info-show-permissions"
