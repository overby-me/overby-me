#!/bin/bash
set -euo pipefail

. "${BASH_SOURCE%/*}/libtest-nix.sh"

setup_repo

# Create a symlink at the persist target pointing to /etc
mkdir -p "$HOME/.var/app/org.test.Hello"
ln -sf /etc "$HOME/.var/app/org.test.Hello/.persist-escape"

# Override to use --persist with the symlinked path
$FLATPAK --user override --persist=.persist-escape org.test.Hello

# Attempt to read /etc/passwd through the symlink from inside the sandbox
output=$(run_sh org.test.Hello "cat \$HOME/.persist-escape/passwd 2>&1 || echo BLOCKED" 2>&1)

# The sandbox must not expose /etc/passwd through the symlink
assert_not_streq "$output" "root:" "Symlink escape must be blocked (CVE-2024-42472)"

if echo "$output" | grep -qE "BLOCKED|No such file"; then
    echo "Symlink persist target correctly rejected"
else
    echo "Unexpected output: $output"
    exit 1
fi

ok "persist symlink escape blocked"
echo "PASS: vm-persist-symlink-escape"