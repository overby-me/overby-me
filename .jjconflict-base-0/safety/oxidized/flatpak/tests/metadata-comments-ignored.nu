#!/usr/bin/env nu

let meta = "# This is a comment
; Another comment
[Application]
name=org.test.Comments
runtime=org.test.Platform/x86_64/master

[Context]
# permission comment
shared=network
"
$meta | save -f --raw ($env.WORK | path join test.meta)
^$env.FLATPAK build-init ($env.WORK | path join commentapp) org.test.Comments org.test.Sdk org.test.Platform
# Overwrite metadata with our version
cp ($env.WORK | path join test.meta) ($env.WORK | path join commentapp metadata)
^$env.FLATPAK build-finish ($env.WORK | path join commentapp) --command test --socket x11
# Check sockets were added (metadata was parsed correctly despite comments)
^grep -q "sockets" ($env.WORK | path join commentapp metadata)
^grep -q "x11" ($env.WORK | path join commentapp metadata)

print "PASS: metadata-comments-ignored"
