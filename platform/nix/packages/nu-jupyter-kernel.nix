{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  fontconfig,
  python3,
  runCommand,
}: let
  nu-jupyter-kernel = rustPlatform.buildRustPackage rec {
    pname = "nu-jupyter-kernel";
    version = "0.1.15+0.111.0";

    src = fetchFromGitHub {
      owner = "cptpiepmatz";
      repo = "nu-jupyter-kernel";
      rev = "nu-jupyter-kernel/v${version}";
      hash = "sha256-hBMmIJYRUs5fDPLYxVhfpiQHShZV4uj/o+DRuQsCIjk=";
    };

    cargoHash = "sha256-GLCK345ADX0YRBpdy0FLonow/9CM2/g/fIJO/lOX3r8=";

    nativeBuildInputs = [
      pkg-config
    ];

    buildInputs = [
      fontconfig
    ];

    meta = {
      description = "A wip jupyter raw kernel for nu";
      homepage = "https://github.com/cptpiepmatz/nu-jupyter-kernel";
      license = lib.licenses.mit;
      maintainers = with lib.maintainers; [overby-me];
      platforms = lib.platforms.linux;
      mainProgram = "nu-jupyter-kernel";
    };
  };
in
  python3.pkgs.toPythonModule (
    runCommand "nu-jupyter-kernel"
    {
      buildInputs = [nu-jupyter-kernel];
      meta.platforms = lib.platforms.linux;
    }
    ''
      export HOME=.
      ${nu-jupyter-kernel}/bin/nu-jupyter-kernel register --user
      mkdir -p $out/share/jupyter/kernels
      cp -r .local/share/jupyter/kernels/nu $out/share/jupyter/kernels
    ''
  )
