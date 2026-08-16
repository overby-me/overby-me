{
  packages.ironclaw-pixtral-tool = {
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
      hash = "sha256-USoZXxhQTu1AqcAIT9o2+zceYBnWaP+zGe7Sg6l8D74=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-pixtral-tool";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./pixtral-tool.capabilities.json
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
          target/wasm32-wasip2/release/pixtral_tool.wasm \
          -o pixtral.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/pixtral_tool.wasm pixtral.wasm

        wasm-tools strip pixtral.wasm -o pixtral.stripped.wasm && mv pixtral.stripped.wasm pixtral.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp pixtral.wasm $out/pixtral.wasm
        cp pixtral-tool.capabilities.json $out/pixtral-tool.capabilities.json
      '';

      meta = {
        description = "Pixtral AI image generation tool for IronClaw AI assistant";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/pixtral";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
