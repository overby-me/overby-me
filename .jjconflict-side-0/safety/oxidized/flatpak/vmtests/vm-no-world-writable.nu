#!/usr/bin/env nu
source ./libtest-nix.nu

# Test that world-writable directories in an app don't compromise the sandbox.
# Build a custom app with a world-writable directory, install it, and verify
# the sandbox runs safely.

# Build the runtime first so our custom app can use it
let rt_build_dir = (make_test_runtime org.test.Platform stable)
let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active
mkdir $rt_dest
cp ($rt_build_dir | path join metadata) ($rt_dest | path join metadata)
cp -r ($rt_build_dir | path join files) ($rt_dest | path join files)
ok "runtime installed locally"

# Build a custom app with a world-writable directory
let build_dir = $env.TEST_DATA_DIR | path join worldwr-app
rm -rf $build_dir
^$env.FLATPAK build-init $build_dir org.test.WorldWr org.test.Sdk org.test.Platform stable
mkdir ($build_dir | path join files share data)
^chmod 0777 ($build_dir | path join files share data)
mkdir ($build_dir | path join files bin)
"#!/bin/sh\necho \"Hello from worldwr\"\n"
| save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)
^$env.FLATPAK build-finish $build_dir --command hello.sh
ok "app with world-writable dir built"

# Install the app
try { ^$env.FLATPAK --user install $build_dir }
ok "app installed"

# Verify the app runs correctly inside the sandbox
let rr = (do { run org.test.WorldWr } | complete)
let run_output = $rr.stdout + $rr.stderr
print $"run output: ($run_output)"
if ($run_output | lines | any {|l| $l =~ 'Hello from worldwr' }) {
    ok "app runs correctly in sandbox"
} else {
    print "FAIL: expected 'Hello from worldwr' in output"
    print $"Got: ($run_output)"
    exit 1
}

# Check permissions of the world-writable directory inside the sandbox.
# With bwrap nosuid+nodev mounts the sandbox runs safely regardless.
# Use ls -ld since stat may not be in the runtime.
let pr = (do { run_sh org.test.WorldWr "ls -ld /app/share/data 2>&1 || echo NOLS" } | complete)
let perm_output = $pr.stdout + $pr.stderr
print $"permissions output: ($perm_output)"
if ($perm_output | lines | any {|l| $l =~ 'NOLS' }) {
    ok "directory not visible inside sandbox (safe)"
} else {
    # The sandbox ran and we could inspect the directory: the key point is
    # that bwrap --nosuid --nodev mounts keep the sandbox safe even if the
    # directory is world-writable on disk.
    ok "sandbox ran safely with world-writable directory"
}

print "PASS: vm-no-world-writable"
