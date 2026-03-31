{
  devShells.wiki = pkgs: {
    packages = with pkgs; [
      just
      deno
    ];
  };

  packages.wiki-frontend = {
    lib,
    inputs,
    ...
  }: let
    envJson = lib.readFile inputs.env.outPath;
    env =
      if envJson != ""
      then lib.fromJSON envJson
      else {};
    gitCommitSha = env.PUBLIC_GIT_COMMIT_SHA or "dev";
  in
    lib.buildDenoProject {
      pname = "wiki-frontend";
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./deno.json
          ./package.json
          ./deno.lock
          ./tsconfig.json
          ./rsbuild.config.ts
          ./src
          ./core
          ./public
        ];
      };
      buildCommand = ''
        export PUBLIC_GIT_COMMIT_SHA="${gitCommitSha}"
        deno run -A npm:@gqty/cli generate
        deno run -A npm:@rsbuild/core build
      '';
      installPhase = "cp -r dist $out";
      meta.description = "Wiki frontend built with Deno + RSBuild";
    };
}
