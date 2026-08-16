#!/usr/bin/env nu

# Start with no overrides dir
rm -rf ($env.HOME | path join .local share flatpak overrides)
^$env.FLATPAK --user override --socket x11 org.test.GlobalDir
if (($env.HOME | path join .local share flatpak overrides) | path type) != "dir" { exit 1 }
if (($env.HOME | path join .local share flatpak overrides org.test.GlobalDir) | path type) != "file" { exit 1 }

print "PASS: override-global-overrides-dir"
