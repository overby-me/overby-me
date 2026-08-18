{lib, ...}: {
  zedExtensions.nickel-zed = lib.cleanSource ./.;

  devShells.nickel-zed = pkgs: {
    packages = with pkgs; [
      (rust-bin.stable.latest.default.override {
        targets = ["wasm32-wasip2"];
      })
    ];
  };
}
