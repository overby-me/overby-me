{
  lib,
  rustPlatform,
  fetchFromGitHub,
  rust-pkg-config,
  openssl,
  stdenv,
  darwin,
  python3,
}:
rustPlatform.buildRustPackage rec {
  pname = "rcgen";
  version = "0.14.3";

  src = fetchFromGitHub {
    owner = "rustls";
    repo = "rcgen";
    rev = "v${version}";
    hash = "sha256-MtzOR7NIXZhGwmGdMvvI8zhKoqRTyiLaS+bIkD4wpeY=";
  };

  cargoHash = "sha256-hh328dPgbRJuy/YDZ/TBI807qmJ6qdiwnUn0Ols0BFo=";

  nativeBuildInputs = [
    rust-pkg-config
    rustPlatform.bindgenHook
    python3
  ];

  buildInputs =
    [
      openssl
    ]
    ++ lib.optionals stdenv.isDarwin [
      darwin.apple_sdk.frameworks.Security
    ];

  meta = {
    description = "Generate X.509 certificates, CSRs";
    homepage = "https://github.com/rustls/rcgen";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "rustls-cert-gen";
  };
}
