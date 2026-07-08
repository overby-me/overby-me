{
  imports = [
    ../homepage/backend/default.nix
  ];

  devShells.homepage-dioxus = pkgs: {
    packages = with pkgs; [
      just
      cargo
      rustc
      rust-analyzer
      dioxus-cli
      wasm-bindgen-cli
      binaryen
    ];
  };
}
