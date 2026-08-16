#!/usr/bin/env nu

^$env.FLATPAK -u config o> ($env.WORK | path join u_out)
^$env.FLATPAK --user config o> ($env.WORK | path join user_out)
# Both should produce output containing "user"
^grep -q "user" ($env.WORK | path join u_out)
^grep -q "user" ($env.WORK | path join user_out)

print "PASS: global-user-flag"
