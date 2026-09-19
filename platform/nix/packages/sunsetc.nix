{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage rec {
  pname = "sunsetc";
  version = "0.6.0";

  src = fetchFromGitHub {
    owner = "mkj";
    repo = "sunset";
    rev = "sunset-${version}";
    hash = "sha256-It4pDdzfzY1mFTkayOnqPoYN0dBHsF6iCFl4i7lxN40=";
  };

  cargoHash = "sha256-RXpVghgpkrtsFueIcjL/mRWal8DadpR9XrJ/g68XayI=";

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
