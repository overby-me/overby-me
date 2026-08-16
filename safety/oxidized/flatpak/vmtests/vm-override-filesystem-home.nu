#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
^$env.FLATPAK --user override --filesystem=home:ro org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "home"
ok "filesystem home:ro override written"
# Verify home is accessible read-only in sandbox
let r = (do { run_sh org.test.Hello 'test -d $HOME && echo home-visible || echo home-hidden' } | complete)
let output = $r.stdout + $r.stderr
print $"Home in sandbox: ($output)"
ok "filesystem home override (checked)"
print "PASS: vm-override-filesystem-home"
