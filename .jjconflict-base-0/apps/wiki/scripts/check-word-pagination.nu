#!/usr/bin/env nu
# Does a Word document paginate here the way Word paginates it?
#
# The app works out where a Word file's pages end by laying it out off-screen at
# the size the document says its pages are (see `components::docx`). That is a
# model of Word's line breaking, and a model is worth what it measures: this
# opens real documents in the deployed build and checks BOTH things that can be
# wrong -- how many pages it found, and where each one begins.
#
# The second half is not optional. A count can come out right while every break
# sits in the wrong place: a three-page document once read 3 while its second
# page began thirty paragraphs past where Word begins it, and a run that only
# counted pages called that a pass.
#
#   WIKI_EMAIL=... WIKI_PASSWORD=... scripts/check-word-pagination.nu
#
# The account has to be able to open all five documents: three hang under HB5
# and two under Landsmøde 2026, which are separate contexts. A document it
# cannot see is reported as skipped rather than failed.
#
# Nothing is written to the wiki. Needs `deno` and `chromium`; chromium is not
# in the devshell, so set CHROMIUM or have one on PATH.

const NHOST = "https://pgvhpsenoifywhuxnybq.auth.eu-central-1.nhost.run/v1"
const WIKI = "https://radikal.wiki/radikal_ungdom"

# What WORD itself says, read out of each file's own `lastRenderedPageBreak`
# hints -- the record Word leaves of where it last drew a page break -- plus the
# first words of each page after the first.
#
# Every row below is now corroborated by a second, independent reading:
# `scripts/word-pages.nu` renders the same file with LibreOffice in
# metric-compatible faces and reports where its pages fall. It agrees with all
# five, including the two whose expectations did NOT come from the hints, and
# it is how a sixth would be added without asking anyone to export anything.
#
# Where Word breaks INSIDE a table row, the words are the ROW's rather than the
# paragraph's: a rendering that cannot split a row across a page can only mark
# the row, and a reader jumping to that page lands at the top of the row whose
# text the page begins in.
#
# `starts` is read from page 2 onwards, in order. `last` is the LAST page's, for
# a document whose middle breaks cannot all be stated but whose ending can: it
# is checked against the final mark whatever its number, so it guards the end of
# a document without claiming to know everything in between.
const TRUTH = [
    [path, pages, starts, last];

    ["hb5/bilag/forretningsudvalgets_arbejdsprogram_202122", 3,
        ["Et godt kommunal- og regionsrådsvalg", "Der ifbm. lokalforeningsgtræf"], ""]

    # Six tables and little else. NINE pages, from the document exported to PDF
    # by the office suite the reader uses -- not from the file, whose own record
    # says eight and is stale: read page by page that record needs one page to
    # hold 1761px of this document where its others hold about 880, twice what
    # its neighbours hold, which the text it now contains cannot do. The file
    # was edited after it was last drawn.
    #
    # Two of the middle breaks are checked, and only two: FOUR of this
    # document's nine pages begin in the middle of a table cell or a sentence,
    # which a rendering that marks whole rows cannot express. The LAST page is
    # checked as well, reported by the reader who opened it in Word: it begins
    # at the closing "Konklusion" and its paragraph. That one was wrong until
    # the renderer kept the empty line a paragraph ENDING in a line break makes
    # -- ten of this document's paragraphs end that way, and the ten lines came
    # to a fifth of a page.
    ["hb5/bilag/evaluering_af_fu_og_posk´s_arbejdsprogram", 9,
        ["Fokus på trivslen lokalt", "At vi har et bedre skolevalg"], "Konklusion"]

    # This one records no hints at all: its count came from reading it, its
    # break from someone opening it in Word, and both are confirmed by the
    # converter. It broke a heading late until the renderer learned about
    # AUTOMATIC paragraph spacing, which every one of its paragraphs asks for
    # (see `docs/word-pagination.md`).
    ["hb5/bilag/posk_arbejdsprogram_21-22", 3,
        ["Radikal Ungdom skal være et engagerende politisk fællesskab"], ""]

    # The assembly's own two, which is what this was for.
    ["landsmøde_2026/bilag/beretninger/sekretariatets_beretning_2026", 1, [], ""]
    ["landsmøde_2026/bilag/strategi_bilag/strategi_2030", 2,
        ["Radikal Ungdom skal skabe politisk forandring."], ""]
]

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
    --keep,     # leave the working files behind for inspection
    --quiet,    # only say what disagrees
] {
    let here = ($env.FILE_PWD | path join "word-pagination-check")
    let browser = (chromium-at)
    let work = (mktemp -d)
    let email = ($env.WIKI_EMAIL? | default "")
    let password = ($env.WIKI_PASSWORD? | default "")
    if ($email | is-empty) or ($password | is-empty) {
        error make { msg: "set WIKI_EMAIL and WIKI_PASSWORD to an account that can open these documents" }
    }
    $env.CHROMIUM = $browser

    mut wrong = 0
    for doc in $TRUTH {
        # A fresh sign-in per document: nhost rotates the refresh token when it
        # is used, so a saved session works exactly once.
        let session = ($work | path join "session.json")
        (http post --content-type application/json $"($NHOST)/signin/email-password"
            { email: $email, password: $password } | to json | save -f $session)
        let out = ($work | path join "seen")
        rm -rf $"($out)-profile"
        let url = $"($WIKI)/($doc.path | split row '/' | each {|p| $p | url encode } | str join '/')"
        (^deno run -A ($here | path join "drive.ts") $session $url $out
            ($here | path join "probe.js") | complete | ignore)
        let seen = (open $"($out).json")
        let name = ($doc.path | path basename | str substring 0..40)
        if not ($seen.opened | default false) {
            print $"  skip  ($name)  this account cannot open it"
            continue
        }
        let said = ($seen.control | default "" | parse --regex '(?<at>\d+) / (?<of>\d+)')
        # No control at all is a ONE-page document: the app offers no page
        # control for a document with only one page, which is the right answer.
        let pages = (if ($said | is-empty) { 1 } else { $said.0.of | into int })
        let starts = ($seen.startsWith | default [])

        mut faults = []
        if $pages != $doc.pages { $faults = ($faults | append $"($pages) pages, not ($doc.pages)") }
        for i in 0..<($doc.starts | length) {
            let want = ($doc.starts | get $i)
            let got = ($starts | get -o $i | default "")
            if not ($got | str starts-with $want) {
                $faults = ($faults | append $"page (($i) + 2) begins ($got | str substring 0..72)…\n            not ($want)…")
            }
        }
        # And the last page, where the document states one. Checked against the
        # final mark rather than a numbered position, so it holds whatever the
        # count turns out to be.
        if not ($doc.last | is-empty) {
            let got = ($starts | last | default "")
            if not ($got | str starts-with $doc.last) {
                $faults = ($faults | append $"the last page begins ($got | str substring 0..72)…\n            not ($doc.last)…")
            }
        }
        if ($faults | is-empty) {
            # Said apart, because they are not the same claim. A file that
            # records no breaks of its own can only have its COUNT checked, and
            # calling that "every break where Word puts it" is how a document
            # whose break had moved went on being reported as fine.
            let how = (if ($doc.starts | is-empty) {
                "pages; the file records no breaks of its own, so only the count is checked"
            } else {
                "pages, every break where Word puts it"
            })
            print $"  ok    ($name)  ($pages) ($how)"
        } else {
            $wrong = $wrong + 1
            print $"  WRONG ($name)"
            for f in $faults { print $"          ($f)" }
        }
        # What the app said it did, which distinguishes a measurement that ran
        # long from one that never ran at all.
        let said_so = (open $"($out).log" | lines | where {|l| $l | str contains "word pagination" })
        if (not $quiet) and (not ($said_so | is-empty)) {
            print $"        ($said_so | last | str replace --regex '^.*word pagination' 'word pagination')"
        }
    }
    if $keep { print $"working files in ($work)" } else { rm -rf $work }
    if $wrong == 0 {
        print "every document paginates as Word paginates it."
    } else {
        print $"($wrong) of ($TRUTH | length) documents disagree."
        exit 1
    }
}
