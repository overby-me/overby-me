# Non-blocking direnv integration daemon.
#
# Ordinary direnv runs synchronously in the prompt hook, so entering a
# directory with a heavy .envrc stalls the shell until it finishes.  This runs
# direnv in the background and updates the environment when it is done, so the
# prompt returns immediately.
#
# Packaged here rather than taken as a flake input to keep the lock small; the
# upstream flake exists but only to expose this same derivation plus its
# home-manager and NixOS modules, and the integration this repo needs is four
# lines in home-manager/modules/programs/direnv.nix.
{
  direnv,
  fetchFromGitHub,
  fish,
  lib,
  nushell,
  rustPlatform,
  tmux,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "direnv-instant";
  version = "1.3.0";

  src = fetchFromGitHub {
    owner = "Mic92";
    repo = "direnv-instant";
    tag = finalAttrs.version;
    hash = "sha256-LVHm/pAZMvfTFCd/NGaerqszZ4O9Mps4XOoV1rXa62Y=";
  };

  cargoHash = "sha256-CPBD52TdCMTzvYyANoHwAT5Bnby6K/u5KrSAc59TGMY=";

  # The integration tests spawn real shells and tmux servers, so give them a
  # scratch HOME and a TMPDIR outside the build directory.
  nativeCheckInputs = [
    direnv
    fish
    nushell
    tmux
  ];

  preCheck = ''
    export HOME=$(mktemp -d)
    export TMPDIR=/tmp
  '';

  # nushell's `source` is a parse-time keyword, so it cannot consume command
  # output the way `eval "$(direnv-instant hook bash)"` does.  Ship the hook as
  # a file that can be sourced by path instead.
  postInstall = ''
    install -Dm644 hooks/nushell.nu $out/share/direnv-instant/nushell.nu
  '';

  meta = {
    description = "Non-blocking direnv integration daemon with tmux support";
    homepage = "https://github.com/Mic92/direnv-instant";
    license = lib.licenses.mit;
    mainProgram = "direnv-instant";
    maintainers = [];
    platforms = lib.platforms.unix;
  };
})
