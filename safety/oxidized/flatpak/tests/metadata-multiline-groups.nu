#!/usr/bin/env nu

^$env.FLATPAK build-init ($env.WORK | path join multigrp) org.test.MultiGrp org.test.Sdk org.test.Platform
^$env.FLATPAK build-finish ($env.WORK | path join multigrp) --command app --share network --socket x11 --device dri --filesystem home
# Check metadata has both [Application] and [Context] groups
let meta = ($env.WORK | path join multigrp metadata)
^grep -q '^\[Application\]' $meta
^grep -q '^\[Context\]' $meta
^grep -q "name=org.test.MultiGrp" $meta
^grep -q "command=app" $meta

print "PASS: metadata-multiline-groups"
