{
  packages.rust-pcre2 = {lib, ...}:
    lib.buildCargoProject {
      pname = "rust-pcre2";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
        ];
      };

      index = ../../nix/lib/cargo/index;

      meta = {
        description = "A pure Rust implementation of PCRE2 (Perl Compatible Regular Expressions)";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/pcre2";
        license = lib.licenses.mit;
        platforms = lib.platforms.linux;
      };
    };
}
