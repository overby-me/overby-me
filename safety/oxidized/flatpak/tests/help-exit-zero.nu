#!/usr/bin/env nu

^$env.FLATPAK --help o+e> /dev/null

print "PASS: help-exit-zero"
