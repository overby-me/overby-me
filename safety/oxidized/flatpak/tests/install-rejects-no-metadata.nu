#!/usr/bin/env nu

# Build a directory with files/ but strip metadata (simulates an OSTree commit
# that is missing the xa.metadata field).
^$env.FLATPAK build-init ($env.WORK | path join nometa) org.test.NoMeta org.test.Sdk org.test.Platform
mkdir ($env.WORK | path join nometa files bin)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join nometa files bin h)
chmod +x ($env.WORK | path join nometa files bin h)
^$env.FLATPAK build-finish ($env.WORK | path join nometa) --command h
rm ($env.WORK | path join nometa metadata)

let r = (do { ^$env.FLATPAK --user install ($env.WORK | path join nometa) } | complete)
let out = $r.stdout + $r.stderr
let rc = $r.exit_code

if $rc == 0 {
  print "FAIL: install should have failed without metadata"
  print $"out: ($out)"
  exit 1
}
if not ($out | str contains "no metadata") {
  print "FAIL: missing 'no metadata' message"
  print $out
  exit 1
}

# Verify nothing was deployed
let deployed = (glob ($env.HOME | path join .local share flatpak "**" files) | where {|p| ($p =~ "org.test.NoMeta") and (($p | path type) == "dir")})
if ($deployed | is-not-empty) {
  print "FAIL: app should not have been deployed"
  exit 1
}

print "PASS: install-rejects-no-metadata"
