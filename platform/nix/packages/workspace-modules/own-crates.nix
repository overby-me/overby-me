# The tree's own crates that host and home modules reach for by name.
#
# In the monorepo these names are defined by the projects themselves and the
# self-overlay makes them ambient; a consumer folding only this workspace has
# neither. mkDefault is the whole trick: the project definitions win at
# normal priority where they exist, and these builds from the published
# sources apply everywhere else. fetchGit with a rev needs no hash, and
# importCargoLock takes its hashes from the lockfile, so a bump is one rev.
{lib, ...}: {
  packages = {
    wclip = lib.mkDefault ({rustPlatform, ...}: let
      # Rev-pinned and hash-free is the point; the pkgs fetchers demand a
      # narHash this module deliberately avoids maintaining.
      # ast-grep-ignore: nix-prefer-lib
      src = builtins.fetchGit {
        url = "https://tangled.org/overby.me/wclip";
        rev = "7b56b39f28680e92f70883362f2a535be6b7e2b6";
      };
    in
      rustPlatform.buildRustPackage {
        pname = "wclip";
        version = "unstable";
        inherit src;
        cargoLock.lockFile = "${src}/Cargo.lock";
        meta.description = "An xclip-style Wayland clipboard tool written in Rust";
        meta.mainProgram = "wclip";
      });

    nushell-plugin-tramp = lib.mkDefault ({rustPlatform, ...}: let
      # Rev-pinned and hash-free is the point; the pkgs fetchers demand a
      # narHash this module deliberately avoids maintaining.
      # ast-grep-ignore: nix-prefer-lib
      src = builtins.fetchGit {
        url = "https://tangled.org/overby.me/nushell-plugin-tramp";
        rev = "0f286d7eb341e726e572fc01bbc7602465edd307";
      };
    in
      rustPlatform.buildRustPackage {
        pname = "nushell-plugin-tramp";
        version = "unstable";
        inherit src;
        cargoLock.lockFile = "${src}/Cargo.lock";
        doCheck = false;
        meta.description = "A TRAMP-inspired remote filesystem plugin for Nushell";
      });
  };
}
