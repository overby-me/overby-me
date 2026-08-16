#!/usr/bin/env nu

source ./libtest-nix.nu

# Build a test app
let build_dir = (make_test_app org.test.DefBranch stable)

# Export to an OSTree repo
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join branch-repo) $build_dir -b stable

# Set the default branch on the repo
^$env.FLATPAK build-update-repo --default-branch=beta ($env.TEST_DATA_DIR | path join branch-repo)

# Verify the config file contains the expected default-branch
let config = $env.TEST_DATA_DIR | path join branch-repo config
if not (open --raw $config | str contains "default-branch=beta") {
    print "FAIL: default-branch not found in repo config"
    print (open --raw $config)
    exit 1
}

ok "default-branch set in repo config"
print "PASS: vm-build-update-repo-default-branch"
