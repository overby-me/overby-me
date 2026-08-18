#!/usr/bin/env nu

source ./libtest-nix.nu

mkdir ($env.WORK | path join docs)
"info-test\n" | save -f --raw ($env.WORK | path join docs info.txt)
let abs_path = $env.WORK | path join docs info.txt

let re = (do { ^$env.FLATPAK document-export $abs_path } | complete)
let doc_id = (
    ($re.stdout + $re.stderr) | lines
    | where {|l| $l | str starts-with "Exported as: " }
    | each {|l| $l | str replace "Exported as: " "" }
    | get 0? | default ""
)
ok $"exported as ($doc_id)"

let ri = (do { ^$env.FLATPAK document-info $doc_id } | complete)
let info = $ri.stdout + $ri.stderr
print $"info: ($info)"
if not ($info | str contains $"ID: ($doc_id)") {
    print "FAIL: missing ID line"
    exit 1
}
if not ($info | str contains $abs_path) {
    print "FAIL: missing original path in info output"
    print $"Got: ($info)"
    exit 1
}
ok "info reports original path"

print "PASS: vm-document-info"
