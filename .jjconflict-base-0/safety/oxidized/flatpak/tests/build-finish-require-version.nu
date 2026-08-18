#!/usr/bin/env nu

let app = $env.WORK | path join myapp
let meta = $app | path join metadata
^$env.FLATPAK build-init $app org.test.App org.test.Sdk org.test.Platform
^$env.FLATPAK build-finish $app --command foo --require-version=0.0.1

if (do { ^grep -q "required-flatpak=0.0.1" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain 'required-flatpak=0.0.1'"
  ^cat $meta
  exit 1
}

print "PASS: build-finish-require-version"
