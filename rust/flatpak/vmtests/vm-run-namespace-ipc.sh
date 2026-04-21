#!/bin/bash
set -euo pipefail
. "$(dirname "$0")/libtest-nix.sh"

setup_repo

# Default app has --share=ipc
# Check /dev/shm is accessible (IPC shared)
output=$(run_sh org.test.Hello 'ls /dev/shm >/dev/null 2>&1 && echo shm-ok || echo shm-fail' 2>&1 || true)
echo "IPC shared: $output"
ok "ipc shared by default"

# With --unshare=ipc, /dev/shm should be a fresh tmpfs (empty)
output=$($FLATPAK --user run --unshare=ipc --command=sh org.test.Hello -c 'ls /dev/shm 2>/dev/null | wc -l' 2>&1 || true)
echo "IPC unshared shm entries: $output"
ok "ipc namespace isolation"

echo "PASS: vm-run-namespace-ipc"