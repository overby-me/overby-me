{
  stdenv,
  lib,
  fetchFromGitHub,
  meson,
  ninja,
  pkg-config,
  qrtr,
  zstd,
}:
stdenv.mkDerivation {
  pname = "tqftpserv";
  version = "408ca1ed5e4e0a9ac3650b13d3f3c60079b3e2a3";

  src = fetchFromGitHub {
    owner = "linux-msm";
    repo = "tqftpserv";
    rev = "408ca1ed5e4e0a9ac3650b13d3f3c60079b3e2a3";
    hash = "sha256-IlM/HVdo/7cA9NnJrCW/B0yKks5jWYqxRyy3ay4wDr8=";
  };

  patches = [
    ./tqftpserv-firmware-path.diff
  ];

  nativeBuildInputs = [meson ninja pkg-config];

  buildInputs = [qrtr zstd];

  meta = with lib; {
    description = "Trivial File Transfer Protocol server over AF_QIPCRTR";
    homepage = "https://github.com/linux-msm/tqftpserv";
    license = licenses.bsd3;
    maintainers = [];
    platforms = platforms.linux;
  };
}
