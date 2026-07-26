#!/usr/bin/env nu
# Design-system lint (design system #1). Three gates:
#
#   A. Spacing / type in the stylesheet must flow through the tokens
#      (--md-sys-spacing-* and the typescale size tokens), so a global density
#      or type-scale change is one edit rather than a find-and-replace.
#   B. The same rule for the COMPONENTS. A raw px in an rsx! `style:` attribute
#      escapes the scale exactly the way a raw px in the stylesheet does, and
#      gate A never saw it. Reach for a class or a token instead; an inline
#      style should carry only values that are genuinely per-instance.
#   C. Every custom property a rule references must be defined somewhere.
#      `var(--md-sys-motion-duration-medium4, 400ms)` reads as token-driven but
#      is a literal wearing a token's name: the fallback silently takes over and
#      the value stops tracking the scale.
#
# A and B are RATCHETS: the counts may only ever go DOWN. When you migrate a
# literal to a token, lower the matching BASELINE; never raise it.
#
# Wire into CI / pre-commit:  nu web/wiki/scripts/check-css-spacing.nu

# Current sanctioned ceilings. Lower these as literals are migrated to tokens.
const CSS_BASELINE = 0
const RSX_BASELINE = 0

# Custom properties that are deliberately undefined: a local API a rule sets on
# itself and reads back through `var(--name, <default>)`.
const LOCAL_PROPS = ["--state-color"]

let root = ($env.FILE_PWD | path dirname)
let assets = ($root | path join "assets")
let src = ($root | path join "src")
let spacing_props = '(padding|margin|margin-top|margin-bottom|margin-left|margin-right|gap|font-size)'
mut failed = false

# ── Gate A: raw px in the stylesheet ────────────────────────────────────────
let css_count = (
    open --raw ($assets | path join "style.css")
    | ^rg --only-matching $'($spacing_props)[^;:]*:[^;]*[0-9]+px'
    | lines
    | where {|l| $l != "" }
    | length
)
print $"A. raw-px spacing/font-size in style.css: ($css_count) \(ceiling ($CSS_BASELINE)\)"
if $css_count > $CSS_BASELINE {
    print ""
    print $"ERROR: raw-px spacing/font-size in style.css rose above ($CSS_BASELINE)."
    print "Use the tokens instead of literal px:"
    print "  padding/margin/gap -> var\(--md-sys-spacing-N\)   \(N: 1=4px 2=8px 3=12px 4=16px 5=20px 6=24px\)"
    print "  font-size          -> the typescale size tokens in m3-tokens.css"
    $failed = true
} else if $css_count < $CSS_BASELINE {
    print $"  Nice — lower CSS_BASELINE to ($css_count) in this script."
}

# ── Gate B: raw px in the components' inline styles ─────────────────────────
# Only `style:` attributes, so a px in a comment or in an unrelated Rust string
# is not mistaken for a design-system escape.
let rsx_hits = (
    do -i {
        ^rg --line-number --only-matching --no-heading $'style: "[^"]*($spacing_props)[^;:"]*:[^;"]*[0-9]+px' -g '*.rs' $src
    }
    | lines
    | where {|l| $l != "" }
)
print $"B. raw-px spacing/font-size in rsx! style: attributes: ($rsx_hits | length) \(ceiling ($RSX_BASELINE)\)"
if ($rsx_hits | length) > $RSX_BASELINE {
    print ""
    print $"ERROR: raw-px spacing/font-size in components rose above ($RSX_BASELINE):"
    $rsx_hits | each {|l| print $"  ($l)" }
    print "Prefer a class \(.stack / .mt-1 / .list-subheader / a component class\);"
    print "when the value really is per-instance, drive it through a token:"
    print "  style: \"padding-left: calc\(var\(--md-sys-spacing-3\) * {depth}\);\""
    $failed = true
} else if ($rsx_hits | length) < $RSX_BASELINE {
    print $"  Nice — lower RSX_BASELINE to ($rsx_hits | length) in this script."
}

# ── Gate C: every referenced custom property is defined ─────────────────────
let all_css = (
    ls $assets
    | where name =~ '\.css$'
    | get name
    | each {|f| open --raw $f }
    | str join "\n"
)
let defined = (
    $all_css
    | ^rg --only-matching '^\s*(--[A-Za-z0-9_-]+)\s*:' -r '$1'
    | lines
    | each {|s| $s | str trim }
    | uniq
)
let referenced = (
    (($all_css | ^rg --only-matching 'var\(\s*(--[A-Za-z0-9_-]+)' -r '$1' | lines)
     ++ (do -i {
            ^rg --no-filename --only-matching 'var\(\s*(--[A-Za-z0-9_-]+)' -r '$1' -g '*.rs' $src
        } | lines))
    | each {|s| $s | str trim }
    | where {|s| $s != "" }
    | uniq
    | sort
)
let undefined = ($referenced | where {|r| $r not-in $defined and $r not-in $LOCAL_PROPS })
print $"C. custom properties referenced but never defined: ($undefined | length)"
if ($undefined | length) > 0 {
    print ""
    print "ERROR: these read as tokens but resolve to their inline fallback:"
    $undefined | each {|u| print $"  ($u)" }
    print "Define the token, or point the reference at the one that already exists."
    $failed = true
}

if $failed { exit 1 }
