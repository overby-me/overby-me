{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  oniguruma,
  rust-jemalloc-sys,
  sqlite,
  zstd,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "pi-agent-rust";
  version = "0.1.9";

  src = fetchFromGitHub {
    owner = "Dicklesworthstone";
    repo = "pi_agent_rust";
    tag = "v${finalAttrs.version}";
    hash = "sha256-enpEsk5lAI0GLoyS0wDDvZ6mc2Ja3e3vf92+Dz47OVE=";
  };

  cargoHash = "sha256-ePylxnZz9gTNZu/stkygim1lnd/4GFJKxW8mJB698I4=";

  doCheck = false;

  postPatch = ''
    # The repo has a git submodule pointer for legacy_pi_mono_code/pi-mono but
    # no .gitmodules entry, so fetchFromGitHub cannot resolve it.  The source
    # code only uses one file from that submodule via include_str!(), so we
    # create a minimal stub that satisfies the compiler.
    mkdir -p legacy_pi_mono_code/pi-mono/packages/ai/src
    touch legacy_pi_mono_code/pi-mono/packages/ai/src/models.generated.ts
  '';

  nativeBuildInputs = [
    pkg-config
  ];

  buildInputs = [
    oniguruma
    rust-jemalloc-sys
    sqlite
    zstd
  ];

  env = {
    RUSTC_BOOTSTRAP = 1;
    RUSTONIG_SYSTEM_LIBONIG = true;
    ZSTD_SYS_USE_PKG_CONFIG = true;
  };

  meta = {
    description = "High-performance AI coding agent CLI written in Rust with zero unsafe code";
    homepage = "https://github.com/Dicklesworthstone/pi_agent_rust";
    changelog = "https://github.com/Dicklesworthstone/pi_agent_rust/blob/${finalAttrs.src.rev}/CHANGELOG.md";
    license = lib.licenses.unfree; # MIT with OpenAI/Anthropic restriction rider
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "pi";
  };
})
