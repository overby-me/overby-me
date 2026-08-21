{lib, ...}: {
  packages = {
    default = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-file";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/lib/cargo/index;

        meta = {
          description = "A GNU file-compatible file type detection tool written in Rust";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/file";
          license = lib.licenses.mit;
          mainProgram = "file";
          platforms = lib.platforms.linux;
        };
      };

    dev = {lib, ...}:
      lib.buildCargoProject {
        pname = "rust-file-dev";

        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./src
          ];
        };

        index = ../../../platform/nix/lib/lib/cargo/index;

        release = false;

        meta = {
          description = "A GNU file-compatible file type detection tool written in Rust (dev build, fast compile)";
          homepage = "https://tangled.org/overby.me/overby.me/tree/main/rust/file";
          license = lib.licenses.mit;
          mainProgram = "file";
          platforms = lib.platforms.linux;
        };
      };
  };

  # One nix check per sample in file/file-tests — diffs `oxidized-file` output
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
      sl = lib.stringLength s;
      fl = lib.stringLength suffix;
    in
      fl <= sl && lib.substring (sl - fl) fl s == suffix;

    replaceDots = s: lib.replaceStrings ["."] ["_"] s;

    # Enumerate `db/<type>/<sample>` pairs at eval time. Skip the companion
    # `.source.txt` provenance files and any stored `.json` metadata — only
    # the binary samples are interesting as test inputs.
    dbDir = "${fileTestsSrc}/db";
    typeEntries = lib.readDir dbDir;
    types = lib.filter (t: typeEntries.${t} == "directory") (lib.attrNames typeEntries);

    samplesInType = type: let
      entries = lib.readDir "${dbDir}/${type}";
      files = lib.filter (f: entries.${f} == "regular") (lib.attrNames entries);
    in
      lib.filter (f: !(hasSuffix ".source.txt" f) && !(hasSuffix ".json" f)) files;

    pairs = lib.concatMap (type: map (file: {inherit type file;}) (samplesInType type)) types;

    # Test attribute names embed `type` and `file`. The two parts are joined
    # by a `__` sentinel so filenames containing `-` don't collide with a
    # `-` separator. Dots become underscores — attribute names accept them,
    # but plain alphanumerics make shell tab-completion cleaner.
    keyOf = p: "${replaceDots p.type}__${replaceDots p.file}";
  in
    lib.listToAttrs (
      map (p: {
        name = "test-${keyOf p}";
        value = pkgs:
          import ./testsuite.nix {
            inherit pkgs fileTestsSrc;
            inherit (p) type file;
          };
      })
      pairs
    );
}
