{
  pkgs,
  lib,
  config,
  ...
}: let
  # Translate a Nix value into a nushell literal. home.sessionVariables
  # values are strings or ints; everything is emitted as a quoted string,
  # which matches how POSIX `export` treats them in hm-session-vars.sh.
  toNuString = v: ''"${lib.replaceStrings [''"'' ''\''] [''\"'' ''\\''] (toString v)}"'';

  # nushell as a login shell on macOS does NOT source hm-session-vars.sh
  # (it's POSIX `export` syntax that nushell can't parse), so none of
  # home.sessionVariables reaches nushell. Re-emit them here in nushell
  # syntax so nushell gets the same environment as zsh/bash, including the
  # proxy and CA vars (HTTPS_PROXY, *_CA_*, NIX_SSL_CERT_FILE, ...).
  sessionVars =
    lib.concatStringsSep "\n"
    (lib.mapAttrsToList (n: v: "$env.${n} = ${toNuString v}")
      config.home.sessionVariables);

  # Likewise, nushell never picks up the nix / home-manager profile PATH
  # entries that /etc/zshenv and hm-session-vars.sh add for POSIX shells.
  # Prepend them so `nix` and the other HM packages are on PATH.
  #
  # `/run/wrappers/bin` MUST come first: it holds the setuid wrappers (sudo,
  # mount, ping, …). Without it here, `/run/current-system/sw/bin` — which ships
  # a non-setuid sudo — shadows the real wrapper and breaks `sudo`.
  profilePaths = [
    "/run/wrappers/bin"
    "/etc/profiles/per-user/${config.home.username}/bin"
    "${config.home.homeDirectory}/.nix-profile/bin"
    "/run/current-system/sw/bin"
    "/nix/var/nix/profiles/default/bin"
  ];
  pathSetup = ''
    $env.PATH = (
      ${lib.toJSON profilePaths}
      | append ($env.PATH | if ($in | describe) == "string" { split row (char esep) } else { $in })
      | uniq
    )
  '';
in {
  programs.nushell = {
    enable = true;
    package = pkgs.pkgsUnstable.nushell;
    configFile.source = ./config.nu;
    envFile.text = ''
      $env.SHELL = "${pkgs.nushell}/bin/nu"

      ${pathSetup}

      ${sessionVars}
    '';
  };

  programs.nushell-plugin-tramp.enable = false;
}
