{
  packages.ironclaw-mail-channel = {
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
      hash = "sha256-PGIrCAXs7Ryxbj1PruwQE+orh1Y0EOWOApe/cCfBd5Y=";
    };
  in
    stdenv.mkDerivation {
      pname = "ironclaw-mail-channel";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./src
          ./wit
          ./mail.capabilities.json
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
          target/wasm32-wasip2/release/mail_channel.wasm \
          -o mail.wasm \
          2>/dev/null || cp target/wasm32-wasip2/release/mail_channel.wasm mail.wasm

        wasm-tools strip mail.wasm -o mail.stripped.wasm && mv mail.stripped.wasm mail.wasm || true
      '';

      installPhase = ''
        mkdir -p $out
        cp mail.wasm $out/mail.wasm
        cp mail.capabilities.json $out/mail.capabilities.json
      '';

      meta = {
        description = "Email channel for IronClaw AI assistant via JMAP";
        homepage = "https://tangled.org/overby.me/overby.me/tree/main/ironclaw/mail";
        license = lib.licenses.mit;
        maintainers = with lib.maintainers; [overby-me];
      };
    };
}
