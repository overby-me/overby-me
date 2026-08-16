#!/usr/bin/env nu

# Test: build-init creates valid metadata that can be verified
^$env.FLATPAK build-init ($env.WORK | path join testapp) org.test.App org.test.Sdk org.test.Platform

let metadata = ($env.WORK | path join testapp metadata)

if not ($metadata | path exists) {
  print $"FAIL: metadata file not created at ($metadata)"
  exit 1
}

if (do { ^grep -q "org.test.App" $metadata } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain 'org.test.App'"
  ^cat $metadata
  exit 1
}

if (do { ^grep -q "org.test.Sdk" $metadata } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain 'org.test.Sdk'"
  ^cat $metadata
  exit 1
}

if (do { ^grep -q "org.test.Platform" $metadata } | complete).exit_code != 0 {
  print "FAIL: metadata does not contain 'org.test.Platform'"
  ^cat $metadata
  exit 1
}

print "PASS: build-init creates valid metadata with expected content"
