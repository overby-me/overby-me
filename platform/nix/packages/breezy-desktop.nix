{
  pkgs,
  lib,
  stdenv,
  fetchFromGitHub,
}: let
  pname = "breezy-desktop";
  # Renamed from breezydesktop@org.xronlinux in 2.12; GNOME keys an extension
  # by this, so the old one leaves a dead entry behind.
  uuid = "breezydesktop@xronlinux.com";
in
  stdenv.mkDerivation {
    inherit pname;
    version = "2.12.2";

    src = fetchFromGitHub {
      owner = "wheaney";
      repo = pname;
      rev = "v2.12.2";
      # The extension symlinks out to modules/sombrero for its shader and
      # calibration texture, and that is a submodule.
      fetchSubmodules = true;
      sha256 = "sha256-U5Qtl93Lrazqo85oFp89P3EzkbffgIPLtKFwa0JxDlU=";
    };

    nativeBuildInputs = with pkgs; [buildPackages.glib];
    installPhase = ''
      mkdir -p $out/share/gnome-shell/extensions/
      # -L because the tree symlinks its shader, schema and textures out to
      # sibling directories that do not ship with the extension.
      cp -rL -T gnome/src $out/share/gnome-shell/extensions/${uuid}
    '';
    meta = {
      description = "Breezy GNOME XR Desktop";
      longDescription = "XR virtual desktop for GNOME.";
      homepage = "https://github.com/wheaney/breezy-desktop";
      license = lib.licenses.gpl2Plus; # https://wiki.gnome.org/Projects/GnomeShell/Extensions/Review#Licensing
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.linux;
    };
    passthru = {
      extensionPortalSlug = pname;
      extensionUuid = uuid;
    };
  }
