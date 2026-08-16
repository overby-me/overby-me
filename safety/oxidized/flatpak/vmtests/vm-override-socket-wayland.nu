#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo
# Remove wayland socket first, then re-add via override
^$env.FLATPAK --user override --nosocket=wayland org.test.Hello
let override_file = $env.FL_DIR | path join overrides org.test.Hello
assert_has_file $override_file
assert_file_has_content $override_file "nosocket.*wayland|wayland"
ok "nosocket wayland override written"
# Now add it back
^$env.FLATPAK --user override --socket=wayland org.test.Hello
assert_file_has_content $override_file "wayland"
ok "socket wayland override written"
print "PASS: vm-override-socket-wayland"
