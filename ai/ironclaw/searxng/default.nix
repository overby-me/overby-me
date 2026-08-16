{
  packages.ironclaw-searxng-tool = {
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
      hash = "sha256-CYhaVKzw1BcbIxgbzECeKiQBtmkRWshVvLftBUpm8Sc=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-searxng-tool";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./searxng-tool.capabilities.json
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
          target/wasm32-wasip2/release/searxng_tool.wasm \
          -o searxng.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/searxng_tool.wasm searxng.wasm

        wasm-tools strip searxng.wasm -o searxng.stripped.wasm && mv searxng.stripped.wasm searxng.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp searxng.wasm $out/searxng.wasm
        cp searxng-tool.capabilities.json $out/searxng-tool.capabilities.json
      '';

      meta = {
        description = "SearXNG web search tool for IronClaw AI assistant";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/searxng";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
