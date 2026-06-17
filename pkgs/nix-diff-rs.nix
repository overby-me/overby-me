{
  lib,
  rustPlatform,
  fetchFromGitHub,
  rust-jemalloc-sys,
}:
rustPlatform.buildRustPackage {
  pname = "nix-diff-rs";
  version = "unstable-2025-11-02";

  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "nix-diff-rs";
    rev = "7b79a68963fd7fcb48a57071b52bc90bd8d60609";
    hash = "sha256-heUqcAnGmMogyVXskXc4FMORb8ZaK6vUX+mMOpbfSUw=";
  };

  cargoHash = "sha256-rPrzxePIdyqt0THYfTy15AB7+NyPPmu/nevsdcHmFgQ=";

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
