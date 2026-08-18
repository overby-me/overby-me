#!/usr/bin/env nu

let app = $env.WORK | path join permapp
^$env.FLATPAK build-init $app org.test.App org.test.Sdk org.test.Platform
^$env.FLATPAK build-finish $app --command app --share network --socket x11 --socket wayland --filesystem home --device dri

let meta = $app | path join metadata

if ($meta | path type) != "file" {
  print $"FAIL: metadata file not found at ($meta)"
  exit 1
}

if (do { ^grep -q "command=app" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain command=app"
  ^cat $meta
  exit 1
}

if (do { ^grep -qi "shared.*network\|network.*shared" $meta } | complete).exit_code != 0 and (do { ^grep -q "network" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain network in shared"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "x11" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain x11 in sockets"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "wayland" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain wayland in sockets"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "home" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain home in filesystems"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "dri" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain dri in devices"
  ^cat $meta
  exit 1
}

print "PASS: build-finish-permissions"
