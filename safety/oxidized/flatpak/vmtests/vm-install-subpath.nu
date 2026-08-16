#!/usr/bin/env nu
source ./libtest-nix.nu

# Build a custom test app with two subdirectories under files/.
let build_dir = $env.TEST_DATA_DIR | path join subpath-app
rm -rf $build_dir
^$env.FLATPAK build-init $build_dir org.test.Sub org.test.Sdk org.test.Platform stable
mkdir ($build_dir | path join files bin) ($build_dir | path join files share)
"#!/bin/sh\necho \"Hello from subpath test\"\n"
| save -f ($build_dir | path join files bin hello.sh)
^chmod +x ($build_dir | path join files bin hello.sh)
"shared data\n" | save -f ($build_dir | path join files share data.txt)
^$env.FLATPAK build-finish $build_dir --command hello.sh
ok "built app with bin/ and share/"

# Install with --subpath=/bin only.
^$env.FLATPAK --user install --subpath=/bin $build_dir
ok "subpath install completed"

let deploy = $env.FL_DIR | path join app org.test.Sub $env.ARCH stable active
assert_has_dir ($deploy | path join files bin)
assert_has_file ($deploy | path join files bin hello.sh)
assert_not_has_file ($deploy | path join files share data.txt)
assert_has_file ($deploy | path join subpaths)
ok "only /bin was installed; share/ excluded; subpaths file written"

print "PASS: vm-install-subpath"
