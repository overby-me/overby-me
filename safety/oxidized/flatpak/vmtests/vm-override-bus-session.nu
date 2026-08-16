#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
^$env.FLATPAK --user override --own-name=org.test.Owned --talk-name=org.test.Talked org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "org.test.Owned"
assert_file_has_content $override_file "org.test.Talked"
ok "session bus name overrides written"
print "PASS: vm-override-bus-session"
