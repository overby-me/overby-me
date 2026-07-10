{
  pkgs,
  lib,
  ...
}: let
  real-jj = pkgs.jujutsu;
  jj-hooks-wrapper = pkgs.writeScriptBin "jj" (
    lib.replaceStrings ["@JJ_BIN@"] ["${real-jj}/bin/jj"]
    (lib.readFile ../packages/scripts/jj-hooks-wrapper)
  );
in {
  programs.jujutsu = {
    enable = true;
    package = pkgs.symlinkJoin {
      name = "jj-with-hooks";
      paths = [jj-hooks-wrapper real-jj];
    };
    settings = {
      user = {
        name = "Niclas Overby";
        email = "niclas@overby.me";
      };
      templates = {
        commit_trailers = "format_signed_off_by_trailer(self)";
      };
      ui = {
        pager = "delta";
        diff-formatter = ":git";
      };
    };
  };
}
