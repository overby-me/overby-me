#!/usr/bin/env nu

source ./libtest-nix.nu

# Build v1 of the app and export to repo
let build_dir = (make_test_app org.test.Hello stable)
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join bundle-repo) $build_dir -b stable

# Create v1 bundle
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join bundle-repo)
    ($env.TEST_DATA_DIR | path join hello-v1.flatpak)
    $"app/org.test.Hello/($env.ARCH)/stable")
assert_has_file ($env.TEST_DATA_DIR | path join hello-v1.flatpak)
ok "v1 bundle created"

# Install runtime locally so the app can run
let rt_build_dir = (make_test_runtime org.test.Platform stable)
let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active
mkdir $rt_dest
cp ($rt_build_dir | path join metadata) ($rt_dest | path join metadata)
cp -r ($rt_build_dir | path join files) ($rt_dest | path join files)
ok "runtime installed locally"

# Import v1 bundle
^$env.FLATPAK --user build-import-bundle ($env.TEST_DATA_DIR | path join hello-v1.flatpak)
ok "v1 bundle imported"

# Verify v1 runs
let r = (do { run org.test.Hello } | complete)
let run_output = $r.stdout + $r.stderr
print $"v1 run output: ($run_output)"
if ($run_output | str contains "Hello world, from a sandbox") {
    ok "v1 app runs correctly"
} else {
    print "FAIL: expected 'Hello world, from a sandbox' in output"
    print $"Got: ($run_output)"
    exit 1
}

# Modify app to produce v2 output
"#!/bin/sh\necho \"Hello v2, updated\"\n"
| save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)

# Re-export v2 to the same repo
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join bundle-repo) $build_dir -b stable
^$env.FLATPAK build-update-repo ($env.TEST_DATA_DIR | path join bundle-repo)
ok "v2 exported to repo"

# Create v2 bundle
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join bundle-repo)
    ($env.TEST_DATA_DIR | path join hello-v2.flatpak)
    $"app/org.test.Hello/($env.ARCH)/stable")
assert_has_file ($env.TEST_DATA_DIR | path join hello-v2.flatpak)
ok "v2 bundle created"

# Import v2 bundle (updates the existing app)
^$env.FLATPAK --user build-import-bundle ($env.TEST_DATA_DIR | path join hello-v2.flatpak)
ok "v2 bundle imported"

# Verify v2 runs
let r2 = (do { run org.test.Hello } | complete)
let run_output2 = $r2.stdout + $r2.stderr
print $"v2 run output: ($run_output2)"
if ($run_output2 | str contains "Hello v2, updated") {
    ok "v2 app runs correctly after bundle update"
} else {
    print "FAIL: expected 'Hello v2, updated' in output"
    print $"Got: ($run_output2)"
    exit 1
}

print "PASS: vm-bundle-update-as-bundle"
