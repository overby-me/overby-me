#!/usr/bin/env nu

# Build an app with two subdirectories under files/.
^$env.FLATPAK build-init ($env.WORK | path join subapp) org.test.Sub org.test.Sdk org.test.Platform
mkdir ($env.WORK | path join subapp files bin) ($env.WORK | path join subapp files share)
"#!/bin/sh\n" | save -f --raw ($env.WORK | path join subapp files bin hello)
chmod +x ($env.WORK | path join subapp files bin hello)
"shared data\n" | save -f --raw ($env.WORK | path join subapp files share data.txt)
^$env.FLATPAK build-finish ($env.WORK | path join subapp) --command hello

# Install with --subpath=/bin only.
^$env.FLATPAK --user install --subpath=/bin ($env.WORK | path join subapp)

let install_dir = ($env.HOME | path join .local share flatpak)
let deploy_dir = (glob ($install_dir | path join "**" org.test.Sub) | where {|p| ($p | path type) == "dir"} | get 0? | default "")
if ($deploy_dir | is-empty) {
  print "FAIL: org.test.Sub not installed"
  exit 1
}

# Find the active deploy directory under that.
let active = (glob ($deploy_dir | path join "**" files) | where {|p| ($p | path type) == "dir"} | get 0? | default "")
if ($active | is-empty) {
  print $"FAIL: no files/ directory under ($deploy_dir)"
  exit 1
}

if not (($active | path join bin hello) | path exists) {
  print $"FAIL: expected ($active)/bin/hello to exist"
  glob ($active | path join "**" "*") | where {|p| ($p | path type) == "file"} | each {|p| print $p }
  exit 1
}

if (($active | path join share data.txt) | path exists) {
  print $"FAIL: ($active)/share/data.txt should NOT exist \(was excluded by --subpath\)"
  exit 1
}

# Subpaths file should be recorded next to the files directory.
let parent = ($active | path dirname)
if not (($parent | path join subpaths) | path exists) {
  print $"FAIL: expected ($parent)/subpaths file"
  exit 1
}

print "PASS: install-subpath"
