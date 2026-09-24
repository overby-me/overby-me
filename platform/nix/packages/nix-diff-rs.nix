{
  lib,
  rustPlatform,
  fetchFromGitHub,
  rust-jemalloc-sys,
}:
rustPlatform.buildRustPackage {
  pname = "nix-diff-rs";
  version = "unstable-2026-09-12";

  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "nix-diff-rs";
    rev = "a8c9a1b679bf412f5a9d3707cdb41dba20938c23";
    hash = "sha256-+cc2D67WrpBHagcqLqVEbkFk8mvZaIAW3NUM1JWqeqE=";
  };

  cargoHash = "sha256-DPHxOPBllnO6fyIRRElPo8WgZEWXL2Dq7qR4ePxiaH4=";

  buildInputs = [
    rust-jemalloc-sys
  ];

  doCheck = false;

  meta = {
    description = "A Rust port of nix-diff, a tool to explain why two Nix derivations differ";
    homepage = "https://github.com/Mic92/nix-diff-rs";
    license = lib.licenses.bsd3;
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "nix-diff";
  };
}
