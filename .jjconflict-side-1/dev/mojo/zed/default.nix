{lib, ...}: {
  zedExtensions.mojo-zed = lib.cleanSource ./.;

  devShells.mojo-zed = pkgs: {
    packages = with pkgs; [
      (rust-bin.stable.latest.default.override {
        targets = ["wasm32-wasip2"];
      })
    ];
  };
}
