#!/usr/bin/env nu
source ./libtest-nix.nu

# Build an app then strip the metadata file. Equivalent to an OSTree commit
# missing xa.metadata: the install must reject it instead of installing a
# silently broken app.
let build_dir = (make_test_app org.test.NoMeta stable)
rm -f ($build_dir | path join metadata)
ok "metadata stripped from build dir"

let r = (do { ^$env.FLATPAK --user install $build_dir } | complete)
let output = $r.stdout + $r.stderr
let rc = $r.exit_code

print $"install output: ($output)"
print $"install rc: ($rc)"

if $rc == 0 {
    print "FAIL: install should have failed without metadata"
    exit 1
}
ok "install rejected (no metadata)"

if ($output | lines | any {|l| $l =~ 'no metadata' }) {
    ok "error message mentions missing metadata"
} else {
    print "FAIL: error message did not mention missing metadata"
    print $"Got: ($output)"
    exit 1
}

# Verify nothing was deployed.
let fr = (do { ^find $env.FL_DIR -path '*org.test.NoMeta*' -type d } | complete)
if ($fr.stdout | str trim | is-not-empty) {
    print "FAIL: org.test.NoMeta should not have been deployed"
    exit 1
}
ok "no deployment created"

print "PASS: vm-metadata-no-xametadata"
