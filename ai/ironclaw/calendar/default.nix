{
  packages.ironclaw-calendar-channel = {
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
      hash = "sha256-URj6mhhifij9DmFUazsLNAWOuiUiPnD7mvPp1SMJP6E=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-calendar-channel";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./calendar.capabilities.json
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
          target/wasm32-wasip2/release/calendar_channel.wasm \
          -o calendar.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/calendar_channel.wasm calendar.wasm

        wasm-tools strip calendar.wasm -o calendar.stripped.wasm && mv calendar.stripped.wasm calendar.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp calendar.wasm $out/calendar.wasm
        cp calendar.capabilities.json $out/calendar.capabilities.json
      '';

      meta = {
        description = "Calendar channel for IronClaw AI assistant via CalDAV";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/calendar";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
