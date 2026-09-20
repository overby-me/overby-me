# The alpha's spaces PDS in a container, beside the stand-in directory
# (`crates/fake-plc`): what `test-spaces.nu` and `rehearse-spaces.nu` run
# against. atproto spaces are an alpha, so nothing real belongs on it.

export const PLC_PORT = 2582
export const PDS_PORT = 2583
const IMAGE = "ghcr.io/bluesky-social/atproto:pds-spaces-alpha"
const NAME = "wiki-spaces-pds"

def kill-port [port: int] {
    try { ^fuser -k $"($port)/tcp" | complete | ignore } catch { }
}

export def stop-spaces-pds [] {
    do -i { ^docker rm -f $NAME } | complete | ignore
    kill-port $PLC_PORT
}

# Start both, and answer the PDS's version. `blob_limit` is the largest file it
# takes, in bytes. Fails with a reason, having stopped what it started.
export def start-spaces-pds [crates: path, blob_limit: int = 5242880]: nothing -> string {
    for cmd in [docker cargo curl fuser] {
        if (which $cmd | where type == "external" | is-empty) { error make {msg: $"required command not found: ($cmd)"} }
    }
    stop-spaces-pds
    let built = (do -i { ^cargo build --quiet -p fake-plc --manifest-path ($crates | path join "Cargo.toml") } | complete)
    if $built.exit_code != 0 { error make {msg: $"fake-plc did not build: ($built.stderr)"} }
    let plc = ($crates | path join "target" "debug" "fake-plc")
    ^bash -c $'($plc) --port ($PLC_PORT) > /dev/null 2>&1 < /dev/null &'

    # On the host's network, so that it reaches the directory and whatever
    # listens on loopback for it. Nothing it writes outlives the container.
    let secret = (random chars --length 32)
    let rotation = (random binary 32 | encode hex | str lowercase)
    let settings = [
        "PDS_HOSTNAME=localhost" $"PDS_PORT=($PDS_PORT)" "PDS_DEV_MODE=true" "NODE_ENV=development"
        "PDS_DATA_DIRECTORY=/pds" "PDS_BLOBSTORE_DISK_LOCATION=/pds/blocks"
        $"PDS_DID_PLC_URL=http://127.0.0.1:($PLC_PORT)"
        $"PDS_JWT_SECRET=($secret)" $"PDS_ADMIN_PASSWORD=(random chars --length 24)"
        $"PDS_PLC_ROTATION_KEY_K256_PRIVATE_KEY_HEX=($rotation)"
        "PDS_SERVICE_HANDLE_DOMAINS=.test" "PDS_INVITE_REQUIRED=false"
        $"PDS_BLOB_UPLOAD_LIMIT=($blob_limit)"
    ]
    let flags = ($settings | each { |s| ["-e" $s] } | flatten)
    let started = (do -i { ^docker run -d --name $NAME --network host --tmpfs /pds ...$flags $IMAGE } | complete)
    if $started.exit_code != 0 { stop-spaces-pds; error make {msg: $"docker could not start the PDS: ($started.stderr)"} }

    mut up = false
    mut waited = 0
    while $waited < 60 and not $up {
        $up = ((do -i { ^curl -s -m 2 -o /dev/null -w "%{http_code}" $"http://localhost:($PDS_PORT)/xrpc/_health" } | complete).stdout | str trim) == "200"
        if not $up { sleep 1sec }
        $waited = $waited + 1
    }
    if not $up {
        let said = (do -i { ^docker logs --tail 20 $NAME } | complete).stderr
        stop-spaces-pds
        error make {msg: $"the PDS did not come up: ($said)"}
    }
    try { http get $"http://localhost:($PDS_PORT)/xrpc/_health" | get version } catch { "unknown" }
}
