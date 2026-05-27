{
  pkgs,
  lib,
  stdenv,
  fetchFromGitHub,
}: let
  pname = "breezy-desktop";
  uuid = "breezydesktop@org.xronlinux";
in
  stdenv.mkDerivation {
    inherit pname;
    version = "";

    src = fetchFromGitHub {
      owner = "wheaney";
      repo = pname;
      rev = "0cba1b8075471dcb47511f8f793160729fc9ce30";
      sha256 = "sha256-4zjs0GHeiRWmnuIEzfUrr/Y/keKlDx7060OsFO7ft0M=";
    };

    nativeBuildInputs = with pkgs; [buildPackages.glib];
    installPhase = ''
      mkdir -p $out/share/gnome-shell/extensions/
      cp -r -T gnome/${uuid} $out/share/gnome-shell/extensions/${uuid}
    '';
    meta = {
      description = "Breezy GNOME XR Desktop";
      longDescription = "XR virtual desktop for GNOME.";
      homepage = "https://github.com/wheaney/breezy-desktop";
      license = lib.licenses.gpl2Plus; # https://wiki.gnome.org/Projects/GnomeShell/Extensions/Review#Licensing
      maintainers = with lib.maintainers; [overby-me];
    };
    passthru = {
      extensionPortalSlug = pname;
      extensionUuid = uuid;
    };
  }
