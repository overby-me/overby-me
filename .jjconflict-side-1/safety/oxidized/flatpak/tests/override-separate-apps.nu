#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user override --socket x11 org.test.App1
^$env.FLATPAK --user override --socket wayland org.test.App2

let override1 = ($env.HOME | path join .local share flatpak overrides org.test.App1)
let override2 = ($env.HOME | path join .local share flatpak overrides org.test.App2)

if not ($override1 | path exists) {
  print "FAIL: override file not created for org.test.App1"
  exit 1
}

if not ($override2 | path exists) {
  print "FAIL: override file not created for org.test.App2"
  exit 1
}

if (do { ^grep -q "x11" $override1 } | complete).exit_code != 0 {
  print "FAIL: org.test.App1 override does not contain 'x11'"
  ^cat $override1
  exit 1
}

if (do { ^grep -q "wayland" $override1 } | complete).exit_code == 0 {
  print "FAIL: org.test.App1 override should not contain 'wayland'"
  ^cat $override1
  exit 1
}

if (do { ^grep -q "wayland" $override2 } | complete).exit_code != 0 {
  print "FAIL: org.test.App2 override does not contain 'wayland'"
  ^cat $override2
  exit 1
}

if (do { ^grep -q "x11" $override2 } | complete).exit_code == 0 {
  print "FAIL: org.test.App2 override should not contain 'x11'"
  ^cat $override2
  exit 1
}

print "PASS: override-separate-apps"
