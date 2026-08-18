#!/usr/bin/env nu

let app = $env.WORK | path join fmtapp
let meta = $app | path join metadata
^$env.FLATPAK build-init $app org.test.Format org.test.Sdk org.test.Platform stable

if (do { ^grep -q '^\[Application\]' $meta } | complete).exit_code != 0 {
  print "FAIL: metadata missing [Application] group header"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "^name=org.test.Format" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata missing name=org.test.Format"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "^runtime=org.test.Platform/" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata missing runtime=org.test.Platform/"
  ^cat $meta
  exit 1
}

if (do { ^grep -q "^sdk=org.test.Sdk/" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata missing sdk=org.test.Sdk/"
  ^cat $meta
  exit 1
}

print "PASS: build-init-metadata-format"
