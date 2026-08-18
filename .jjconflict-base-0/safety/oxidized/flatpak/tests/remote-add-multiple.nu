#!/usr/bin/env nu

^$env.FLATPAK --user remote-add remote1 https://example.com/repo1
^$env.FLATPAK --user remote-add remote2 https://example.com/repo2
^$env.FLATPAK --user remotes o> ($env.WORK | path join remotes_out)
^grep -q "remote1" ($env.WORK | path join remotes_out)
^grep -q "remote2" ($env.WORK | path join remotes_out)

print "PASS: remote-add-multiple"
