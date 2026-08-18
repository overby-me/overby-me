#!/usr/bin/env nu

# Cargo release-profile policy.
#
# Manifest policy is not code, so neither clippy nor ast-grep reaches it:
# clippy lints Rust, and TOML is not one of ast-grep's built-in languages.
#
# Two checks, deliberately at different strengths.
#
# `panic = "unwind"` in a release profile FAILS. It is usually left over from
# debugging; abort gives a smaller binary and removes the unwinding path. A
# crate that genuinely needs unwinding - one calling catch_unwind, or a cdylib
# whose host unwinds through it - should say so in a comment on the line, which
# this accepts as the reason.
#
# A missing `overflow-checks` only REPORTS. Fifteen of this tree's
# .deslop.toml files already turn that rule off with a reason, and those
# reasons hold: a clipboard relay doing no untrusted arithmetic gains nothing
# from it. Where it matters is code that computes on input from elsewhere - the
# video decoders, the parsers - and that is a per-project judgement rather than
# something to enforce across the tree.

def manifests [] {
  glob **/Cargo.toml --exclude [
    **/.git/** **/target/** **/vendor/** **/.jj/**
  ]
}

# The lines of a manifest's [profile.release] section, if it has one.
def release-section [path: path] {
  let lines = (open --raw $path | lines)
  let start = ($lines | enumerate | where item =~ '^\[profile\.release\]' | get index? | first)
  if $start == null { return [] }
  let rest = ($lines | skip ($start + 1))
  let stop = ($rest | enumerate | where item =~ '^\[' | get index? | first)
  if $stop == null { $rest } else { $rest | first $stop }
}

def main [] {
  mut failures = []
  mut advisories = []

  for path in (manifests) {
    let section = (release-section $path)
    if ($section | is-empty) { continue }

    # A comment on the line is how a crate records why it needs unwinding.
    let unwind = ($section | where {|l| ($l =~ 'panic\s*=\s*"unwind"') and (not ($l =~ '#')) })
    if (not ($unwind | is-empty)) {
      $failures = ($failures | append $path)
    }

    if ($section | where {|l| $l =~ '^\s*overflow-checks' } | is-empty) {
      $advisories = ($advisories | append $path)
    }
  }

  if (not ($advisories | is-empty)) {
    print $"($advisories | length) release profiles do not set overflow-checks:"
    for p in $advisories { print $"  ($p)" }
    print "  Reported, not enforced. Worth setting where arithmetic runs on"
    print "  input from elsewhere; the .deslop.toml files record where it was"
    print "  already considered and declined."
    print ""
  }

  if (not ($failures | is-empty)) {
    print $"($failures | length) release profiles pin panic = \"unwind\":"
    for p in $failures { print $"  ($p)" }
    print "  Use abort, or put the reason in a comment on the same line."
    exit 1
  }

  print "cargo profiles: no release profile pins panic = \"unwind\""
}
