#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
^$env.FLATPAK --user override --nodevice=dri org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "dri"
ok "nodevice dri override written"
^$env.FLATPAK --user override --device=dri org.test.Hello
assert_file_has_content $override_file "dri"
ok "device dri override written"
print "PASS: vm-override-device-dri"
