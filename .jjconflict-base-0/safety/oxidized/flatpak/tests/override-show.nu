#!/usr/bin/env nu

^$env.FLATPAK --user override --socket x11 --device dri --share network --env FOO=bar --filesystem home org.test.ShowTest

let override_file = ($env.HOME | path join .local share flatpak overrides org.test.ShowTest)
^grep -q '\[Context\]' $override_file
^grep -q "sockets" $override_file
^grep -q "x11" $override_file
^grep -q "devices" $override_file
^grep -q "dri" $override_file
^grep -q "shared" $override_file
^grep -q "network" $override_file
^grep -q "filesystems" $override_file
^grep -q "home" $override_file
^grep -q '\[Environment\]' $override_file
^grep -q "FOO=bar" $override_file

print "PASS: override-show"
