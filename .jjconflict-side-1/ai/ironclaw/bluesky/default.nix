{
  packages.ironclaw-bluesky-channel = {
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
      hash = "sha256-50FUEbkktLbxlgCXqRU6nQmNtSQIYWL2VAq9cBAF3dg=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-bluesky-channel";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./bluesky.capabilities.json
        ];
      };

      nativeBuildInputs = [rustWithWasm wasm-tools];

      buildPhase = ''
        mkdir -p .cargo
        # The vendor's own config, which names <vendor>/source-registry-0 -
        # where fetchCargoVendor actually puts the crates. Pointing cargo at
        # the vendor root instead found no crates at all.
        sed 's|@vendor@|${vendoredDeps}|g' \
          ${vendoredDeps}/.cargo/config.toml > .cargo/config.toml

        cargo build --release --target wasm32-wasip2 --offline

        wasm-tools component new \
          target/wasm32-wasip2/release/bluesky_channel.wasm \
          -o bluesky.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/bluesky_channel.wasm bluesky.wasm

        wasm-tools strip bluesky.wasm -o bluesky.stripped.wasm && mv bluesky.stripped.wasm bluesky.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp bluesky.wasm $out/bluesky.wasm
        cp bluesky.capabilities.json $out/bluesky.capabilities.json
      '';

      meta = {
        description = "Bluesky/AT Protocol channel for IronClaw AI assistant";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/bluesky";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
