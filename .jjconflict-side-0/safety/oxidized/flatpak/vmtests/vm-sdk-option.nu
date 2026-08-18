#!/usr/bin/env nu
source ./libtest-nix.nu

# Test that build-finish --sdk= is recorded in metadata

let build_dir = $env.TEST_DATA_DIR | path join sdk-test-app
rm -rf $build_dir
^$env.FLATPAK build-init $build_dir org.test.SdkApp org.test.Sdk org.test.Platform stable

mkdir ($build_dir | path join files bin)
"#!/bin/sh\necho \"Hello from sdk test\"\n" | save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)

# Use --sdk= option
^$env.FLATPAK build-finish $build_dir --command hello.sh --sdk=org.test.Sdk/x86_64/stable

# Verify sdk is recorded in the metadata file
assert_has_file ($build_dir | path join metadata)
assert_file_has_content ($build_dir | path join metadata) "sdk=org.test.Sdk/x86_64/stable"
ok "build-finish --sdk= recorded in metadata"

# Also test the --sdk KEY form (space-separated)
let build_dir2 = $env.TEST_DATA_DIR | path join sdk-test-app2
rm -rf $build_dir2
^$env.FLATPAK build-init $build_dir2 org.test.SdkApp2 org.test.Sdk org.test.Platform stable

mkdir ($build_dir2 | path join files bin)
"#!/bin/sh\necho \"Hello from sdk test 2\"\n" | save -f ($build_dir2 | path join files bin hello.sh)
^chmod +x ($build_dir2 | path join files bin hello.sh)

^$env.FLATPAK build-finish $build_dir2 --command hello.sh --sdk org.test.Sdk/x86_64/stable

assert_has_file ($build_dir2 | path join metadata)
assert_file_has_content ($build_dir2 | path join metadata) "sdk=org.test.Sdk/x86_64/stable"
ok "build-finish --sdk (space-separated) recorded in metadata"

print "PASS: vm-sdk-option"
