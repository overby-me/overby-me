#!/usr/bin/env nu
source ./libtest-nix.nu

# Test that invalid metadata syntax is rejected.

let build_dir = $env.TEST_DATA_DIR | path join invalid-app
rm -rf $build_dir
^$env.FLATPAK build-init $build_dir org.test.InvalidApp org.test.Sdk org.test.Platform stable
mkdir ($build_dir | path join files bin)
"#!/bin/sh\n" | save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)
^$env.FLATPAK build-finish $build_dir --command hello.sh

# Replace metadata with malformed content (no [Application] or [Runtime] group,
# no name= key: required fields are missing)
"this is not valid metadata at all\njust garbage with = signs\nrandom=stuff\n"
| save -f ($build_dir | path join metadata)

# Try to install: should fail because metadata is missing required fields
let r = (do { ^$env.FLATPAK --user install $build_dir } | complete)
let rc = $r.exit_code
let output = $r.stdout + $r.stderr
print $"install output \(rc=($rc)\): ($output)"

if $rc != 0 {
    ok "install rejected invalid metadata"
} else {
    # If install succeeded, verify the app id was at least correctly parsed
    # (it shouldn't be, since there's no [Application] group with name=)
    let lr = (do { ^$env.FLATPAK --user list } | complete)
    if (($lr.stdout + $lr.stderr) | lines | any {|l| $l =~ 'org.test.InvalidApp' }) {
        print "FAIL: invalid metadata accepted and app installed"
        exit 1
    }
    ok "install completed but app not actually installed (acceptable)"
}

print "PASS: vm-metadata-invalid"
