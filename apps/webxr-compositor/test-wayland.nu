#!/usr/bin/env nu

# test-wayland.nu: start the host and ask wayland-info what it advertises.
#
# The browser test proves the HTTP/WebSocket side; this proves the other
# half: a real libwayland client can connect to the socket and enumerate
# every global the compositor claims to provide.
#
# Exit codes: 0 all expected globals present · 1 missing or broken · 2 setup
# failed.

const HOST_PORT = 8373
const SOCKET = "wayland-webxr-test"
const HOST_BIN = "host/target/debug/webxr-compositor"
const EXPECTED = [
    wl_compositor
    wl_subcompositor
    wl_shm
    wl_seat
    wl_output
    zxdg_output_manager_v1
    xdg_wm_base
    wl_data_device_manager
]

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset) ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[pass](ansi reset) ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset) ($msg | str join ' ')" }

def main []: nothing -> nothing {
    let host_bin = ($env.FILE_PWD | path join $HOST_BIN)
    if not ($host_bin | path exists) {
        log-fail $"no host at ($host_bin); run `cargo build --manifest-path host/Cargo.toml` first"
        exit 2
    }

    # Reap leftovers of an aborted earlier run; a survivor keeps the port and
    # the fresh host dies at bind.
    ^pkill -f $host_bin | complete | ignore

    let socket_path = ($env.XDG_RUNTIME_DIR | path join $SOCKET)
    if ($socket_path | path exists) { rm $socket_path }

    let host = (job spawn {
        with-env {
            WEBXR_COMPOSITOR_LISTEN: $"127.0.0.1:($HOST_PORT)"
            WEBXR_COMPOSITOR_WAYLAND_DISPLAY: $SOCKET
        } { ^$host_bin | complete | ignore }
    })

    mut up = false
    for _ in 0..40 {
        if ($socket_path | path exists) { $up = true; break }
        sleep 250ms
    }
    if not $up {
        try { job kill $host }
        ^pkill -f $host_bin | complete | ignore
        log-fail "the wayland socket never appeared"
        exit 2
    }
    log-info $"socket at ($socket_path)"

    let info = (with-env { WAYLAND_DISPLAY: $SOCKET } { ^wayland-info | complete })
    try { job kill $host }
    ^pkill -f $host_bin | complete | ignore

    if $info.exit_code != 0 {
        log-fail $"wayland-info failed:\n($info.stdout)\n($info.stderr)"
        exit 1
    }

    let advertised = (
        $info.stdout
        | lines
        | where {|l| $l =~ "interface:" }
        | each {|l| $l | parse --regex `interface: '(?<name>[a-z0-9_]+)'` | get 0?.name? }
        | compact
    )
    print ($advertised | str join "\n")

    mut missing = []
    for g in $EXPECTED {
        if not ($g in $advertised) { $missing = ($missing | append $g) }
    }

    if ($missing | is-empty) {
        log-ok $"all (($EXPECTED | length)) expected globals are advertised"
    } else {
        for g in $missing { log-fail $"missing global ($g)" }
        exit 1
    }
}
