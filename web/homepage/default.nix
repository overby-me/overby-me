{
  imports = [
    ./backend
  ];

  devShells.homepage = pkgs: {
    packages = with pkgs; [
      which
      just
      cargo
      rustc
      rust-analyzer
      dioxus-cli
      wasm-bindgen-cli
      binaryen
      lld
    ];
  };
}
