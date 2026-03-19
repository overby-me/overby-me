{
  lib,
  rustPlatform,
  fetchFromGitHub,
  libseccomp,
}:
rustPlatform.buildRustPackage rec {
  pname = "hakoniwa";
  version = "1.3.1";

  src = fetchFromGitHub {
    owner = "souk4711";
    repo = "hakoniwa";
    rev = "v${version}";
    hash = "sha256-2QvOAcJvgXDE8tEqzaoZDV0R+yHK1ggAAEbnlK6jBac=";
  };

  cargoHash = "sha256-I+GM6G0BqGyxbrYKT9x1RaK2uKOldxgitjhjRZXgT4Y=";

  buildInputs = [
    libseccomp
  ];

  # Tests tries to use /bin/sleep
  doCheck = false;

  meta = {
    description = "Process isolation for Linux using namespaces, resource limits, cgroups, landlock and seccomp";
    homepage = "https://github.com/souk4711/hakoniwa";
    license = lib.licenses.lgpl3Only;
    maintainers = with lib.maintainers; [noverby];
    mainProgram = "hakoniwa";
  };
}
