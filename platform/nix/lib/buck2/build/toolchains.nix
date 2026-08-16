# Map a buck2 "local" toolchain command name to a nixpkgs package, so builds
# are hermetic instead of depending on the host's compilers. Overridable via
# buildBuck2Project's `toolchainPackages`.
pkgs: {
  "clang++" = pkgs.clang;
  "clang" = pkgs.clang;
  "c++" = pkgs.clang;
  "cc" = pkgs.clang;
  "gcc" = pkgs.gcc;
  "g++" = pkgs.gcc;
  "rustc" = pkgs.rustc;
  "go" = pkgs.go;
  # Archivers and the rest of binutils: a cc_library's archive step runs bare `ar`
  # (and `ranlib`/`strip` alongside it), which is not the C compiler and so was not
  # covered by the compiler entries above.
  "ar" = pkgs.binutils;
  "ranlib" = pkgs.binutils;
  "strip" = pkgs.binutils;
  "ld" = pkgs.binutils;
  "llvm-ar" = pkgs.llvmPackages.bintools;
  "llvm-ranlib" = pkgs.llvmPackages.bintools;
  "llvm-strip" = pkgs.llvmPackages.bintools;
}
