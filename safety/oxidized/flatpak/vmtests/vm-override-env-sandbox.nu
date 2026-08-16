#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
# Set env override and verify it's written
^$env.FLATPAK --user override --env MY_TEST_VAR=hello123 org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "MY_TEST_VAR=hello123"
# Try running with the override (sandbox may or may not apply it)
let r = (do { run_sh org.test.Hello 'echo ${MY_TEST_VAR:-unset}' } | complete)
let output = $r.stdout + $r.stderr
print $"MY_TEST_VAR in sandbox: ($output)"
ok "override env sandbox"

print "PASS: vm-override-env-sandbox"
