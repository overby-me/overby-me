# GNU Make → rust-make
#
# rust-make is a GNU Make-compatible build system driver written in Rust.
# It implements Makefile parsing, variable expansion (recursive/simple/append/
# conditional/shell), explicit and pattern rules, automatic variables,
# built-in functions (subst, patsubst, filter, wildcard, shell, foreach,
# call, eval, etc.), conditional directives, include, and phony targets.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "make";
  original = pkgs.gnumake;
  replacement = pkgs.rust-make;
  status = status.available;
  source = source.repo;
  phase = 4;
  description = "Build system driver (GNU Make)";
  notes = "Rust rewrite at rust/make";
}
