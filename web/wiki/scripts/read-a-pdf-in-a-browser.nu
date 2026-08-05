#!/usr/bin/env nu
# Read a PDF in the deployed app, in a real browser, and check what came out.
#
# The reflowing PDF reader is tested from both ends already: `pdf_text` has a
# few hundred unit tests over the reconstruction, and the page arithmetic is a
# pure function with tests of its own. What neither covers is the thing a
# reader actually does -- open a document and press the buttons -- and that gap
# is where every fault in this reader has been found: a page control that
# scrolled and then panicked on the write, and one that answered with the page
# above the one it had just moved to. Both looked fine in the unit tests.
#
#   WIKI_EMAIL=... WIKI_PASSWORD=... scripts/read-a-pdf-in-a-browser.nu
#
# Nothing is written to the wiki. A file node the account can already see is
# made to LOOK like a PDF on its way to the app, and the fixture below is
# served in place of its bytes; everything else is the deployed build against
# the real backend. Needs `deno` and `chromium`, both in the devshell.

const NHOST = "https://pgvhpsenoifywhuxnybq.auth.eu-central-1.nhost.run/v1"
# A file node in an old meeting context, used only as somewhere for the reader
# to be mounted. What it actually holds never reaches the browser.
const STAND_IN = "https://radikal.wiki/radikal_ungdom/hb5/test-image"

# Chromium is not in the devshell, so it is named rather than assumed: set
# CHROMIUM, or have one on PATH.
def chromium-at [] {
    let named = ($env.CHROMIUM? | default "")
    if not ($named | is-empty) { return $named }
    let found = (which chromium)
    if ($found | is-empty) {
        error make { msg: "no chromium: set CHROMIUM to one, e.g. CHROMIUM=(nix build --print-out-paths nixpkgs#chromium)/bin/chromium" }
    }
    $found | get 0.path
}

def main [
    --url: string = $STAND_IN,   # the file page to open the reader on
    --keep,                      # leave the working files behind for inspection
] {
    let here = ($env.FILE_PWD | path join "pdf-reader-check")
    let browser = (chromium-at)
    let work = (mktemp -d)
    let fixture = ($work | path join "fixture.pdf")

    # A four-page document with a contents list that points into itself, an
    # external link, an address, a picture, italic and bold, and a bullet list.
    print "printing the fixture..."
    (^$browser --headless=new --no-sandbox --disable-gpu
        $"--print-to-pdf=($fixture)" --no-pdf-header-footer
        --virtual-time-budget=5000 $"file://($here | path join 'fixture.html')"
        | complete | ignore)
    if not ($fixture | path exists) { error make { msg: "chromium printed no fixture" } }

    let email = ($env.WIKI_EMAIL? | default "")
    let password = ($env.WIKI_PASSWORD? | default "")
    if ($email | is-empty) or ($password | is-empty) {
        error make { msg: "set WIKI_EMAIL and WIKI_PASSWORD to an account that can open a file page" }
    }
    # A fresh sign-in every run: nhost rotates the refresh token when it is
    # used, so a saved session works exactly once.
    print "signing in..."
    let session = ($work | path join "session.json")
    (http post --content-type application/json $"($NHOST)/signin/email-password"
        { email: $email, password: $password } | to json | save -f $session)

    print $"opening ($url) ..."
    let out = ($work | path join "seen")
    $env.CHROMIUM = $browser
    (^deno run -A ($here | path join "drive.ts") $session $fixture $url $out | complete | ignore)
    let seen = (open $"($out).json")

    mut bad = []
    let opened = $seen.opened
    if $opened.blocks < 10 { $bad = ($bad | append $"the reader showed ($opened.blocks) blocks; it should show the document") }
    if $opened.marks < 3 { $bad = ($bad | append $"($opened.marks) page marks, expected 3") }
    if $opened.images < 1 { $bad = ($bad | append "the picture did not arrive") }
    if $opened.italics < 1 { $bad = ($bad | append "the italic did not arrive") }
    let links = ($opened.links | compact)
    if ($links | where {|l| $l | str starts-with "#" } | is-empty) {
        $bad = ($bad | append "the contents list does not point into the document")
    }
    if ($links | where {|l| $l | str starts-with "https://" } | is-empty) {
        $bad = ($bad | append "the external link did not survive")
    }
    if ($links | where {|l| $l | str starts-with "mailto:" } | is-empty) {
        $bad = ($bad | append "the address did not become a link")
    }
    if $opened.page != "1" { $bad = ($bad | append $"opened on page ($opened.page), expected 1") }
    # One press, one page. This is the check that would have caught both faults.
    if $seen.afterForward.page != "2" {
        $bad = ($bad | append $"one press forward said page ($seen.afterForward.page), expected 2")
    }
    if $seen.afterBack.page != "1" {
        $bad = ($bad | append $"one press back said page ($seen.afterBack.page), expected 1")
    }
    if $seen.atTheEnd.page != $opened.of {
        $bad = ($bad | append $"the end of the document said page ($seen.atTheEnd.page), expected ($opened.of)")
    }
    let crashes = ($seen.console | where {|l| ($l | str contains "PANIC") or ($l | str contains "panicked") })
    if not ($crashes | is-empty) { $bad = ($bad | append $"the app panicked: ($crashes | first)") }

    if $keep { print $"working files in ($work)" } else { rm -rf $work }
    if ($bad | is-empty) {
        print "the reader opened the document, kept its links, and turned a page per press."
    } else {
        for b in $bad { print $"  ($b)" }
        error make { msg: $"($bad | length) thing\(s\) wrong with the reader" }
    }
}
