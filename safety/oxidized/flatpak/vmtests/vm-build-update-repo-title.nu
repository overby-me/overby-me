#!/usr/bin/env nu

source ./libtest-nix.nu

# Build a test app
let build_dir = (make_test_app org.test.Title stable)

# Export to an OSTree repo
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join title-repo) $build_dir -b stable

# Set a title on the repo
^$env.FLATPAK build-update-repo "--title=Test Repo" ($env.TEST_DATA_DIR | path join title-repo)

# Verify the config contains the expected title
let config = $env.TEST_DATA_DIR | path join title-repo config
if not (open --raw $config | str contains "title=Test Repo") {
    print "FAIL: title not found in repo config"
    print (open --raw $config)
    exit 1
}

ok "title set in repo config"
print "PASS: vm-build-update-repo-title"
