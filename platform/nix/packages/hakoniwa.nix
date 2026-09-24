{
  lib,
  rustPlatform,
  fetchFromGitHub,
  libseccomp,
}:
rustPlatform.buildRustPackage rec {
  pname = "hakoniwa";
  version = "1.7.2";

  src = fetchFromGitHub {
    owner = "souk4711";
    repo = "hakoniwa";
    rev = "v${version}";
    hash = "sha256-UKudgBDn1kziBWIAZMVdaF2/+1pL42xqbvDBgxUuGCU=";
  };

  cargoHash = "sha256-HAHjLS7GGQVqFFUrfHU366JXsWQn6EA1tHqMQw35DSY=";

  buildInputs = [
    libseccomp
  ];

  # Tests tries to use /bin/sleep
  doCheck = false;

  meta = {
    description = "Process isolation for Linux using namespaces, resource limits, cgroups, landlock and seccomp";
    homepage = "https://github.com/souk4711/hakoniwa";
    license = lib.licenses.lgpl3Only;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "hakoniwa";
  };
}
