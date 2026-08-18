{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage rec {
  pname = "sunsetc";
  version = "0.3.0";

  src = fetchFromGitHub {
    owner = "mkj";
    repo = "sunset";
    rev = "sunset-${version}";
    hash = "sha256-EMuxu6ELhrdRT34CQSeYQVvmreD40ZTmwbbpUIeLzZg=";
  };

  cargoHash = "sha256-Wr663dve1d+wz5eP39IOFxdvUuHjdpp3gI2JNZa2ggM=";

  cargoBuildFlags = ["--example" "sunsetc" "-p" "sunset-stdasync"];

  postInstall = ''
    mkdir -p $out/bin
    cp target/*/release/examples/* $out/bin/
  '';

  meta = {
    description = "SSH for Rust, no_std and elsewhere";
    homepage = "https://github.com/mkj/sunset";
    changelog = "https://github.com/mkj/sunset/blob/${src.rev}/changelog.md";
    license = lib.licenses.bsd0;
    maintainers = with lib.maintainers; [overby-me];
  };
}
