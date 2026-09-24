{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  makeWrapper,
  libdrm,
  libxkbcommon,
  wayland,
  fontconfig,
  expat,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "cosmic-nightlight";
  version = "0.5.1";

  src = fetchFromGitHub {
    owner = "cosmic-nightlight";
    repo = "cosmic-nightlight";
    tag = "v${finalAttrs.version}";
    hash = "sha256-CbCABZO+5UgdCG5DolJGg5BShRHbWMZfR7jNUnYFAgU=";
  };

  cargoHash = "sha256-0A+KBqEU+brqmtZ1hfwcmRmvuetkVPuO1q5n5P8KGHc=";

  # The helper and rule paths are compile-time constants aimed at /usr, and the
  # rule whitelists the helper by absolute path, so all three have to agree on
  # the store path. The rule-location probe learns the NixOS spelling too, or
  # the app forever offers a "setup" that is already done.
  postPatch = ''
    substituteInPlace crates/cosmic-nightlight/src/backend.rs \
      --replace-fail '"/usr/bin/cosmic-nightlight-helper"' '"${placeholder "out"}/bin/cosmic-nightlight-helper"' \
      --replace-fail '"/etc/polkit-1/rules.d/49-cosmic-nightlight.rules"' '"/run/current-system/sw/share/polkit-1/rules.d/49-cosmic-nightlight.rules"'
    substituteInPlace polkit/49-cosmic-nightlight.rules \
      --replace-fail '"/usr/bin/cosmic-nightlight-helper"' '"${placeholder "out"}/bin/cosmic-nightlight-helper"'
    substituteInPlace systemd/cosmic-nightlight.service \
      --replace-fail /usr/local/bin/cosmic-nightlight ${placeholder "out"}/bin/cosmic-nightlight
  '';

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    libdrm
    libxkbcommon
    wayland
    fontconfig
    expat
  ];

  postInstall = ''
    wrapProgram $out/bin/cosmic-nightlight \
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [libxkbcommon wayland]}
    install -Dm644 polkit/49-cosmic-nightlight.rules \
      $out/share/polkit-1/rules.d/49-cosmic-nightlight.rules
    install -Dm644 data/io.github.cosmic_nightlight.desktop \
      $out/share/applications/io.github.cosmic_nightlight.desktop
    install -Dm644 data/io.github.cosmic_nightlight.settings.desktop \
      $out/share/applications/io.github.cosmic_nightlight.settings.desktop
    install -Dm644 data/io.github.cosmic_nightlight.metainfo.xml \
      $out/share/metainfo/io.github.cosmic_nightlight.metainfo.xml
    install -Dm644 systemd/cosmic-nightlight.service \
      $out/lib/systemd/user/cosmic-nightlight.service
    install -Dm644 data/icons/hicolor/128x128/apps/io.github.cosmic_nightlight.png \
      $out/share/icons/hicolor/128x128/apps/io.github.cosmic_nightlight.png
    install -Dm644 data/icons/hicolor/scalable/apps/io.github.cosmic_nightlight.svg \
      $out/share/icons/hicolor/scalable/apps/io.github.cosmic_nightlight.svg
  '';

  meta = {
    description = "Night-light / gamma utility for the COSMIC desktop, via DRM/KMS and a polkit helper";
    homepage = "https://github.com/cosmic-nightlight/cosmic-nightlight";
    license = lib.licenses.mpl20;
    maintainers = with lib.maintainers; [overby-me];
    platforms = lib.platforms.linux;
    mainProgram = "cosmic-nightlight";
  };
})
