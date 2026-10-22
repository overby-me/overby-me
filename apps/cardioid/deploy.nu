#!/usr/bin/env nu

# deploy.nu — upload the built bundle to statichost.eu.
#
# statichost takes a zip of the site and serves it as-is
# (https://www.statichost.eu/docs/direct-upload/). Their own uploader is a
# bash script; this is the same three steps in nushell, which is what this
# repository writes scripts in.
#
# The API key is read from STATICHOST_APIKEY and is never written down here.
# Set it for one command rather than exporting it, so it does not sit in the
# shell's history or environment:
#
#   STATICHOST_APIKEY=... just deploy
#
# Usage:
#   nu deploy.nu <site> <directory>

def log-info [...msg: string] { print -e $"(ansi blue_bold)[info](ansi reset) ($msg | str join ' ')" }
def log-ok [...msg: string] { print -e $"(ansi green_bold)[ok](ansi reset) ($msg | str join ' ')" }
def log-fail [...msg: string] { print -e $"(ansi red_bold)[fail](ansi reset) ($msg | str join ' ')" }

def main [
    site: string        # the site's name on statichost
    dir: string         # the directory to upload, usually the built public/
    --builder: string = "https://builder.statichost.eu"
] {
    let key = ($env | get -o STATICHOST_APIKEY | default "")
    if ($key | is-empty) {
        log-fail "STATICHOST_APIKEY is not set."
        print -e "Make one at https://builder.statichost.eu/account, then:"
        print -e "  STATICHOST_APIKEY=... just deploy"
        exit 2
    }

    if not ($dir | path exists) {
        log-fail $"($dir) does not exist. Run `just build` first."
        exit 2
    }
    let index = ($dir | path join "index.html")
    if not ($index | path exists) {
        log-fail $"($dir) has no index.html, so it is not a built site."
        exit 2
    }

    # The zip must hold the *contents* of the directory, not the directory
    # itself, or every path on the site gains a leading component and nothing
    # resolves. Their documentation is emphatic about this and it is the one
    # way to get a deploy silently wrong.
    let zip = (mktemp --tmpdir --suffix .zip statichost-XXXXXX)
    rm -f $zip
    let files = (ls $dir | length)
    log-info $"zipping ($files) entries from ($dir)"
    do { cd $dir; ^zip -qr $zip . }

    let size = (ls $zip | get 0.size)
    log-info $"uploading ($size) to ($builder)/($site)/drop"

    let out = (
        ^curl --silent --show-error --fail-with-body
            -X POST $"($builder)/($site)/drop"
            -H $"Authorization: Bearer ($key)"
            -H "Content-Type: application/zip"
            -H "Accept: text/plain"
            --data-binary $"@($zip)"
        | complete
    )
    rm -f $zip

    if $out.exit_code != 0 {
        log-fail $"upload failed \(curl ($out.exit_code)\)"
        if not ($out.stdout | is-empty) { print -e $out.stdout }
        if not ($out.stderr | is-empty) { print -e $out.stderr }
        exit 1
    }
    if not ($out.stdout | is-empty) { print $out.stdout }
    log-ok $"deployed ($site)"
}
