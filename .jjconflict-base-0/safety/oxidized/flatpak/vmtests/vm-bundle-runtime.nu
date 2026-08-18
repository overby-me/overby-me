#!/usr/bin/env nu

source ./libtest-nix.nu

# Build a test runtime
let rt_build_dir = (make_test_runtime org.test.Platform stable)

# Export to an OSTree repo
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join rt-repo) $rt_build_dir -b stable

# Create a bundle file from the repo
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join rt-repo)
    ($env.TEST_DATA_DIR | path join platform.flatpak)
    $"runtime/org.test.Platform/($env.ARCH)/stable")

# Assert the bundle file exists and is non-empty
assert_has_file ($env.TEST_DATA_DIR | path join platform.flatpak)
if not (test-flag "-s" ($env.TEST_DATA_DIR | path join platform.flatpak)) {
    print "FAIL: platform.flatpak is empty"
    exit 1
}
ok "runtime bundle created"

# Import the bundle
^$env.FLATPAK --user build-import-bundle ($env.TEST_DATA_DIR | path join platform.flatpak)

# Verify the runtime is installed
let r = (do { ^$env.FLATPAK --user list --runtime } | complete)
if (($r.stdout + $r.stderr) | str contains "org.test.Platform") {
    ok "runtime listed after import"
} else {
    # Fall back to checking the directory directly
    assert_has_file ($env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active metadata)
    ok "runtime directory exists after import"
}

print "PASS: vm-bundle-runtime"
