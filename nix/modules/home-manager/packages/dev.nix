{pkgs, ...}: {
  home.packages = with pkgs.pkgsUnstable; [
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
    lurk
    tracexec
    llvmPackages.bintools
    binwalk
    hyperfine
    inferno # Flamegraph svg generator
    flamelens # Flamegraph cli viewer
    #darling

    # Tangled
    tangled-cli
    tangled-spindle-nix-engine # spindle-run

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
  ];
}
