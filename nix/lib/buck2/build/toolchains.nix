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
}
