{
  devShells.mojo-wasm = pkgs: let
    inherit (pkgs) lib stdenv;
  in {
    packages = with pkgs;
      [
        just
        mojo
        deno
        wabt
        llvmPackages_latest.llvm
        llvmPackages_latest.lld
        wasmtime.lib
        wasmtime.dev
        jq
      ]
      # Servo browser engine is broken on Darwin in nixpkgs.
      ++ lib.optionals stdenv.isLinux [
        servo
      ];
  };
}
