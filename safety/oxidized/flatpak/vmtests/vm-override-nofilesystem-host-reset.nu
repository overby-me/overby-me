#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
# First add a filesystem override, then reset it
^$env.FLATPAK --user override --filesystem=home org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
^$env.FLATPAK --user override --nofilesystem=host:reset org.test.Hello
assert_file_has_content $override_file "host:reset|reset"
ok "nofilesystem host:reset override written"
print "PASS: vm-override-nofilesystem-host-reset"
