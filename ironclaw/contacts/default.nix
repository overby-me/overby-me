{
  packages.ironclaw-contacts-channel = {
    lib,
    rust-bin,
    makeRustPlatform,
    wasm-tools,
    stdenv,
  }: let
    rustWithWasm = rust-bin.stable.latest.default.override {
      targets = ["wasm32-wasip2"];
    };
    rustPlatform = makeRustPlatform {
      cargo = rustWithWasm;
      rustc = rustWithWasm;
    };
    vendoredDeps = rustPlatform.fetchCargoVendor {
      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
        ];
      };
      hash = "sha256-gPp35N+hvotP1ws8SnP7rls788fIpZ01ISboPjz05fA=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-contacts-channel";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./contacts.capabilities.json
        ];
      };

      nativeBuildInputs = [rustWithWasm wasm-tools];

      buildPhase = ''
        mkdir -p .cargo
        cat > .cargo/config.toml <<EOF
        [source.crates-io]
        replace-with = "vendored-sources"
        [source.vendored-sources]
        directory = "${vendoredDeps}"
        EOF

        cargo build --release --target wasm32-wasip2 --offline

        wasm-tools component new \
          target/wasm32-wasip2/release/contacts_channel.wasm \
          -o contacts.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/contacts_channel.wasm contacts.wasm

        wasm-tools strip contacts.wasm -o contacts.stripped.wasm && mv contacts.stripped.wasm contacts.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp contacts.wasm $out/contacts.wasm
        cp contacts.capabilities.json $out/contacts.capabilities.json
      '';

      meta = {
        description = "Contacts channel for IronClaw AI assistant via CardDAV";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/contacts";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
