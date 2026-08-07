#!/usr/bin/env nu
# Where a Word document's pages END, worked out here rather than asked for.
#
# The app has to model Word's page breaking (see `docs/word-pagination.md`), and
# a model needs something to be measured against. Word itself is the truth, and
# nobody here runs Word, so this converts the document with LibreOffice and
# reports what came out: how many pages, and what each one begins with.
#
#   scripts/word-pages.nu some.docx
#   scripts/word-pages.nu some.docx --pdf out.pdf     # keep the rendering
#
# **The fonts are the whole trick.** LibreOffice was written off as a second
# opinion once because it made a document ten pages where Word makes eight, but
# that was its font substitution, not its layout. Given the same
# metric-compatible faces the app measures in (Carlito for Calibri, Caladea for
# Cambria, Liberation Sans/Serif for Arial and Times), it agrees with Word: the
# document that read ten came out nine, which is what the reader's own export of
# it says, and the break this app was getting wrong landed exactly where the
# reader reported Word puts it.
#
# So the substitutions below are not a convenience. A run without them measures
# a different document. Nothing is downloaded that nix does not already cache;
# the first run builds LibreOffice's closure and later ones are instant.

# A store path for a flake attribute, built if it is not there yet.
def store-path [attr: string] {
    let out = (^nix build --no-link --print-out-paths $attr | complete)
    if $out.exit_code != 0 {
        error make { msg: $"could not build ($attr): ($out.stderr | str trim)" }
    }
    $out.stdout | lines | first | str trim
}

# The faces cut to the metrics Word's own faces have, and a fontconfig that
# serves ONLY those: whatever the machine has installed must not get a vote.
def metric-fonts [work: path] {
    let dir = ($work | path join "fonts")
    mkdir $dir
    for attr in ["nixpkgs#carlito" "nixpkgs#caladea" "nixpkgs#liberation_ttf"] {
        let src = (store-path $attr)
        for f in (glob $"($src)/share/fonts/**/*.ttf") { cp $f $dir }
    }
    let conf = ($work | path join "fonts.conf")
    $"<?xml version='1.0'?>
<!DOCTYPE fontconfig SYSTEM 'fonts.dtd'>
<fontconfig>
  <dir>($dir)</dir>
  <cachedir>($work | path join 'fontcache')</cachedir>
  <alias binding='same'><family>Calibri</family><accept><family>Carlito</family></accept></alias>
  <alias binding='same'><family>Cambria</family><accept><family>Caladea</family></accept></alias>
  <alias binding='same'><family>Arial</family><accept><family>Liberation Sans</family></accept></alias>
  <alias binding='same'><family>Helvetica</family><accept><family>Liberation Sans</family></accept></alias>
  <alias binding='same'><family>Times New Roman</family><accept><family>Liberation Serif</family></accept></alias>
  <!-- Word 2024's default has no metric-compatible cut anywhere. Carlito stands
       in so the page is not laid out in something wilder still, and a document
       in it is worth less as truth than the rest. -->
  <alias binding='same'><family>Aptos</family><accept><family>Carlito</family></accept></alias>
</fontconfig>
" | save -f $conf
    $conf
}

def main [
    file: path,          # the document to paginate
    --pdf: path,         # keep the rendering here rather than throwing it away
] {
    if not ($file | path exists) {
        error make { msg: $"no such file: ($file)" }
    }
    let work = (mktemp -d)
    mkdir ($work | path join "fontcache")
    let conf = (metric-fonts $work)
    let office = (store-path "nixpkgs#libreoffice")
    let poppler = (store-path "nixpkgs#poppler-utils")

    # A copy, because the converter writes its output beside its input and this
    # should not litter the document's own directory.
    let name = ($file | path basename)
    cp $file ($work | path join $name)
    (^env $"HOME=($work)" $"FONTCONFIG_FILE=($conf)"
        $"($office)/bin/soffice" --headless --convert-to pdf
        --outdir $work ($work | path join $name) | complete | ignore)
    let rendered = ($work | path join $"($name | path parse | get stem).pdf")
    if not ($rendered | path exists) {
        rm -rf $work
        error make { msg: "the converter produced nothing" }
    }

    let text = (^$"($poppler)/bin/pdftotext" -layout $rendered - | complete).stdout
    # A trailing blank page is the converter's, not the document's: LibreOffice
    # gives the paragraphs after the last table a page of their own more readily
    # than Word does, and an empty one tells the reader nothing.
    let pages = ($text | split row "\u{0c}" | where {|p| ($p | str trim) != "" })
    print $"($pages | length) pages"
    for p in ($pages | enumerate) {
        let first = ($p.item | lines | where {|l| ($l | str trim) != "" } | first | str trim)
        print $"  page ($p.index + 1)  ($first | str substring 0..76)"
    }
    if $pdf != null { cp $rendered $pdf; print $"rendering kept at ($pdf)" }
    rm -rf $work
}
