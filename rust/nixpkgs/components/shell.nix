# Shell: bash → rust-bash
#
# rust-bash is a Bash-compatible shell written in Rust for this project.
# It implements a lexer, parser, and interpreter covering core bash features:
# variables, control flow, functions, pipes, redirections, parameter expansion,
# command substitution, arithmetic, and common builtins.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "shell";
  original = pkgs.bash;
  replacement = pkgs.rust-bash;
  status = status.available;
  source = source.repo;
  phase = 1;
  description = "POSIX/Bash-compatible shell";
  notes = "Rust rewrite at rust/bash";
}
