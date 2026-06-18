{
  pkgs,
  lib,
  ...
}: {
  home.packages = with pkgs.pkgsUnstable;
    [
      # General dev
      lazyjj
      glab
      granted

      # AI
      #mistral-vibe
      # AI tools/languages

      # System dev
      #lldb
      gdb
      cling # C++ repl
      evcxr # Rust repl
      llvmPackages.bintools
      binwalk
      hyperfine
      inferno # Flamegraph svg generator
      flamelens # Flamegraph cli viewer
      #darling

      # Nix dev
      nix-du
      nix-sweep
      nix-diff-rs
      devenv
      nix-prefetch-git
      nix-fast-build
      nix-init
      comma
      nurl
      nxv
    ]
    ++ lib.optionals pkgs.stdenv.isLinux [
      # Tangled (server/CI tooling, not needed on Darwin workstations).
      tangled-cli
      tangled-spindle-nix-engine # spindle-run

      # ptrace-based syscall tracers; Linux-only (not available on Darwin).
      lurk
    ];
}
