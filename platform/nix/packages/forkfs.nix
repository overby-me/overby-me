{
  lib,
  rustPlatform,
  fetchFromGitHub,
}:
rustPlatform.buildRustPackage rec {
  pname = "forkfs";
  version = "0.2.8";

  src = fetchFromGitHub {
    owner = "SUPERCILEX";
    repo = "forkfs";
    rev = version;
    hash = "sha256-WrJdk/M40xxzQygP9M1PaMG4jHSHw1iI6AXDJLdnFvs=";
  };

  cargoHash = "sha256-Dwzgm42BUiP2VxMVCtke45Ah5TV+Ip+4zK1PNv5B3hU=";

  # Stable needs three things nightly did not: the edition that makes
  # let-chains legal, the feature gates gone (a stabilised feature named
  # in #![feature] still refuses to compile on stable), and the two
  # dir_entry_ext2 call sites on the owned file_name().
  postPatch = ''
    substituteInPlace Cargo.toml \
      --replace 'edition = "2021"' 'edition = "2024"'
    substituteInPlace src/lib.rs \
      --replace '#![feature(let_chains)]' "" \
      --replace '#![feature(dir_entry_ext2)]' ""
    substituteInPlace src/sessions.rs \
      --replace '    os::unix::fs::DirEntryExt2,' "" \
      --replace 'let name = entry.file_name_ref().to_string_lossy();' 'let file_name = entry.file_name(); let name = file_name.to_string_lossy();' \
      --replace 'let mut session = TmpPath::new(&mut sessions_dir, entry.file_name_ref());' 'let file_name = entry.file_name(); let mut session = TmpPath::new(&mut sessions_dir, &file_name);'
  '';

  doCheck = false;

  meta = {
    description = "ForkFS allows you to sandbox a process's changes to your file system";
    homepage = "https://github.com/SUPERCILEX/forkfs";
    license = lib.licenses.asl20;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "forkfs";
  };
}
