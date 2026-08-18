#!/usr/bin/env nu

source ./libtest-nix.nu

# Build v1 of the app and export to repo
let build_dir = (make_test_app org.test.Hello stable)
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join bundle-repo) $build_dir -b stable

# Create v1 bundle
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join bundle-repo)
    ($env.TEST_DATA_DIR | path join hello-v1.flatpak)
    $"app/org.test.Hello/($env.ARCH)/stable")
ok "v1 bundle created"

# Install runtime locally so the app can run
let rt_build_dir = (make_test_runtime org.test.Platform stable)
let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active
mkdir $rt_dest
cp ($rt_build_dir | path join metadata) ($rt_dest | path join metadata)
cp -r ($rt_build_dir | path join files) ($rt_dest | path join files)
ok "runtime installed locally"

# Import v1 bundle (initial install)
^$env.FLATPAK --user build-import-bundle ($env.TEST_DATA_DIR | path join hello-v1.flatpak)
ok "v1 bundle imported"

# Verify v1 runs
let r = (do { run org.test.Hello } | complete)
let run_output = $r.stdout + $r.stderr
if ($run_output | str contains "Hello world, from a sandbox") {
    ok "v1 runs correctly"
} else {
    print "FAIL: expected 'Hello world, from a sandbox' in output"
    print $"Got: ($run_output)"
    exit 1
}

# Modify the app to produce v2 output
"#!/bin/sh\necho \"Hello v2, updated via bundle\"\n"
| save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)

# Re-export v2 and create v2 bundle
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join bundle-repo) $build_dir -b stable
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join bundle-repo)
    ($env.TEST_DATA_DIR | path join hello-v2.flatpak)
    $"app/org.test.Hello/($env.ARCH)/stable")
ok "v2 bundle created"

# Use `flatpak update --bundle=PATH` to refresh the installed app
let r2 = (do { ^$env.FLATPAK --user update $"--bundle=($env.TEST_DATA_DIR | path join hello-v2.flatpak)" } | complete)
let update_output = $r2.stdout + $r2.stderr
print $"update output: ($update_output)"
if ($update_output | str contains "Updated from bundle") {
    ok "update --bundle reported success"
} else {
    print "FAIL: expected 'Updated from bundle' message"
    print $"Got: ($update_output)"
    exit 1
}

# Verify v2 runs correctly
let r3 = (do { run org.test.Hello } | complete)
let run_output2 = $r3.stdout + $r3.stderr
if ($run_output2 | str contains "Hello v2, updated via bundle") {
    ok "v2 runs correctly after bundle update"
} else {
    print "FAIL: expected 'Hello v2, updated via bundle' in output"
    print $"Got: ($run_output2)"
    exit 1
}

print "PASS: vm-bundle-update"
