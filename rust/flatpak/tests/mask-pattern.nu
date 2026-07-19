#!/usr/bin/env nu

mkdir ($env.HOME | path join .local share flatpak)

^$env.FLATPAK --user mask org.test.Masked

# Check that a mask file was created somewhere under the installation
let masks_dir = ($env.HOME | path join .local share flatpak)
let files = (glob ($masks_dir | path join "**" "*") | where {|p| ($p | path type) == "file"})
let found = ($files | any {|f| (try { open --raw $f | str contains "org.test.Masked" } catch { false }) })
if not $found {
  print $"FAIL: mask for org.test.Masked not found under ($masks_dir)"
  $files | first 20 | each {|f| print $f }
  exit 1
}

print "PASS: mask-pattern"
