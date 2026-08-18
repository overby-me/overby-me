# Pinned sources for the Euro-Office from-source build (see ./PLAN.md §2).
#
# All repos are submodules of Euro-Office/DesktopEditors. They are pinned to the
# commits referenced by the super-repo's `main` branch state captured on
# 2026-06-29 (the `v9.4.0` *tag* predates several submodules being added, so the
# branch tip is used to get a complete, consistent set).
#
# `hash` is the *unpacked* tree hash (what fetchFromGitHub/fetchzip yields).
# Entries still marked `lib.fakeHash` are filled in when their build phase lands;
# Nix prints the real hash on the first fetch.
{lib}: {
  version = "9.4.0";
  buildNumber = "nix.1";

  # Pinned to the exact build whose libcef_dll wrapper desktop-sdk vendors
  # (CEF_VERSION 109.1.18+gf1c41e4+chromium-109.0.5414.120). The wrapper is
  # ABI-locked to that branch, so the framework must match it exactly.
  #
  # nixpkgs' `cef-binary` cannot be used: it tracks a far newer branch and is
  # Linux-only, throwing on aarch64-darwin. We fetch the official upstream
  # prebuilt from the Spotify CDN instead (see ./cef.nix). This is the only
  # binaryNativeCode artifact in the build — and it is genuine upstream CEF, not
  # an ONLYOFFICE binary.
  cef = {
    version = "109.1.18+gf1c41e4+chromium-109.0.5414.120";
    # Split form consumed by the Linux path (nixpkgs `cef-binary.override`,
    # which assembles the CDN URL from these three parts + the platform tag).
    cefVersion = "109.1.18";
    gitRevision = "f1c41e4";
    chromiumVersion = "109.0.5414.120";
    macosarm64 = {
      # URL-encoded '+' as %2B; see cef.nix for the full URL assembly.
      hash = "sha256-9z6O9aUVFYY5BMLPpAWcwmqbM/CVFanrnMrDTqMVOOk=";
    };
    # Linux prebuilt (the `_minimal` tarball nixpkgs' cef-binary fetches). Filled
    # in on first build; Nix prints the real hash.
    linux64 = {
      hash = "sha256-ukuco3y3d7JW/fXhjJvbxm6/BgNn6FQjmGdrQru8Dlk=";
    };
    linuxarm64 = {
      hash = lib.fakeHash;
    };
  };

  repos = {
    core = {
      owner = "Euro-Office";
      repo = "core";
      rev = "ec2bd75f7e4d91856afcbd9996cafaff42ad7ea3";
      hash = "sha256-vXOrcdfsYd8MQ3+BGjR3BJUsA+3PMktWSfE83hpLjhs="; # phase 4 (candidate; nix will correct on fetch)
    };
    desktop-sdk = {
      owner = "Euro-Office";
      repo = "desktop-sdk";
      rev = "00827afd78f555d4d8f2d5181aba319f8cd684f5";
      hash = "sha256-DSkwqdG8bRqU4lsknJ086e1YaQX9ZQROeNLa/2k+S1I="; # phase 6
    };
    desktop-apps = {
      owner = "Euro-Office";
      repo = "desktop-apps";
      rev = "de61732967e7a639b913146054a2a14df8467b93";
      hash = "sha256-9/jIpv9YCrS80ywI6Q+0QNp8NDXGmAI7xRyHIdOR5Sk="; # phase 7
    };
    sdkjs = {
      owner = "Euro-Office";
      repo = "sdkjs";
      rev = "bf4a2db383f2dc9712c328e8704d3c58abb6a93e";
      hash = lib.fakeHash; # phase 5
    };
    sdkjs-forms = {
      owner = "Euro-Office";
      repo = "sdkjs-forms";
      rev = "e7859d1d421f14848a3114d7a22e55608d9c98f7";
      hash = lib.fakeHash; # phase 5
    };
    web-apps = {
      owner = "Euro-Office";
      repo = "web-apps";
      rev = "c6368d5eaa92b2178da722cd4792946403b4c42f";
      hash = lib.fakeHash; # phase 5
    };

    # --- vendored third-party source deps that `core` compiles in-tree ---
    # Pinned to the exact commits upstream's Common/3dParty/*/nc-fetch.sh use,
    # so the in-tree patches (also under Common/3dParty/...) apply. We fetch
    # them here instead of running build_3rdparty.py (network during configure).
    katana-parser = {
      owner = "jasenhuang";
      repo = "katana-parser";
      rev = "be6df458d4540eee375c513958dcb862a391cdd1";
      hash = "sha256-SYJFLtrg8raGyr3zQIEzZDjHDmMmt+K0po3viipZW5c=";
    };
    gumbo-parser = {
      owner = "google";
      repo = "gumbo-parser";
      rev = "aa91b27b02c0c80c482e24348a457ed7c3c088e0";
      hash = "sha256-+607iXJxeWKoCwb490pp3mqRZ1fWzxec0tJOEFeHoCs=";
    };
    harfbuzz = {
      owner = "harfbuzz";
      repo = "harfbuzz";
      rev = "894a1f72ee93a1fd8dc1d9218cb3fd8f048be29a";
      hash = "sha256-sO0Kd2wAbMm+Auf7tXsDNal7hqND8iwkb0M/9WWt9sI=";
    };
    brotli = {
      owner = "google";
      repo = "brotli";
      rev = "a47d7475063eb223c87632eed806c0070e70da29";
      hash = "sha256-ns5/4ziE186iqjncaPxl6ffi1prfedV3a0/OUOEoSf8=";
    };
    # Upstream nc-fetch.sh clones hyphen's default branch unpinned; we pin the
    # latest release tag (v2.8.9) for reproducibility. graphics #includes its
    # .c/.h directly via "hyphen/..." relative to EO_CORE_3RD_PARTY_INSTALL_DIR.
    hyphen = {
      owner = "hunspell";
      repo = "hyphen";
      rev = "22de76351caba7eed96fd9c44aa26c02352b871b"; # v2.8.9
      hash = "sha256-F7PJQjEiE5t5i1gi5B8wzrwAJQl8FWzopRA8uDsaZBc=";
    };
    # socket.io-client-cpp + its recursively-pinned submodules, as upstream's
    # Common/3dParty/socketio/nc-fetch.sh checks them out. kernel_network
    # compiles the socket.io C++ client (with + without TLS) in-tree.
    socketio = {
      owner = "socketio";
      repo = "socket.io-client-cpp";
      rev = "da779141a7379cc30c870d48295033bc16a23c66";
      hash = "sha256-75Gth0QLMkeKTbvmVCGW15LSgItiWlO1w0p36+W62tk=";
    };
    asio = {
      owner = "chriskohlhoff";
      repo = "asio";
      rev = "230c0d2ae035c5ce1292233fcab03cea0d341264";
      hash = "sha256-HOqjX3afK6vCa1j48nRZgFgpms1hedEW8X7DRr60o/E=";
    };
    websocketpp = {
      owner = "zaphoyd";
      repo = "websocketpp";
      rev = "56123c87598f8b1dd471be83ca841ceae07f95ba";
      hash = "sha256-9fIwouthv2GcmBe/UPvV7Xn9P2o0Kmn2hCI4jCh0hPM=";
    };
    rapidjson = {
      owner = "Tencent";
      repo = "rapidjson";
      rev = "a36110e11874bcf35af854940e0ce910c19a8b49";
      hash = "sha256-uZlgmFNxdxoSeU5CCOGwYFPJ9l+MkHl7mIYXWR6Gu+g=";
    };
    # md4c (markdown parser) — HtmlFile2 compiles it in-tree from the md dir.
    md4c = {
      owner = "mity";
      repo = "md4c";
      rev = "481fbfbdf72daab2912380d62bb5f2187d438408";
      hash = "sha256-zhInM3R0CJUqnzh6wRxMwlUdErovplbZQ5IwXe9XzZ4=";
    };
    # Apple iWork (Pages/Numbers/Keynote) import stack — the libwpd family that
    # Apple/IWorkFile compiles in-tree. Pins from Common/3dParty/apple/fetch.sh.
    glm = {
      owner = "g-truc";
      repo = "glm";
      rev = "33b4a621a697a305bc3a7610d290677b96beb181";
      hash = "sha256-wwGI17vlQzL/x1O0ANr5+KgU1ETnATpLw3njpKfjnKQ=";
    };
    mdds = {
      owner = "kohei-us";
      repo = "mdds";
      rev = "0783158939c6ce4b0b1b89e345ab983ccb0f0ad0";
      hash = "sha256-HMGMxMRO6SadisUjZ0ZNBGQqksNDFkEh3yaQGet9rc0=";
    };
    librevenge = {
      owner = "Distrotech";
      repo = "librevenge";
      rev = "becd044b519ab83893ad6398e3cbb499a7f0aaf4";
      hash = "sha256-2YRxuMYzKvvQHiwXH08VX6GRkdXnY7q05SL05Vbn0Vs=";
    };
    libodfgen = {
      owner = "Distrotech";
      repo = "libodfgen";
      rev = "8ef8c171ebe3c5daebdce80ee422cf7bb96aa3bc";
      hash = "sha256-Bv/smZFmZn4PEAcOlXD2Z4k96CK7A7YGDHFDsqZpuiE=";
    };
    libetonyek = {
      owner = "LibreOffice";
      repo = "libetonyek";
      rev = "cb396b4a9453a457469b62a740d8fb933c9442c3";
      hash = "sha256-nFYI7PbcLyquhAWVGkjNLHp+tymv+Pzvfa5DNPeqZiw=";
    };

    # --- font / dictionary / template data (phase 2, hashes verified) ---
    core-fonts = {
      owner = "Euro-Office";
      repo = "core-fonts";
      rev = "2de90688e07e2442a5fffc434fdf0a3d212c1f42";
      hash = "sha256-FXE9YUwCleAP4T9YB30sP+gSDPqHanL/4K3IcCgYQ74=";
    };
    dictionaries = {
      owner = "Euro-Office";
      repo = "dictionaries";
      rev = "08d46f8631b82af00f83b28657c54ed1aaf9e3fc";
      hash = "sha256-MqPKb6wnxRVKeQwnpdi295NniP6y6iEes2JwYglbKm8=";
    };
    document-templates = {
      owner = "Euro-Office";
      repo = "document-templates";
      rev = "d697ab3d75dbec104e4a99f47852398752cb2ace";
      hash = "sha256-+52+MK/8DARJrQRbIpN5nk3j3J9cy6Wd1FDMnCVZKRE=";
    };
  };
}
