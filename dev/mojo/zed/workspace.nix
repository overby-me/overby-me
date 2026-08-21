{lib, ...}: {
  zedExtensions.default = lib.cleanSource ./.;

  devShells.default = pkgs: {
    packages = with pkgs; [
      (rust-bin.stable.latest.default.override {
        targets = ["wasm32-wasip2"];
      })
    ];
  };
}
