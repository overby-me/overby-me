#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
^$env.FLATPAK --user override --nofilesystem=host org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "host"
ok "nofilesystem host override written"
print "PASS: vm-override-nofilesystem-host"
