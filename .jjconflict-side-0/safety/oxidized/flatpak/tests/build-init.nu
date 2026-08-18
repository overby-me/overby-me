#!/usr/bin/env nu

# Test: flatpak build-init creates app directory with metadata

let app = $env.WORK | path join testapp
let meta = $app | path join metadata
^$env.FLATPAK build-init $app org.test.App org.test.Sdk org.test.Platform master

# Check metadata file exists
if ($meta | path type) != "file" {
  print $"FAIL: ($meta) does not exist"
  exit 1
}

# Check metadata contains the app name
if (do { ^grep -q "org.test.App" $meta } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain org.test.App"
  ^cat $meta
  exit 1
}

# Check files/ directory exists
if (($app | path join files) | path type) != "dir" {
  print $"FAIL: ($app)/files/ directory does not exist"
  exit 1
}

print "PASS: build-init"
