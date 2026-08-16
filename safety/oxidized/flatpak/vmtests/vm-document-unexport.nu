#!/usr/bin/env nu

source ./libtest-nix.nu

mkdir ($env.WORK | path join docs)
"u\n" | save -f --raw ($env.WORK | path join docs u.txt)
let re = (do { ^$env.FLATPAK document-export ($env.WORK | path join docs u.txt) } | complete)
let doc_id = (
    ($re.stdout + $re.stderr) | lines
    | where {|l| $l | str starts-with "Exported as: " }
    | each {|l| $l | str replace "Exported as: " "" }
    | get 0? | default ""
)
ok $"exported as ($doc_id)"

^$env.FLATPAK document-unexport $doc_id
ok "unexport succeeded"

# Subsequent info should fail.
let ri = (do { ^$env.FLATPAK document-info $doc_id } | complete)
let out = $ri.stdout + $ri.stderr
if $ri.exit_code == 0 {
    print "FAIL: document-info should fail after unexport"
    print $"Got: ($out)"
    exit 1
}
ok "document-info errors after unexport"

# Listing should not include this doc id.
let rl = (do { ^$env.FLATPAK documents } | complete)
let list = $rl.stdout + $rl.stderr
if ($list | str contains $doc_id) {
    print $"FAIL: ($doc_id) still listed after unexport"
    exit 1
}
ok "doc id removed from listing"

print "PASS: vm-document-unexport"
