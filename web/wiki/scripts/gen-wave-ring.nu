#!/usr/bin/env nu

# The wavy ring behind the M3 Expressive loading indicator (`.spinner` in
# assets/style.css).
#
# Material 3 Expressive gave the progress indicators a wavy active track. There
# is no CSS primitive for "circle, but sinusoidal", so the shape is an SVG path
# used as a `mask-image`: the mask carries the geometry and the colour comes from
# `background`, which keeps the indicator on theme tokens in both schemes.
#
# The path is a polar sinusoid, r(t) = radius + amplitude * sin(crests * t),
# sampled as a closed polyline. Round joins hide the sampling at every size the
# app renders it (18px, 22px, 44px).
#
# It is generated rather than hand-written because the numbers are the design:
# raising `crests` or `amplitude` means regenerating, not editing 128 coordinate
# pairs by hand. Paste the printed line into the `--m3-wave-ring` custom property.
#
#   nu scripts/gen-wave-ring.nu
#   nu scripts/gen-wave-ring.nu --crests 10 --amplitude 1.8

const PI = 3.141592653589793

def main [
  --radius: float = 18.0      # of the circle the wave is drawn around
  --amplitude: float = 1.9    # how far each crest departs from it
  --crests: int = 8           # crests around the full turn
  --stroke: float = 4.4       # line thickness, in the 48-unit viewBox
  --per-crest: int = 16       # samples per crest
] {
  let center = 24.0
  let steps = $crests * $per_crest
  let points = (0..$steps | each {|i|
    let t = 2 * $PI * ($i / $steps)
    let r = $radius + $amplitude * ($crests * $t | math sin)
    let x = ($center + $r * ($t | math cos) | math round --precision 2)
    let y = ($center + $r * ($t | math sin) | math round --precision 2)
    $"($x),($y)"
  })
  # `Z` closes the loop; the first and last samples coincide, so the join is
  # exactly on a crest and never shows a seam.
  let d = $"M($points | first)L($points | skip 1 | str join 'L')Z"

  let svg = $"%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'%3E%3Cpath d='($d)' fill='none' stroke='%23000' stroke-width='($stroke)' stroke-linejoin='round'/%3E%3C/svg%3E"

  print $"/* radius ($radius), amplitude ($amplitude), ($crests) crests, stroke ($stroke). Regenerate: nu scripts/gen-wave-ring.nu */"
  print $"--m3-wave-ring: url\(\"data:image/svg+xml,($svg)\"\);"
}

