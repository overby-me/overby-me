{
  devShells.backend = pkgs: {
    packages = [
      # The nixpkgs toolchain (not rust-bin) so this shell agrees with the
      # homepage/wiki-dioxus shells in the merged default devshell. A second
      # toolchain here shadows their rustc on PATH, and rust-bin's default
      # profile ships no wasm32-unknown-unknown std, which breaks `dx build`.
      pkgs.cargo
      pkgs.rustc
      pkgs.openssl
      #pkgs.scaleway-cli
    ];
  };
}
