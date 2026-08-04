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
  # Rung 1 (docs/ROADMAP.md "Ship one increment"): one rust component under the
  # C systemd manager. Boots with C PID 1 but the rust systemd-tmpfiles.
  checks.rust-rung1-tmpfiles = pkgs:
    import ./rung1-tmpfiles-test.nix {inherit pkgs;};
  # Rung 1b: a second rust component (systemd-sysusers) under C PID 1.
  checks.rust-rung1-sysusers = pkgs:
    import ./rung1-sysusers-test.nix {inherit pkgs;};
}
