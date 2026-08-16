#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
^$env.FLATPAK --user override --allow=multiarch org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "multiarch"
ok "allow multiarch override written"
^$env.FLATPAK --user override --disallow=multiarch org.test.Hello
assert_file_has_content $override_file "multiarch"
ok "disallow multiarch override written"
print "PASS: vm-override-allow-disallow"
