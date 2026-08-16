#!/usr/bin/env nu
source ./libtest-nix.nu

# CVE-2021-43860: App with NUL-hidden permissions must be rejected.
# Build an app and inject NUL bytes into its metadata to hide permissions.

let build_dir = $env.TEST_DATA_DIR | path join nul-app
rm -rf $build_dir
^$env.FLATPAK build-init $build_dir org.test.NulApp org.test.Sdk org.test.Platform stable
mkdir ($build_dir | path join files bin)
"#!/bin/sh\n" | save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)
^$env.FLATPAK build-finish $build_dir --command hello.sh

# Inject NUL bytes into metadata to hide [Context] permissions
# Format: legitimate content + NUL + hidden permissions
"[Application]\nname=org.test.NulApp\nruntime=org.test.Platform/x86_64/stable\ncommand=hello.sh\n\u{00}[Context]\nfilesystems=host;\n"
| save -f ($build_dir | path join metadata)

# Verify the file actually has NUL bytes
if (do { ^grep -aPq '\x00' ($build_dir | path join metadata) } | complete).exit_code != 0 {
    print "FAIL: test setup error - NUL byte not in metadata"
    exit 1
}
ok "metadata file has NUL bytes (test setup)"

# Try to install: must fail due to NUL byte rejection
let r = (do { ^$env.FLATPAK --user install $build_dir } | complete)
let rc = $r.exit_code
let output = $r.stdout + $r.stderr
print $"install output \(rc=($rc)\): ($output)"

if ($rc != 0) and ($output | lines | any {|l| $l =~ '(?i)NUL|CVE-2021-43860|invalid|reject' }) {
    ok "install rejected metadata with NUL bytes"
} else {
    print $"FAIL: install should have rejected NUL-byte metadata \(rc=($rc)\)"
    print $"Got: ($output)"
    exit 1
}

# Verify the app was NOT installed
let lr = (do { ^$env.FLATPAK --user list } | complete)
if (($lr.stdout + $lr.stderr) | lines | any {|l| $l =~ 'org.test.NulApp' }) {
    print "FAIL: app was installed despite NUL bytes"
    exit 1
}
ok "app not present after rejected install"

print "PASS: vm-metadata-hidden-perms"
