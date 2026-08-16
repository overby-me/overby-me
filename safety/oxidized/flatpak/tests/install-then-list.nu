#!/usr/bin/env nu

# Build
^$env.FLATPAK build-init ($env.WORK | path join fullapp) org.test.Full org.test.Sdk org.test.Platform stable
mkdir ($env.WORK | path join fullapp files bin)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join fullapp files bin myapp)
chmod +x ($env.WORK | path join fullapp files bin myapp)
^$env.FLATPAK build-finish ($env.WORK | path join fullapp) --command myapp --share network --socket x11

# Install
^$env.FLATPAK --user install ($env.WORK | path join fullapp)

# List shows it
^$env.FLATPAK --user list o> ($env.WORK | path join list_out)
^grep -q "org.test.Full" ($env.WORK | path join list_out)

# Info works
^$env.FLATPAK --user info org.test.Full o> ($env.WORK | path join info_out)
^grep -q "org.test.Full" ($env.WORK | path join info_out)

# Info with metadata
^$env.FLATPAK --user info --show-metadata org.test.Full o> ($env.WORK | path join meta_out)
^grep -q "name=org.test.Full" ($env.WORK | path join meta_out)
^grep -q "command=myapp" ($env.WORK | path join meta_out)

# Uninstall
^$env.FLATPAK --user uninstall org.test.Full

# List no longer shows it
^$env.FLATPAK --user list o> ($env.WORK | path join list_out2)
if (do { ^grep -q "org.test.Full" ($env.WORK | path join list_out2) } | complete).exit_code == 0 {
  print "FAIL: app still in list after uninstall"
  exit 1
}

print "PASS: install-then-list"
