{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage {
  pname = "layout";
  version = "unstable-2025-05-22";

  src = fetchFromGitHub {
    owner = "nadavrot";
    repo = "layout";
    rev = "440e032a8ce21b61f5ada67e2b1450fb77842b7c";
    hash = "sha256-1MR96QGFWVhIQiSSBQE5XXDT0stNIvh6d3hoy4akXTo=";
  };

  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  postPatch = ''
    ln -s ${./Cargo.lock} Cargo.lock
  '';

  meta = {
    description = "Layout is a rust library and a tool that renders Graphviz dot files";
    homepage = "https://github.com/nadavrot/layout";
    license = lib.licenses.mit;
    maintainers = with lib.maintainers; [overby-me];
    mainProgram = "layout";
  };
}
