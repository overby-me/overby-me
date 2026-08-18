{
  lib,
  stdenv,
  fetchFromGitLab,
  bluez,
  coreutils,
  gnugrep,
  gnused,
  gawk,
  iproute2,
  util-linux,
  makeWrapper,
}:
stdenv.mkDerivation rec {
  pname = "bootmac";
  version = "0.7.0";

  src = fetchFromGitLab {
    domain = "gitlab.postmarketos.org";
    owner = "postmarketOS";
    repo = "bootmac";
    rev = "v${version}";
    hash = "sha256-HMXre5oyVhit+nFJlqTiZtZi+GWjn5++2Js/JjqJWus=";
  };

  nativeBuildInputs = [makeWrapper];

  buildInputs = [
    bluez
    coreutils
    gnugrep
    gnused
    gawk
    iproute2
    util-linux
  ];

  dontBuild = true;

  installPhase = ''
    runHook preInstall

    install -Dm755 bootmac $out/bin/bootmac

    wrapProgram $out/bin/bootmac \
      --prefix PATH : ${lib.makeBinPath [
      bluez
      coreutils
      gnugrep
      gnused
      gawk
      iproute2
      util-linux
    ]}

    runHook postInstall
  '';

  meta = with lib; {
    description = "Configure MAC addresses at boot for WLAN and Bluetooth interfaces";
    homepage = "https://gitlab.postmarketos.org/postmarketOS/bootmac";
    license = licenses.gpl3Plus;
    maintainers = [];
    platforms = platforms.linux;
  };
}
