{
  lib,
  stdenv,
  fetchFromGitHub,
  cmake,
  python3,
  rust-pkg-config,
  xeus,
  xeus-zmq,
  cppzmq,
  openssl,
  nlohmann_json,
  libuuid,
  boost,
  lix,
  capnproto,
  libsodium,
}:
python3.pkgs.toPythonModule (stdenv.mkDerivation {
  pname = "xeus-lix";
  version = "unstable";

  src = fetchFromGitHub {
    owner = "ptrpaws";
    repo = "xeus-lix";
    rev = "a88bd9e02f8c960f02cb5642fbe36e6a2fc6cbc7";
    hash = "sha256-5S6ErMn5iJCJek3FmKLgQdIHDau7YTvVql39UeO2YHk=";
  };

  patches = [
    ./xeus-lix.patch
  ];

  nativeBuildInputs = [
    cmake
    rust-pkg-config
  ];

  buildInputs = [
    (lix.overrideAttrs {doCheck = false;}).dev
    capnproto
    xeus
    xeus-zmq
    cppzmq
    openssl
    nlohmann_json
    libuuid
    boost
    libsodium
  ];

  cmakeFlags = [
    "-DPython3_EXECUTABLE=${python3}/bin/python3"
    "-DCMAKE_CXX_FLAGS=-DSYSTEM=\\\"${stdenv.hostPlatform.system}\\\""
  ];

  meta = {
    description = "";
    homepage = "https://github.com/ptrpaws/xeus-lix";
    license = lib.licenses.lgpl21Only;
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "xlix";
    platforms = lib.platforms.all;
  };
})
