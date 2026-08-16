#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
^$env.FLATPAK --user override --system-own-name=org.test.SysOwned --system-talk-name=org.test.SysTalked org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "org.test.SysOwned"
assert_file_has_content $override_file "org.test.SysTalked"
ok "system bus name overrides written"
print "PASS: vm-override-bus-system"
