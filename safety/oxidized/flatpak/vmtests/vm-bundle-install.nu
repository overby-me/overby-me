#!/usr/bin/env nu

source ./libtest-nix.nu

# Build test app and export to repo
let build_dir = (make_test_app org.test.Hello stable)
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join bundle-repo) $build_dir -b stable

# Create bundle file from repo
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join bundle-repo)
    ($env.TEST_DATA_DIR | path join hello.flatpak)
    $"app/org.test.Hello/($env.ARCH)/stable")

assert_has_file ($env.TEST_DATA_DIR | path join hello.flatpak)
if not (test-flag "-s" ($env.TEST_DATA_DIR | path join hello.flatpak)) {
    print "FAIL: hello.flatpak is empty"
    exit 1
}
ok "bundle created"

# Install runtime locally so the app can run
let rt_build_dir = (make_test_runtime org.test.Platform stable)
let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active
mkdir $rt_dest
cp ($rt_build_dir | path join metadata) ($rt_dest | path join metadata)
cp -r ($rt_build_dir | path join files) ($rt_dest | path join files)
ok "runtime installed locally"

# Import the bundle
^$env.FLATPAK --user build-import-bundle ($env.TEST_DATA_DIR | path join hello.flatpak)
ok "bundle imported"

# Verify it's installed
let r = (do { ^$env.FLATPAK --user list } | complete)
let list_output = $r.stdout + $r.stderr
if ($list_output | str contains "org.test.Hello") {
    ok "app listed after bundle import"
} else {
    print "FAIL: org.test.Hello not found in list"
    print $"Got: ($list_output)"
    exit 1
}

# Verify it can run
let r2 = (do { run org.test.Hello } | complete)
let run_output = $r2.stdout + $r2.stderr
if ($run_output | str contains "Hello world, from a sandbox") {
    ok "app runs correctly from bundle"
} else {
    print "FAIL: expected 'Hello world, from a sandbox' in output"
    print $"Got: ($run_output)"
    exit 1
}

print "PASS: vm-bundle-install"
