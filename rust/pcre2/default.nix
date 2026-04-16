{
  packages.rust-pcre2 = {
    lib,
    rustPlatform,
  }:
    rustPlatform.buildRustPackage {
      pname = "rust-pcre2";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      cargoLock.lockFile = ./Cargo.lock;

      meta = {
        description = "A pure Rust implementation of PCRE2 (Perl Compatible Regular Expressions)";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/pcre2";
        license = lib.licenses.mit;
      };
    };
}
