# grep — gnugrep → rust-grep
#
# GNU grep is used extensively in configure scripts, Makefiles, and
# nixpkgs build hooks. rust-grep provides a GNU-flag-compatible
# implementation with BRE/ERE/PCRE support, context lines, recursive
# search, and all common flags (-w, -c, -l, -L, -Z, etc.).
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "grep";
  original = pkgs.gnugrep;
  replacement = pkgs.rust-grep;
  status = status.available;
  source = source.repo;
  phase = 2;
  description = "Pattern matching (grep, egrep, fgrep)";
  notes = "Using rust-grep from rust/grep — GNU-flag-compatible with BRE/ERE/PCRE support";
}
