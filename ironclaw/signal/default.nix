{
  packages.ironclaw-signal-channel = {
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
      hash = "sha256-9Tvlx4PhTWTc9jX4PZpk7ki99z4cpI6MGP3Ghmj9Tks=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-signal-channel";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./signal.capabilities.json
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
          target/wasm32-wasip2/release/signal_channel.wasm \
          -o signal.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/signal_channel.wasm signal.wasm

        wasm-tools strip signal.wasm -o signal.stripped.wasm && mv signal.stripped.wasm signal.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp signal.wasm $out/signal.wasm
        cp signal.capabilities.json $out/signal.capabilities.json
      '';

      meta = {
        description = "Signal channel for IronClaw AI assistant";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/signal";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
