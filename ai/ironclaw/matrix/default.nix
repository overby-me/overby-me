{
  packages.ironclaw-matrix-channel = {
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
      hash = "sha256-Ht6AQ1c1KsUMu/VNzs8z/nJ4W/wsu6ZqIPxFdoMIHeE=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-matrix-channel";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./matrix.capabilities.json
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
          target/wasm32-wasip2/release/matrix_channel.wasm \
          -o matrix.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/matrix_channel.wasm matrix.wasm

        wasm-tools strip matrix.wasm -o matrix.stripped.wasm && mv matrix.stripped.wasm matrix.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp matrix.wasm $out/matrix.wasm
        cp matrix.capabilities.json $out/matrix.capabilities.json
      '';

      meta = {
        description = "Matrix channel for IronClaw AI assistant";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/matrix";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
