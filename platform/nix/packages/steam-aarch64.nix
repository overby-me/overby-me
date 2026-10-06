# Valve's own aarch64 Steam client, the one built for the Steam Frame, run as
# the ordinary desktop client. x86 games and Proton go through FEX, which Steam
# downloads as a compatibility tool and invokes itself, so nothing here needs an
# x86 binfmt handler, an x86 rootfs of our own, or muvm. That microVM exists to
# give Valve's binaries the 4K pages they assume on hosts whose pages are
# larger, which is Asahi's problem and not a Snapdragon's.
#
# Bundled from github:kossLAN/steam-aarch64-nix instead of taken as an input:
# the whole flake is a package and a module, and the module says less than
# enabling it on one host does.
#
# nixpkgs ships no aarch64 Steam, so this is steam-unwrapped with Valve's arm64
# bootstrap swapped in, wrapped by the same FHS env nixpkgs wraps the x86 client
# with.
{
  lib,
  steam,
  steam-unwrapped,
  buildFHSEnv,
  fetchurl,
  runCommand,
  writeShellScript,
  zstd,
}: let
  # Valve's repository keeps about four builds, some two weeks, so this pin
  # 404s once it ages out. What is still there:
  #   curl -s https://steamdeck-packages.steamos.cloud/archlinux-deckard-hotfixes/release/0.4.x/
  version = "r1790377368+bffc849f";

  client = fetchurl {
    pname = "deckard-steam-rel";
    inherit version;
    url = "https://steamdeck-packages.steamos.cloud/archlinux-deckard-hotfixes/release/0.4.x/deckard-steam-rel-${lib.escapeURL version}-1-aarch64.pkg.tar.zst";
    hash = "sha256-pa/nEtT4cMOFycFkE1vc37TmMDgKj8lCgWpAYJEQsRo=";
  };

  bootstrap =
    runCommand "steam-aarch64-bootstrap.tar.zst" {
      nativeBuildInputs = [zstd];
    } ''
      tar -xOf ${client} usr/lib/steam/steam.tar.zst > $out
    '';

  launcher = writeShellScript "steam" ''
    steamroot="$HOME/.local/share/Steam"
    client="$steamroot/steamrtarm64"

    if [[ ! -x "$client/steam" ]]; then
      mkdir -p "$steamroot"
      tar --zstd -xf ${bootstrap} -C "$steamroot"
    fi

    # Steam resolves its own install through ~/.steam rather than through the
    # directory it was started from, and the arm64 client adds sdkarm64 and
    # binarm64 to the set the x86 one wanted.
    mkdir -p ~/.steam
    ln -sfnT "$steamroot" ~/.steam/steam
    ln -sfnT "$steamroot" ~/.steam/root
    ln -sfnT "$steamroot/linux32" ~/.steam/sdk32
    ln -sfnT "$steamroot/linux64" ~/.steam/sdk64
    ln -sfnT "$steamroot/linuxarm64" ~/.steam/sdkarm64
    ln -sfnT "$steamroot/ubuntu12_32" ~/.steam/bin32
    ln -sfnT "$steamroot/ubuntu12_64" ~/.steam/bin64
    ln -sfnT "$client" ~/.steam/binarm64

    export LD_LIBRARY_PATH="$client"

    # The runtime's own run.sh is an x86 binary, so asking it where its
    # libraries are means asking it through FEX. Absent until Steam has
    # downloaded the compatibility tool, which is why this is conditional.
    export STEAM_RUNTIME="$steamroot/ubuntu12_32/steam-runtime"
    fex="$steamroot/steamapps/common/FEX-Emu/fex-compat-tool"
    if [[ -x "$fex" ]]; then
      export STEAM_RUNTIME_LIBRARY_PATH="$("$fex" run -- "$STEAM_RUNTIME/run.sh" --print-steam-runtime-library-paths)"
    fi

    cd "$steamroot"
    # 42 is Steam asking to be restarted after updating itself.
    while true; do
      "$client/steam" "$@"
      status=$?

      if (( status != 42 )); then
        exit "$status"
      fi
    done
  '';

  unwrapped = steam-unwrapped.overrideAttrs (old: {
    pname = "steam-aarch64-unwrapped";

    postInstall =
      (old.postInstall or "")
      + ''
        rm $out/lib/steam/{bin_steam.sh,bootstraplinux_ubuntu12_32.tar.xz}
        ln -sf ${launcher} $out/bin/steam
      '';

    meta =
      old.meta
      // {
        platforms = ["aarch64-linux"];
      };
  });
in
  steam.override {
    steam-unwrapped = unwrapped;

    # nixpkgs' FHS env asks for glibc_multi, which exists on x86_64-linux alone.
    buildFHSEnv = args:
      buildFHSEnv (args
        // {
          targetPkgs = pkgs: args.targetPkgs (pkgs // {glibc_multi = pkgs.glibc;});
        });

    extraPkgs = pkgs:
      with pkgs; [
        lsof
        util-linux
        xwininfo
        zstd
      ];

    # The arm64 client links against the host rather than against the Steam
    # runtime, which carries no arm64 build of any of these.
    extraLibraries = pkgs:
      with pkgs; [
        alsa-lib
        cups
        gtk2
        ibus
        libasyncns
        libice
        libpulseaudio
        libsm
        libuuid
        libvdpau
        libxcomposite
        libxdamage
        libxfixes
        libxinerama
        libxkbcommon
        libxtst
        nss
        openal
        pipewire
        SDL2
        systemdLibs
      ];
  }
