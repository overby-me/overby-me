# gnused → rust-sed
#
# GNU sed is used pervasively in stdenv for text substitution — configure
# scripts, setup-hooks, substituteInPlace, and many build systems depend
# on it.
#
# rust-sed is our own implementation that supports all common delimiters
# (including &), BRE/ERE, in-place editing, branch/label commands, hold
# space, and the full set of commands needed by autoconf's config.status.
# uutils-sed was previously used but has a critical bug where & cannot
# be used as a substitute delimiter.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "gnused";
  original = pkgs.gnused;
  replacement = pkgs.rust-sed;
  status = status.available;
  source = source.repo;
  phase = 2;
  description = "Stream editor for filtering and transforming text";
  notes = "Using rust-sed from rust/sed — GNU-compatible with all delimiter support";
}
