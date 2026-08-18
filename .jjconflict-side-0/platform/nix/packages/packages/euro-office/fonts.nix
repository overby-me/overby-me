# Font, dictionary and template data. Three submodules of ready-to-use files
# with nothing to compile, fetched and exposed under one output so core's
# `allfontsgen` and the final bundle can reach them by subdirectory:
#
#   $out/core-fonts          → fonts fed to allfontsgen
#   $out/dictionaries        → spellcheck dictionaries
#   $out/document-templates  → new-document templates
{
  lib,
  stdenvNoCC,
  fetchFromGitHub,
}: let
  sources = import ./sources.nix {inherit lib;};
  fetch = name: fetchFromGitHub sources.repos.${name};
in
  stdenvNoCC.mkDerivation {
    pname = "euro-office-data";
    inherit (sources) version;

    dontUnpack = true;

    coreFonts = fetch "core-fonts";
    dictionaries = fetch "dictionaries";
    documentTemplates = fetch "document-templates";

    installPhase = ''
      runHook preInstall

      mkdir -p "$out"
      cp -r "$coreFonts" "$out/core-fonts"
      cp -r "$dictionaries" "$out/dictionaries"
      cp -r "$documentTemplates" "$out/document-templates"

      runHook postInstall
    '';

    meta = {
      description = "Euro-Office bundled fonts, dictionaries and document templates";
      homepage = "https://github.com/Euro-Office";
      license = with lib.licenses; [ofl mit asl20]; # mixed; see upstream repos
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.all;
    };
  }
