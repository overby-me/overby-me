{
  stdenv,
  lib,
  fetchFromGitHub,
  udev,
  qrtr,
  qmic,
}:
stdenv.mkDerivation {
  pname = "rmtfs";
  # No versioned releases, so let's use the commit hash for now.
  version = "f7566e4c8262c618c09173b93282bec6a340663c";

  src = fetchFromGitHub {
    owner = "linux-msm";
    repo = "rmtfs";
    rev = "f7566e4c8262c618c09173b93282bec6a340663c";
    hash = "sha256-dpW68CXp9q8itzumtRWnr8qyjCup/2sb2CEwsOXAubI=";
  };

  # qmic is a code generator that runs at build time, so it must be a
  # nativeBuildInput to ensure it is built for the build platform during
  # cross-compilation (the upstream repo incorrectly puts it in buildInputs).
  nativeBuildInputs = [qmic];

  buildInputs = [udev qrtr];

  installFlags = ["prefix=$(out)"];

  meta = with lib; {
    description = "Qualcomm Remote Filesystem Service";
    homepage = "https://github.com/linux-msm/rmtfs";
    license = licenses.bsd3;
    maintainers = [];
    platforms = platforms.linux;
  };
}
