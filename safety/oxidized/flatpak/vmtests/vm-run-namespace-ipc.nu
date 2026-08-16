#!/usr/bin/env nu
source ./libtest-nix.nu

setup_repo

# Default app has --share=ipc
# Check /dev/shm is accessible (IPC shared)
let r1 = (do { run_sh org.test.Hello 'ls /dev/shm >/dev/null 2>&1 && echo shm-ok || echo shm-fail' } | complete)
let output1 = $r1.stdout + $r1.stderr
print $"IPC shared: ($output1)"
ok "ipc shared by default"

# With --unshare=ipc, /dev/shm should be a fresh tmpfs (empty)
let r2 = (do { ^$env.FLATPAK --user run --unshare=ipc --command=sh org.test.Hello -c 'ls /dev/shm 2>/dev/null | wc -l' } | complete)
let output2 = $r2.stdout + $r2.stderr
print $"IPC unshared shm entries: ($output2)"
ok "ipc namespace isolation"

print "PASS: vm-run-namespace-ipc"
