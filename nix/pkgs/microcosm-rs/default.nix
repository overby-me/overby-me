{
  lib,
  rustPlatform,
  fetchFromGitHub,
  rust-pkg-config,
  openssl,
  perl,
  zstd,
  libclang,
  rocksdb,
}:
rustPlatform.buildRustPackage rec {
  pname = "microcosm-rs";
  version = "0.0.2";

  src = fetchFromGitHub {
    owner = "at-microcosm";
    repo = "microcosm-rs";
    rev = "reflector-v${version}";
    hash = "sha256-GcOV2/LhxQ7Du+Yr9CZIAKiYVBgS90u0/sAxoPHIpWc=";
  };

  postPatch = ''
    cp ${./Cargo.lock} Cargo.lock

    # Pin fjall to avoid update to 3.0.1 which breaks compilation
    sed -i 's|fjall = { git = "https://github.com/fjall-rs/fjall.git", features = \["lz4"\] }|fjall = { git = "https://github.com/fjall-rs/fjall.git", rev = "42d811f7c8cc9004407d520d37d2a1d8d246c03d", features = ["lz4"] }|' ufos/Cargo.toml

    # Pin jwt-compact
    sed -i 's|jwt-compact = { git = "https://github.com/fatfingers23/jwt-compact.git", features = \["es256k"\] }|jwt-compact = { git = "https://github.com/fatfingers23/jwt-compact.git", rev = "aed088b8ff5ad44ef2785c453f6a4b7916728b1c", features = ["es256k"] }|' pocket/Cargo.toml

    # Pin fjall in quasar to use git version instead of registry
    sed -i 's|fjall = "2.11.2"|fjall = { git = "https://github.com/fjall-rs/fjall.git", rev = "42d811f7c8cc9004407d520d37d2a1d8d246c03d" }|' quasar/Cargo.toml

    cat >> Cargo.toml <<EOF

    [patch.crates-io]
    atrium-api = { git = "https://github.com/uniphil/atrium.git", branch = "fix/resolve-handle-https-accept-whitespace" }
    atrium-common = { git = "https://github.com/uniphil/atrium.git", branch = "fix/resolve-handle-https-accept-whitespace" }
    atrium-xrpc = { git = "https://github.com/uniphil/atrium.git", branch = "fix/resolve-handle-https-accept-whitespace" }
    atrium-identity = { git = "https://github.com/uniphil/atrium.git", branch = "fix/resolve-handle-https-accept-whitespace" }
    atrium-oauth = { git = "https://github.com/uniphil/atrium.git", branch = "fix/resolve-handle-https-accept-whitespace" }
    EOF
  '';

  cargoLock = {
    lockFile = ./Cargo.lock;
    outputHashes = {
      "atrium-api-0.25.4" = "sha256-7qBKwPzUMmgJWca4Z4nT9nXq9XvRja9VSNjBFsP7WXg=";
      "atrium-common-0.1.2" = "sha256-7qBKwPzUMmgJWca4Z4nT9nXq9XvRja9VSNjBFsP7WXg=";
      "atrium-identity-0.1.5" = "sha256-7qBKwPzUMmgJWca4Z4nT9nXq9XvRja9VSNjBFsP7WXg=";
      "atrium-oauth-0.1.3" = "sha256-7qBKwPzUMmgJWca4Z4nT9nXq9XvRja9VSNjBFsP7WXg=";
      "atrium-xrpc-0.12.3" = "sha256-7qBKwPzUMmgJWca4Z4nT9nXq9XvRja9VSNjBFsP7WXg=";
      "fjall-2.11.2" = "sha256-z1Kd9SPzsqAaizNbVc4rzcD2cDOUTuamsXzCL7M3ut4=";
      "jwt-compact-0.9.0-beta.1" = "sha256-x1n7eSMn/i2G5umypVkEtLiiWyn8xrRjeXHLtSkFYeU=";
    };
  };

  cargoBuildFlags = ["-p" "constellation" "-p" "spacedust" "-p" "slingshot" "-p" "ufos" "-p" "pocket"];

  nativeBuildInputs = [
    rust-pkg-config
    perl
  ];

  buildInputs = [
    openssl
    zstd
    rocksdb
  ];

  LIBCLANG_PATH = lib.makeLibraryPath [libclang];
  ROCKSDB_LIB_DIR = lib.makeLibraryPath [rocksdb];

  # Force openssl-sys to use the system OpenSSL
  OPENSSL_NO_VENDOR = "1";

  meta = {
    description = "Rust atproto crates and services for microcosm";
    homepage = "https://github.com/at-microcosm/microcosm-rs";
    license = with lib.licenses; [agpl3Only];
    maintainers = with lib.maintainers; [overby-me];
  };
}
