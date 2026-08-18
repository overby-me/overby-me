#!/usr/bin/env nu

source ./libtest-nix.nu

# Build a test app
let build_dir = (make_test_app org.test.Redir stable)

# Export to an OSTree repo
^$env.FLATPAK build-export ($env.TEST_DATA_DIR | path join redir-repo) $build_dir -b stable

# Set redirect-url on the repo
^$env.FLATPAK build-update-repo --redirect-url=http://example.com/new ($env.TEST_DATA_DIR | path join redir-repo)

# Verify the config contains the redirect-url
let config = $env.TEST_DATA_DIR | path join redir-repo config
if not (open --raw $config | str contains "redirect-url=http://example.com/new") {
    print "FAIL: redirect-url not found in repo config"
    exit 1
}

ok "redirect-url set correctly"
print "PASS: vm-build-update-repo-redirect"
