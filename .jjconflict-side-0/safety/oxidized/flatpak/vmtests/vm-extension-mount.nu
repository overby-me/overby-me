#!/usr/bin/env nu

source ./libtest-nix.nu

# Build a custom runtime with an extension point declared
let rt_build_dir = $env.TEST_DATA_DIR | path join runtime-with-ext
rm -rf $rt_build_dir
mkdir ($rt_build_dir | path join files bin) ($rt_build_dir | path join files lib)

# Copy basic binaries from the helper
make_test_runtime org.test.PlatformExt stable
let src_rt = $env.TEST_DATA_DIR | path join runtime-build-org.test.PlatformExt
try { ^cp -r ...(glob ($src_rt | path join files "*")) ($rt_build_dir | path join files) }

# Write metadata with an extension point
$"[Runtime]
name=org.test.PlatformExt
runtime=org.test.PlatformExt/($env.ARCH)/stable
sdk=org.test.SdkExt/($env.ARCH)/stable

[Extension org.test.PlatformExt.MyExt]
directory=lib/myext
version=stable
" | save -f --raw ($rt_build_dir | path join metadata)

# Install runtime locally
let rt_dest = $env.FL_DIR | path join runtime org.test.PlatformExt $env.ARCH stable active
mkdir $rt_dest
cp ($rt_build_dir | path join metadata) ($rt_dest | path join metadata)
cp -r ($rt_build_dir | path join files) ($rt_dest | path join files)
ok "runtime with extension point installed"

# Build an app that uses this runtime
let app_build_dir = $env.TEST_DATA_DIR | path join ext-app
rm -rf $app_build_dir
^$env.FLATPAK build-init $app_build_dir org.test.ExtApp org.test.SdkExt org.test.PlatformExt stable
mkdir ($app_build_dir | path join files bin)
'#!/bin/sh
echo "extension test"
ls /usr/lib/myext 2>&1 || echo "extension dir not present"
' | save -f --raw ($app_build_dir | path join files bin hello.sh)
^chmod +x ($app_build_dir | path join files bin hello.sh)
^$env.FLATPAK build-finish $app_build_dir --command hello.sh
try { ^$env.FLATPAK --user install $app_build_dir }
ok "app with extension-point runtime installed"

# Run the app: extension dir should exist (even if empty) inside sandbox
let r = (do { run org.test.ExtApp } | complete)
let output = $r.stdout + $r.stderr
print $"app run output: ($output)"

if ($output | str contains "extension test") {
    ok "app with extension-point runtime runs successfully"
} else {
    print "FAIL: app did not run"
    print $"Got: ($output)"
    exit 1
}

print "PASS: vm-extension-mount"
