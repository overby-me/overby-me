#!/usr/bin/env nu

^$env.FLATPAK --user remote-add alpha-remote https://alpha.example.com/repo
^$env.FLATPAK --user remote-add beta-remote https://beta.example.com/repo
^$env.FLATPAK --user remote-add gamma-remote https://gamma.example.com/repo
^$env.FLATPAK --user remotes o> ($env.WORK | path join order_out)
^grep -q "alpha-remote" ($env.WORK | path join order_out)
^grep -q "beta-remote" ($env.WORK | path join order_out)
^grep -q "gamma-remote" ($env.WORK | path join order_out)

print "PASS: remote-add-order-preserved"
