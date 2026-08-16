#!/usr/bin/env nu

# implicit: if it exits non-zero, nu aborts on external failure
^$env.FLATPAK --version o+e> /dev/null

print "PASS: version-nonzero-exit"
