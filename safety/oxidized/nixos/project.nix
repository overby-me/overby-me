# A project module, applied to its own label by the workspace.
#
# This one contributes what nixDir contributes - a devshell, NixOS
# configurations, checks - from the project's own directory rather than from a
# directory named after each output type. The file says which output it feeds
# and the label says what the entry is called, so `oxidized-nixos` appears
# nowhere below even though every name it produces starts with it.
label: {
  devShells = label.names {
    default = pkgs: {
      packages = with pkgs; [
        just
        nix-tree
      ];
    };
  };

  nixosConfigurations =
    label.names {
      default = _: {
        system = "x86_64-linux";
        modules = [
          ./base.nix
          ./systemd.nix
          ./bash.nix
          ./sudo.nix
          # ./coreutils.nix
        ];
      };
    }
    // {
      # The stock NixOS this one is measured against, named for what it is
      # rather than for the project that keeps it: a baseline is not a
      # rung of the port.
      nixos-nix = _: {
        system = "x86_64-linux";
        modules = [
          ./base.nix
        ];
      };
    };

  checks = label.names {
    boot = pkgs: import ./nixos-test.nix {inherit pkgs;};

    # Rung 1 (docs/ROADMAP.md "Ship one increment"): one rust component under
    # the C systemd manager. Boots with C PID 1 but the rust systemd-tmpfiles.
    rung1-tmpfiles = pkgs: import ./rung1-tmpfiles-test.nix {inherit pkgs;};

    # Rung 1b: a second rust component (systemd-sysusers) under C PID 1.
    rung1-sysusers = pkgs: import ./rung1-sysusers-test.nix {inherit pkgs;};

    # Rung 1: the first rust DAEMON (systemd-timesyncd) under C PID 1.
    rung1-timesyncd = pkgs: import ./rung1-timesyncd-test.nix {inherit pkgs;};

    # Rung 1: a second rust DAEMON (systemd-resolved) under C PID 1.
    rung1-resolved = pkgs: import ./rung1-resolved-test.nix {inherit pkgs;};

    # Rung 1: the network-config rust DAEMON (systemd-networkd) under C PID 1.
    rung1-networkd = pkgs: import ./rung1-networkd-test.nix {inherit pkgs;};
  };
}
