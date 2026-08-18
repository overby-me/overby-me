#!/usr/bin/env nu
source ./libtest-nix.nu

# Test --require-version: app requiring a newer flatpak should fail to install,
# while an app requiring an older flatpak version should install fine.

# App A: requires unrealistically high version
let build_dir_a = $env.TEST_DATA_DIR | path join req-app-a
rm -rf $build_dir_a
^$env.FLATPAK build-init $build_dir_a org.test.NeedsNewer org.test.Sdk org.test.Platform stable
mkdir ($build_dir_a | path join files bin)
"#!/bin/sh\necho \"Hello from needs-newer\"\n" | save -f ($build_dir_a | path join files bin hello.sh)
^chmod +x ($build_dir_a | path join files bin hello.sh)
^$env.FLATPAK build-finish $build_dir_a --command hello.sh --require-version=99.0.0
ok "built app A requiring flatpak 99.0.0"

assert_file_has_content ($build_dir_a | path join metadata) "required-flatpak=99.0.0"

# Set up runtime so the install dependency is satisfied
make_test_runtime org.test.Platform stable
let rt_dest = $env.FL_DIR | path join runtime org.test.Platform $env.ARCH stable active
mkdir $rt_dest
cp ($env.TEST_DATA_DIR | path join runtime-build-org.test.Platform metadata) ($rt_dest | path join metadata)
cp -r ($env.TEST_DATA_DIR | path join runtime-build-org.test.Platform files) ($rt_dest | path join files)
ok "runtime installed locally"

let ir = (do { ^$env.FLATPAK --user install $build_dir_a } | complete)
let install_status = $ir.exit_code
let install_out = $ir.stdout + $ir.stderr
print $"install output: ($install_out)"
if $install_status == 0 {
  print "FAIL: install of app requiring flatpak 99.0.0 unexpectedly succeeded"
  exit 1
}
if not ($install_out | lines | any {|l| $l =~ 'needs Flatpak' }) {
  print "FAIL: expected stderr to mention 'needs Flatpak'"
  print $install_out
  exit 1
}
ok "install of app A correctly rejected with version-check error"

# App B: requires a low version that we satisfy
let build_dir_b = $env.TEST_DATA_DIR | path join req-app-b
rm -rf $build_dir_b
^$env.FLATPAK build-init $build_dir_b org.test.NeedsOld org.test.Sdk org.test.Platform stable
mkdir ($build_dir_b | path join files bin)
"#!/bin/sh\necho \"Hello from needs-old\"\n" | save -f ($build_dir_b | path join files bin hello.sh)
^chmod +x ($build_dir_b | path join files bin hello.sh)
^$env.FLATPAK build-finish $build_dir_b --command hello.sh --require-version=0.0.1
ok "built app B requiring flatpak 0.0.1"

^$env.FLATPAK --user install $build_dir_b
ok "install of app B with require-version=0.0.1 succeeded"

print "PASS: vm-version-check"
