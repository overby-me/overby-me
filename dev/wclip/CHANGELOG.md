# Changelog

All notable changes to `wclip`.

## Unreleased

### Added

- Initial release: an `xclip`-style clipboard tool for Wayland.
- Pure-Rust Wayland wire-protocol client over the compositor socket (no
  `libwayland`); only `libc` is used, for `SCM_RIGHTS` fd passing, `pipe2`,
  and `fork`.
- Clipboard via the `ext-data-control-v1` protocol (preferred) with a fallback
  to `wlr-data-control-unstable-v1`; both detected at runtime.
- Copy (`-i`) and paste (`-o`) modes for the `primary` and `clipboard`
  selections (`-selection`, with `xclip`-style abbreviations).
- MIME-type selection with `-target`, including the `TARGETS` pseudo-target to
  list the available types when pasting.
- Copy daemonizes (fork + `setsid`) to keep serving the selection in the
  background; `-foreground` keeps it attached.
- `-loops`, `-rmlastnl`, `-filter`, `-display`, `-help`, and `-version`
  options, matching `xclip` semantics.
- Roundtrip tests against an in-process mock compositor covering both the
  copy-serve and paste-receive paths, including file-descriptor passing.
