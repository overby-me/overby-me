# Build the upstream patchelf test fixtures (compiled ELF binaries +
# helper data files + shell scripts) once, in a single derivation, so
# every per-test check can mount them read-only.
#
# We deliberately do NOT run "make check" here — that would invoke the
# upstream C patchelf. We only build the check_PROGRAMS targets and copy
# the whole tests/ tree into $out/tests.
{
  stdenv,
  patchelf,
  autoreconfHook,
  pkg-config,
}:
stdenv.mkDerivation {
  pname = "rust-patchelf-fixtures";
  version = patchelf.version;
  src = patchelf.src;

  nativeBuildInputs = [autoreconfHook pkg-config];

  doCheck = false;

  # nixpkgs fixupPhase runs patchelf --shrink-rpath, strip, and refuses
  # /build/... references. The upstream fixtures legitimately have all
  # three (libbar.so is linked with rpath=$PWD/no-such-path, the test
  # scripts assert the exact rpath/interpreter survived, etc.), so we
  # bypass every step of fixup that would rewrite or reject them.
  dontPatchELF = true;
  dontStrip = true;
  dontPatchShebangs = false;
  noAuditTmpdir = true;
  noBrokenSymlinks = true;
  preFixup = ''
    # Stub out the nixpkgs reference-check hook; libbar.so keeps a
    # runpath pointing into $PWD, which contains /build/.
    fixupOutputHooks=()
  '';

  buildPhase = ''
    runHook preBuild
    cd tests
    make check TESTS=
    cd ..
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r tests $out/tests
    for arch in amd64 armel armhf hurd-i386 i386 ia64 kfreebsd-amd64 \
                kfreebsd-i386 mips mipsel powerpc s390 sh4 sparc; do
      ln -sf no-rpath-prebuild.sh $out/tests/no-rpath-$arch.sh
    done
    find $out/tests -name "*.o" -delete
    runHook postInstall
  '';
}
