#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# Create a symlink at the persist target pointing to /etc
mkdir ($env.HOME | path join .var app org.test.Hello)
^ln -sf /etc ($env.HOME | path join .var app org.test.Hello .persist-escape)

# Override to use --persist with the symlinked path
^$env.FLATPAK --user override --persist=.persist-escape org.test.Hello

# Attempt to read /etc/passwd through the symlink from inside the sandbox
let r = (do { run_sh org.test.Hello 'cat $HOME/.persist-escape/passwd 2>&1 || echo BLOCKED' } | complete)
let output = $r.stdout + $r.stderr

# The sandbox must not expose /etc/passwd through the symlink (CVE-2024-42472)
assert_not_streq $output "root:"

if ($output | lines | any {|l| $l =~ 'BLOCKED|No such file' }) {
    print "Symlink persist target correctly rejected"
} else {
    print $"Unexpected output: ($output)"
    exit 1
}

ok "persist symlink escape blocked"
print "PASS: vm-persist-symlink-escape"
