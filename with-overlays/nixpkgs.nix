(final: prev: {
  # Add unstable packages to attributeset to pkgs.pkgsUnstable
  pkgsUnstable =
    (import prev.inputs.nixpkgs-unstable {
      inherit (final.stdenv.hostPlatform) system;
      config.allowUnfree = true;
    })
    // prev.inputs.self.packages.${final.stdenv.hostPlatform.system};
})
