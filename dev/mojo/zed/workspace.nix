{lib, ...}: {
  zedExtensions.default = lib.cleanSource ./.;

  devShell = pkgs: {
    packages = with pkgs; [
      (rust-bin.stable.latest.default.override {
        targets = ["wasm32-wasip2"];
      })
    ];
  };
}
