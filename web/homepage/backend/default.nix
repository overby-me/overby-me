{
  devShells.backend = pkgs: {
    packages = [
      pkgs.rust-bin.stable.latest.default
      pkgs.openssl
      #pkgs.scaleway-cli
    ];
  };
}
