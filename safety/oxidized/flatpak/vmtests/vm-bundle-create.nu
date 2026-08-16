#!/usr/bin/env nu

source ./libtest-nix.nu

# Build a test app
let build_dir = (make_test_app org.test.Hello stable)

# Export to an OSTree repo
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join bundle-repo) $build_dir -b stable

# Create a .flatpak bundle from the repo
(^$env.FLATPAK build-bundle ($env.TEST_DATA_DIR | path join bundle-repo)
    ($env.TEST_DATA_DIR | path join hello.flatpak)
    $"app/org.test.Hello/($env.ARCH)/stable")

# Verify the bundle file exists and is non-empty
assert_has_file ($env.TEST_DATA_DIR | path join hello.flatpak)
if not (test-flag "-s" ($env.TEST_DATA_DIR | path join hello.flatpak)) {
    print "FAIL: hello.flatpak is empty"
    exit 1
}

ok "bundle created"
print "PASS: vm-bundle-create"
