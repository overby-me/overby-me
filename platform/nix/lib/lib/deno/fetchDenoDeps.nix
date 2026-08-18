# npm dependencies from a deno.lock, one fetchurl per package rather than a
# single fixed-output derivation with a hand-maintained hash. The lock is parsed
# at evaluation time and its integrity hashes are the fetch hashes, so the
# derivation graph follows the lock's content: each npm entry becomes its own
# fetch, and a pure derivation assembles them into a DENO_DIR cache layout.
{
  lib,
  stdenvNoCC,
  fetchurl,
  jq,
  writeText,
}: {lockFile}: let
  lockData = lib.fromJSON (lib.readFile lockFile);
  npmPackages = lockData.npm or {};

  # Parse npm key "name@version[_peerinfo]" into { name, version }
  parseNpmKey = key: let
    scopedMatch = lib.match "(@[^@]+)@([^_]+)(_.*)?" key;
    unscopedMatch = lib.match "([^@]+)@([^_]+)(_.*)?" key;
    m =
      if scopedMatch != null
      then scopedMatch
      else unscopedMatch;
  in {
    name = lib.elemAt m 0;
    version = lib.elemAt m 1;
  };

  tarballBasename = name:
    lib.last (lib.splitString "/" name);

  mkTarballUrl = name: version: let
    basename = tarballBasename name;
  in "https://registry.npmjs.org/${name}/-/${basename}-${version}.tgz";

  # Deduplicate by name@version (peer dep variants share the same tarball).
  uniquePackages = let
    entries =
      lib.mapAttrsToList (key: value: let
        p = parseNpmKey key;
      in {
        name = "${p.name}@${p.version}";
        value = {
          inherit (p) name version;
          inherit (value) integrity;
        };
      })
      npmPackages;
  in
    lib.listToAttrs entries;

  fetchedPackages =
    lib.mapAttrsToList (_: {
      name,
      version,
      integrity,
    }: {
      inherit name version integrity;
      url = mkTarballUrl name version;
      tarball = fetchurl {
        url = mkTarballUrl name version;
        hash = integrity;
      };
    })
    uniquePackages;

  # Generate a manifest file mapping package info to tarball store paths.
  # This avoids inlining huge bash scripts that exceed argument length limits
  # for projects with many dependencies (e.g. wiki has 500+ packages).
  manifest = lib.toJSON (map (pkg: {
      inherit (pkg) name version integrity url;
      tarballPath = "${pkg.tarball}";
    })
    fetchedPackages);

  manifestFile = writeText "deno-deps-manifest.json" manifest;
in
  stdenvNoCC.mkDerivation {
    name = "deno-npm-deps";
    dontUnpack = true;
    nativeBuildInputs = [jq];

    buildPhase = ''
      runHook preBuild
      mkdir -p $out/npm/registry.npmjs.org

      # Phase 1: Extract all package tarballs into DENO_DIR cache layout
      jq -r '.[] | "\(.name)\t\(.version)\t\(.tarballPath)"' ${manifestFile} \
        | while IFS=$'\t' read -r name version tarball; do
        dir="$out/npm/registry.npmjs.org/$name/$version"
        mkdir -p "$dir"
        tar xzf "$tarball" -C "$dir" --strip-components=1
      done

      # Phase 2: a registry.json per package, which Deno needs to resolve npm,
      # and above all for packages carrying bin or scripts.
      jq -r '.[].name' ${manifestFile} | sort -u | while read -r name; do
        pkg_dir="$out/npm/registry.npmjs.org/$name"
        registry_json="$pkg_dir/registry.json"

        versions_json='{}'
        for version_dir in "$pkg_dir"/*/; do
          [ -d "$version_dir" ] || continue
          version=$(basename "$version_dir")
          pkg_json="$version_dir/package.json"
          [ -f "$pkg_json" ] || continue

          # Extract bin and scripts from the extracted package.json
          version_entry=$(jq -c '{
            version: .version,
            dist: {},
            bin: (.bin // null),
            scripts: (.scripts // null)
          } | with_entries(select(.value != null))' "$pkg_json")

          # Patch in the correct integrity and tarball URL from our manifest
          integrity=$(jq -r --arg name "$name" --arg ver "$version" \
            '.[] | select(.name == $name and .version == $ver) | .integrity' ${manifestFile})
          url=$(jq -r --arg name "$name" --arg ver "$version" \
            '.[] | select(.name == $name and .version == $ver) | .url' ${manifestFile})

          if [ -n "$integrity" ] && [ "$integrity" != "null" ]; then
            version_entry=$(echo "$version_entry" | jq -c \
              --arg integrity "$integrity" --arg tarball "$url" \
              '.dist.integrity = $integrity | .dist.tarball = $tarball')
          fi

          versions_json=$(echo "$versions_json" | jq -c \
            --arg ver "$version" --argjson entry "$version_entry" \
            '. + {($ver): $entry}')
        done

        jq -n --arg name "$name" --argjson versions "$versions_json" \
          '{name: $name, versions: $versions}' > "$registry_json"
      done

      runHook postBuild
    '';

    installPhase = "true";
  }
