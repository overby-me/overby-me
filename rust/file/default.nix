{
  packages = {
    rust-file = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-file";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        meta = {
          description = "A GNU file-compatible file type detection tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/file";
          license = lib.licenses.mit;
          mainProgram = "file";
        };
      };

    rust-file-dev = {
      lib,
      rustPlatform,
    }:
      rustPlatform.buildRustPackage {
        pname = "rust-file-dev";
        version = "0.1.0";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        cargoLock.lockFile = ./Cargo.lock;

        buildType = "debug";

        meta = {
          description = "A GNU file-compatible file type detection tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/file";
          license = lib.licenses.mit;
          mainProgram = "file";
        };
      };
  };

  # One nix check per sample in file/file-tests — diffs `rust-file` output
  # against the upstream `file` binary (both running in the same sandbox).
  checks = let
    # Pinned snapshot of https://github.com/file/file-tests. `fetchTarball`
    # runs at nix-eval time so we can enumerate the sample set from the
    # filesystem directly. Bump rev+sha256 to refresh the corpus.
    fileTestsSrc = builtins.fetchTarball {
      url = "https://github.com/file/file-tests/archive/0bcc555a638bc38cfd9a962af1bd236dfbcfdbc4.tar.gz";
      sha256 = "0nqqvdhv0g7cj9gj1xngyp9d20lfcmj8i5hi4f09d0bpca4b3kks";
    };

    hasSuffix = suffix: s: let
      sl = builtins.stringLength s;
      fl = builtins.stringLength suffix;
    in
      fl <= sl && builtins.substring (sl - fl) fl s == suffix;

    replaceDots = s: builtins.replaceStrings ["."] ["_"] s;

    # Enumerate `db/<type>/<sample>` pairs at eval time. Skip the companion
    # `.source.txt` provenance files and any stored `.json` metadata — only
    # the binary samples are interesting as test inputs.
    dbDir = "${fileTestsSrc}/db";
    typeEntries = builtins.readDir dbDir;
    types =
      builtins.filter (t: typeEntries.${t} == "directory")
      (builtins.attrNames typeEntries);

    samplesInType = type: let
      entries = builtins.readDir "${dbDir}/${type}";
      files =
        builtins.filter (f: entries.${f} == "regular")
        (builtins.attrNames entries);
    in
      builtins.filter
      (f: !(hasSuffix ".source.txt" f) && !(hasSuffix ".json" f))
      files;

    pairs =
      builtins.concatMap (type:
        map (file: {inherit type file;}) (samplesInType type))
      types;

    # Test attribute names embed `type` and `file`. The two parts are joined
    # by a `__` sentinel so filenames containing `-` don't collide with a
    # `-` separator. Dots become underscores — attribute names accept them,
    # but plain alphanumerics make shell tab-completion cleaner.
    keyOf = p: "${replaceDots p.type}__${replaceDots p.file}";
  in
    builtins.listToAttrs (map (p: {
        name = "rust-file-test-${keyOf p}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs fileTestsSrc;
            inherit (p) type file;
          };
      })
      pairs);
}
