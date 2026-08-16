#!/usr/bin/env nu

let app = $env.WORK | path join idemp
let meta = $app | path join metadata
^$env.FLATPAK build-init $app org.test.Idemp org.test.Sdk org.test.Platform
^$env.FLATPAK build-finish $app --command app1 --share network
^$env.FLATPAK build-finish $app --command app2 --socket x11

# command should be overwritten to app2
if (do { ^grep -q "command=app2" $meta } | complete).exit_code != 0 {
  print "FAIL: command was not overwritten to app2"
  ^cat $meta
  exit 1
}

# network should still be there from first run
if (do { ^grep -q "network" $meta } | complete).exit_code != 0 {
  print "FAIL: network share from first build-finish is missing"
  ^cat $meta
  exit 1
}

# x11 should be added
if (do { ^grep -q "x11" $meta } | complete).exit_code != 0 {
  print "FAIL: x11 socket from second build-finish is missing"
  ^cat $meta
  exit 1
}

print "PASS: build-finish-idempotent"
