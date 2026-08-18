#!/usr/bin/env nu

# CVE-2021-43860: metadata containing NUL bytes must be rejected.
^$env.FLATPAK build-init ($env.WORK | path join nul-app) org.test.NulMeta org.test.Sdk org.test.Platform stable
mkdir ($env.WORK | path join nul-app files bin)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join nul-app files bin app)
chmod +x ($env.WORK | path join nul-app files bin app)
^$env.FLATPAK build-finish ($env.WORK | path join nul-app) --command app

# Inject NUL bytes
"[Application]\nname=org.test.NulMeta\nruntime=org.test.Platform/x86_64/stable\ncommand=app\n\u{0}[Context]\nfilesystems=host;\n" | into binary | save -f --raw ($env.WORK | path join nul-app metadata)

let r = (do { ^$env.FLATPAK --user install ($env.WORK | path join nul-app) } | complete)
let output = $r.stdout + $r.stderr
if $r.exit_code == 0 {
  print "FAIL: install should reject NUL-byte metadata"
  exit 1
}

# Verify rejection mentions CVE or NUL
if not (($output | str downcase) =~ 'nul|cve-2021-43860|invalid') {
  print "FAIL: expected rejection message to mention NUL/CVE"
  print $"Got: ($output)"
  exit 1
}

print "PASS: metadata-nul-byte-rejected"
