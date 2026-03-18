# gnused → uutils-sed
#
# GNU sed is used pervasively in stdenv for text substitution — configure
# scripts, setup-hooks, substituteInPlace, and many build systems depend
# on it.
#
# uutils-sed (https://github.com/uutils/sed) is a Rust rewrite that
# implements all POSIX commands plus common GNU extensions (-i, -E,
# address ranges, branch/label commands). Packaged in nix/pkgs.
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
  replacement = pkgs.uutils-sed;
  status = status.available;
  source = source.nixpkgs;
  phase = 2;
  description = "Stream editor for filtering and transforming text";
  notes = "Using uutils-sed — Rust rewrite from uutils project with GNU extension support";
}
