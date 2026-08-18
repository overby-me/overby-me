#!/usr/bin/env nu
# Spawn a real C pipewire daemon, connect with our Rust protocol-native
# client, and verify we receive a Core.Info plus at least one Registry
# Global event. This proves wire-format compatibility end-to-end.

# Set up a private XDG_RUNTIME_DIR so the daemon's socket is isolated.
$env.XDG_RUNTIME_DIR = $env.TMPDIR | path join run
mkdir $env.XDG_RUNTIME_DIR
^chmod 700 $env.XDG_RUNTIME_DIR

# Minimal daemon config: only the protocol-native module so we don't pull
# in audio backends (no /dev/snd in the sandbox).
mkdir ($env.TMPDIR | path join conf)
"context.properties = {
    core.daemon = true
    core.name   = pipewire-test
    log.level   = 0
    mem.warn-mlock = false
    mem.allow-mlock = false
    support.dbus   = false
}
context.spa-libs = {
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-client-node }
]
context.objects = []
context.exec = []
" | save -f --raw ($env.TMPDIR | path join conf pipewire.conf)

$env.PIPEWIRE_CORE = "pipewire-test"
$env.PIPEWIRE_CONFIG_DIR = $env.TMPDIR | path join conf
$env.HOME = $env.TMPDIR | path join no-home
$env.XDG_CONFIG_HOME = $env.TMPDIR | path join no-xdg

# Use the upstream C pipewire daemon, not our symlinked Rust stub.
let daemon = $env.pipewireBin | path join bin pipewire
if (do { ^test -x $daemon } | complete).exit_code != 0 {
    print $"FAIL: pipewire daemon not found at ($daemon)"
    exit 1
}

# Spawn the daemon as a background job. The job (and its child) are killed
# when this nu process exits, which stands in for the bash EXIT trap.
let daemon_log = $env.TMPDIR | path join daemon.log
let daemon_job = job spawn { ^$daemon o+e> $daemon_log }

let socket = $env.XDG_RUNTIME_DIR | path join pipewire-test

# Wait up to 5 seconds for the socket to appear.
mut appeared = false
for _ in 1..50 {
    if (($socket | path exists) and ((ls -l $socket | get 0.type) == "socket")) {
        $appeared = true
        break
    }
    sleep 100ms
}
if not $appeared {
    print "FAIL: daemon socket never appeared"
    print "daemon log:"
    print (open --raw $daemon_log)
    try { job kill $daemon_job }
    exit 1
}

# Run our client.
try { ^$env.RUST proto-probe 30 o+e> ($env.TMPDIR | path join probe.out) }
let exit_code = $env.LAST_EXIT_CODE
print $"probe exit: ($exit_code)"

# The probe output should contain at least one Core.Info (id=0 opcode=0)
# and at least one Registry.Global (id=1 opcode=0) event.
let probe_lines = open --raw ($env.TMPDIR | path join probe.out) | lines
let core_info_lines = $probe_lines | where {|l| $l =~ '^msg id=0 opcode=0 ' } | length
let registry_lines = $probe_lines | where {|l| $l =~ '^msg id=1 opcode=0 ' } | length

print "probe output (first 5 messages):"
$probe_lines | first 5 | each {|l| print $"  ($l)" }

if $core_info_lines < 1 {
    print "FAIL: no Core.Info received"
    print "daemon log:"
    print (open --raw $daemon_log)
    try { job kill $daemon_job }
    exit 1
}
if $registry_lines < 1 {
    print "FAIL: no Registry events received"
    print "daemon log:"
    print (open --raw $daemon_log)
    try { job kill $daemon_job }
    exit 1
}

print $"PASS: proto/hello-info \(($core_info_lines) Core.Info, ($registry_lines) Registry events)"
try { job kill $daemon_job }
