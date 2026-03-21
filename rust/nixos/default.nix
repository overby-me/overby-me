{
  devShells.rust-nixos = pkgs: {
    packages = with pkgs; [
      just
      nix-tree
    ];
  };
  nixosConfigurations.nixos-nix = _: {
    system = "x86_64-linux";
    modules = [
      ./base.nix
    ];
  };
  nixosConfigurations.rust-nixos = _: {
    system = "x86_64-linux";
    modules = [
      ./base.nix
      ./systemd.nix
      ./bash.nix
      ./sudo.nix
      # ./coreutils.nix
    ];
  };
  checks.rust-nixos-boot = pkgs:
    import ./nixos-test.nix {inherit pkgs;};
}
