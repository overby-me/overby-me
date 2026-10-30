# Vendored from nixpkgs PR #537897 (tracey: init at 1.4.0, by olestrohm),
# which has not merged; drop this file once the pin carries it.
{
  fetchFromGitHub,
  fetchPnpmDeps,
  gitUpdater,
  git,
  lib,
  nodejs,
  pnpmConfigHook,
  pnpm,
  rustPlatform,
  stdenv,
  versionCheckHook,
}:
rustPlatform.buildRustPackage rec {
  pname = "tracey";
  version = "1.4.0";
  __structuredAttrs = true;

  src = fetchFromGitHub {
    owner = "bearcove";
    repo = pname;
    tag = "v${version}";
    hash = "sha256-S0uAAZpJrYp24ROMjGn3gcMjzzsBjOeICRO0qXXnmxg=";
  };
  passthru.updateScript = gitUpdater {rev-prefix = "v";};

  cargoHash = "sha256-AUDBWCWXBdCUK3535RlFWsbUVH0Uws6DxqWzcfyZrJI=";

  pnpmDeps = fetchPnpmDeps {
    inherit pname src;
    sourceRoot = "${src.name}/crates/tracey/src/bridge/http/dashboard";
    fetcherVersion = 3;
    hash = "sha256-M9dDIZC8fdk3xvbs+1J41FbvtGHh8kJjDG+QNuB6EXQ=";
  };
  pnpmRoot = "crates/tracey/src/bridge/http/dashboard";

  prePnpmInstall = ''
    # Approve build scripts of dependencies that require it
    pnpm config set --location=project --json allowBuilds '{ "esbuild": true, "@parcel/watcher": true }'
  '';

  nativeBuildInputs = [
    nodejs
    pnpmConfigHook
    pnpm
  ];

  # The cargo tests fail on darwin; linux still runs them.
  doCheck = !stdenv.hostPlatform.isDarwin;

  nativeCheckInputs = [
    git
  ];

  nativeInstallCheckInputs = [
    versionCheckHook
  ];
  doInstallCheck = true;

  meta = {
    description = "CLI, Web, LSP, and MCP toolkit to measure spec coverage in Rust codebases";
    homepage = "https://tracey.bearcove.eu/";
    license = with lib.licenses; [
      mit
      asl20
    ];
    mainProgram = "tracey";
    maintainers = with lib.maintainers; [
      olestrohm
    ];
  };
}
