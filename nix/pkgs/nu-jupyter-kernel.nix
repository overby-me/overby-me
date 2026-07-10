{
  lib,
  rustPlatform,
  fetchFromGitHub,
  rust-pkg-config,
  fontconfig,
  python3,
  runCommand,
}: let
  nu-jupyter-kernel = rustPlatform.buildRustPackage rec {
    pname = "nu-jupyter-kernel";
    version = "0.1.14+0.110.0";

    src = fetchFromGitHub {
      owner = "cptpiepmatz";
      repo = "nu-jupyter-kernel";
      rev = "nu-jupyter-kernel/v${version}";
      hash = "sha256-D56tUCFe4jedjOHLRY2fBi3bMgwPECkwJWRnQSW/NeU=";
    };

    cargoHash = "sha256-ZqTleo/ql8KY2UV8/xglRbO1KJhfXtdyvksguT2xOaM=";

    nativeBuildInputs = [
      rust-pkg-config
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
