# Euro-Office: pure (from-source) build — scoping & plan

This document maps the full derivation graph required to build **Euro-Office
DesktopEditors** from source in Nix, for both `x86_64-linux` and
`aarch64-darwin`, replacing the upstream Docker/Xcode orchestration with native
Nix derivations.

It is the plan referenced by the incremental package work under this directory.
Read it before touching any of the sub-derivations.

> **Status (aarch64-darwin, 2026-06-30):** everything builds from source on this
> machine EXCEPT the final storyboard compile (which needs Xcode's `ibtool`).
> Buildable attributes:
>
> - `.#euro-office.data` (phase 2) — fonts/dictionaries/templates.
> - `.#euro-office.desktop-common` (phase 5) — the official EO editors JS/WASM
>   payload (the only `binaryBytecode` piece).
> - `.#euro-office.core` (phase 4) — the `x2t` converter engine + all format
>   libraries, native arm64, V8-free.
> - `.#euro-office.cef` — Chromium Embedded Framework branch 109, a pinned
>   upstream prebuilt (`binaryNativeCode`, genuine CEF, NOT an ONLYOFFICE blob).
> - `.#euro-office.desktop-sdk` (phase 6) — `ascdocumentscore` (the CEF client
>   library the macOS app links), built from source against nixpkgs Qt 5.15 +
>   the CEF package + core. Links the CEF framework via `@executable_path`.
> - `.#euro-office.app` (phase 7) — assembles `Euro-Office.app` from source
>   (compiles the 72 Obj-C++ sources + asset catalog with clang/`actool`,
>   embeds the frameworks + the CEF `editors_helper.app`, ad-hoc code-signs).
>
> **The one remaining blocker:** the app's UI is defined in macOS storyboards
> (`Main.storyboard`, 1582 lines). Compiling `.storyboard` → `.storyboardc`
> requires Apple's `ibtool`, which ships ONLY with full Xcode (not the
> CommandLineTools on this host, and no working open-source equivalent exists
> for *macOS* storyboards — `davidquesada/ibtool` is iOS-only). `app.nix`
> already calls `/usr/bin/ibtool` with a fallback, so installing Xcode is the
> last mile to a launchable app. We refuse to ship an ONLYOFFICE binary.
>
> On Linux the from-source path remains viable (§10 option 1); the Qt host there
> is `qtascdocumentscore`/QCefView (the Linux/Windows path, not used on macOS).

## Key findings (verified against the v9.4.0 `main` sources)

These materially lower the risk versus the worst-case in §1:

1. **The desktop app does NOT link V8.** `desktop-apps/win-linux/CMakeLists.txt`
   links only CEF + Qt + `core` libs (`ascdocumentscore`, `kernel`, `graphics`,
   `PdfFile`, …) + openssl/hunspell/icu. V8 is used solely by `core`'s
   server-side `DoctRenderer` (JS document conversion), which the desktop editor
   doesn't need (editing runs as `sdkjs` JS *inside CEF/Chromium*). So **V8 can
   likely be dropped** — removing the single hardest dependency. (Confirm the
   `core` libs we link don't transitively pull `DoctRenderer`/V8; if they do,
   build a V8-less converter or stub it.)
2. **`desktop-common` is an assembly, not a build.** It just lays out the
   *already-built* `sdkjs`, `web-apps`, and `desktop-js` outputs plus static
   copies (converter config, templates, dictionaries, fonts). So phase 5
   decomposes into independent JS builds + a final copy step.
3. **JS builds are Node 20 + grunt + Java closure-compiler.** `web-apps` builds
   standalone (`THEME=euro-office grunt`, no WASM dep) — the cleanest first
   real compile. `sdkjs` additionally consumes `core-wasm`.
4. **`core-wasm` = `emcmake cmake` on the `core` project** (emsdk 5.0.4 →
   `pkgs.emscripten`). Produces the pdf engine / zlib / hash / spell / libfont
   WASM that `sdkjs` embeds. Tractable, independent of CEF/Qt.
5. **Euro-Office branding lives in `web-apps` (`THEME=euro-office`)** and via the
   CMake `ABOUT_PAGE_APP_NAME`/`COMPANY_NAME` args — no source edits needed.
6. **Euro-Office's own CI publishes the assembled `desktop-common` JS/WASM
   payload publicly** at `ghcr.io/euro-office/desktop-common:<super-repo-sha>`
   (anonymously pullable; amd64 image, content is arch-independent). This is a
   genuine Euro-Office artifact (NOT ONLYOFFICE).

### Strategic decision for phase 5 (the JS/WASM payload)

There are two ways to obtain `desktop-common`:

- **(A) Pure rebuild** — `web-apps` (Node20+grunt, many native node-gyp + Java
  closure deps), `sdkjs`+`sdkjs-forms` (Node+grunt consuming `core-wasm`),
  `core-wasm` (Emscripten), then assemble. Fully from-source but a large, fiddly
  Node toolchain effort (`file:` local deps, imagemin/spritesmith native
  modules, requirejs optimizer).
- **(B) Fetch the official payload** — `dockerTools.pullImage` the public
  Euro-Office `desktop-common` image and extract `/editors`, `/converter`,
  `/fonts`, etc. Genuine Euro-Office output, dramatically less work, but a
  prebuilt blob for the JS/WASM layer (`sourceProvenance` gains `binaryBytecode`).

**Decision:** pursue **(B) first** to get a *running editor* end-to-end sooner
(it only leaves the native C++ app — phases 3/4/6/7 — to build from source), then
optionally revisit **(A)** to make the JS layer fully from-source. (B) keeps the
result 100% Euro-Office. The native shell (CEF/Qt app) is still built from source
either way, so the bulk of "pure build" value is preserved.

> Note: `desktop-common` images are tagged by the **super-repo** commit SHA, not
> submodule SHAs, and not every commit has an image. Pin to a SHA that has one
> (e.g. `0bd0e7a`) and that matches the submodule revs in `sources.nix`.

---

## 1. Why this is hard (and why nixpkgs ships the .deb)

Euro-Office is a fork of ONLYOFFICE DesktopEditors. A full build is a
**multi-repository, multi-toolchain** affair. Upstream orchestrates it with:

- **Linux:** `docker buildx bake` (Ubuntu 22.04 base, vcpkg, clang-13, ccache).
- **Windows:** `build.ps1` (MSVC, sccache).
- **macOS:** `desktop-apps/macos/*.xcodeproj` driven by **fastlane/Xcode**
  (cannot run in the Nix sandbox; needs Xcode + signing).

The native app links against **CEF** (Chromium Embedded Framework) and embeds
**V8**, plus a from-source **Qt5**. CEF + V8 are the two hardest pieces to make
hermetic in Nix.

The realistic Nix target is therefore **`x86_64-linux` first** (mirrors the
clean, containerizable upstream Linux build). `aarch64-darwin` is a stretch goal
documented in §7 — it likely cannot be made fully hermetic without Xcode.

---

## 2. Source repositories (submodules of `Euro-Office/DesktopEditors`)

All on the `main`/`master` default branch of the `Euro-Office` org. Pin each to
the commit referenced by the `DesktopEditors` super-repo tag we target
(currently `v9.4.0`). Use `fetchFromGitHub` per submodule (NOT recursive clone)
so each is independently cacheable.

| Submodule           | Repo                              | Role                                            |
| ------------------- | --------------------------------- | ----------------------------------------------- |
| `core`              | `Euro-Office/core`                | C++ document engine + `Common/3dParty` recipes  |
| `desktop-sdk`       | `Euro-Office/desktop-sdk`         | C++ SDK bridging core ↔ app (links CEF)         |
| `desktop-apps`      | `Euro-Office/desktop-apps`        | the app shell; `win-linux/CMakeLists.txt` (build definition), `macos/` (Xcode) |
| `sdkjs`             | `Euro-Office/sdkjs`               | JS SDK for the editors                           |
| `sdkjs-forms`       | `Euro-Office/sdkjs-forms`         | forms editor JS                                  |
| `web-apps`          | `Euro-Office/web-apps`            | editor web frontends (HTML/JS)                  |
| `core-fonts`        | `Euro-Office/core-fonts`          | bundled fonts (input to `allfontsgen`)          |
| `dictionaries`      | `Euro-Office/dictionaries`        | spellcheck dictionaries                          |
| `document-templates`| `Euro-Office/document-templates`  | new-document templates                          |

A small Nix attrset (`sources.nix`) should hold `{ owner, repo, rev, hash }` for
each, so version bumps touch one file. Consider generating it with a script that
reads the super-repo's submodule gitlinks for a given tag.

---

## 3. Third-party native dependencies

Upstream resolves these two ways. We must map each to nixpkgs or a dedicated
fetch+build derivation.

### 3a. vcpkg manifest (`core/vcpkg.json`)

Currently tiny: **`hunspell` (1.7.2)** (+ `gtest` for the test feature). Easy —
use `pkgs.hunspell` and drop the vcpkg toolchain entirely. The CMake projects
take `-DCMAKE_TOOLCHAIN_FILE=.../vcpkg.cmake`; we replace that with normal
`find_package` against nixpkgs inputs (may need small CMake patches).

### 3a-bis. How `core` consumes third-party libs (verified in `common.cmake`)

This is the **central obstacle**, now fully understood:

- `core/common.cmake` runs `Common/3dParty/build_3rdparty.py` **during CMake
  configure** (guarded by `if(NOT THIRD_PARTY_PREPARED)`), with
  `--except=openssl-hash,icu-wasm` (i.e. it builds boost, icu, icu-desktop, v8,
  openssl, cef, **qt** from source / fetches prebuilts from a private Nextcloud).
  Running a network+compile step mid-configure is incompatible with the Nix
  sandbox.
- **Escape hatch:** pass `-DTHIRD_PARTY_PREPARED=TRUE` and **pre-populate**
  `EO_CORE_3RD_PARTY_INSTALL_DIR` with the exact expected layout. Then CMake
  skips `build_3rdparty.py` entirely and just `find_package`s / links the files
  we placed. This is the only viable Nix path for `core`.
- Expected install-dir layout (from `common.cmake`):
  - `boost/`            — static libs + `include/` (`Boost_USE_STATIC_LIBS ON`,
    components: system filesystem regex date_time)
  - `icu/lib/libicuuc.so.74`, `libicudata.so.74` (**ICU 74**)
  - `icu-desktop/lib/libicuuc.so.60`, `libicudata.so.60`, `libicui18n.so.60`
    (**ICU 60** — a *different, older* ICU just for desktop!)
  - `openssl/lib/{libssl.a,libcrypto.a}`
  - `v8/libv8_monolith.a` (only if building x2t/docbuilder)
  - `qt/qt/lib/cmake/Qt5` + headers/libs (**Qt 5.9.9** — see below)
  - `cef/cmake/FindCEF.cmake` + `Release/` + `Resources/` (**CEF branch 5414**)

### Pinned third-party versions (from `Common/3dParty/*/vcpkg.json`)

| Dep | Upstream pin | nixpkgs reality | Risk |
| --- | --- | --- | --- |
| **Qt** | **5.9.9** (2019) | only Qt **5.15.x** / 6.x | 🔴 major: 5.9.9 unavailable; must try 5.15 and hope the app compiles, or build 5.9.9 from source |
| **CEF** | branch **5414** (~Chromium 110) | `cef-binary` tracks specific versions | 🔴 major: must find/pin a CEF 5414 binary matching the API the code uses |
| **V8** | **8.9** | `v8` is much newer | 🟡 droppable for desktop (only x2t/docbuilder need it) |
| ICU | 74 (general) + **60** (desktop) | `icu` is single-version | 🟠 need two ICU versions incl. EOL ICU 60 |
| boost, openssl, curl, hunspell, harfbuzz, brotli, libheif, hyphen, glew | recent | available | 🟢 |

**Reality check:** the desktop build hard-depends on **Qt 5.9.9** and **CEF
5414**, neither of which is a drop-in from nixpkgs. Making `core` + the app build
from source therefore requires either (a) building Qt 5.9.9 and obtaining CEF
5414 ourselves (large), or (b) patching the code to build against Qt 5.15 + a
nixpkgs-available CEF (uncertain API compatibility). Both are high-effort,
high-risk. This is *the* gate; see the decision note in §10.

### 3b. `core/Common/3dParty` (built by `build_3rdparty.py`)

This is the hard set. Each entry is a fetch-and-build recipe; we replace each
with a nixpkgs package or a pinned binary/source derivation:

| 3dParty entry      | Nix strategy                                                         |
| ------------------ | ------------------------------------------------------------------- |
| `qt`               | **Custom Qt5** upstream. Try `pkgs.qt5` first; fall back to a pinned source build if ABI/patches matter. ⚠️ biggest risk after CEF. |
| `cef`              | **CEF binary.** Use nixpkgs `cef-binary` (patched prebuilt) pinned to the CEF version upstream expects. ⚠️ highest risk. |
| `v8` / `v8_89`     | V8 9.x — needs clang-13 + `gn`/`ninja`. Try `pkgs.v8` pin; else build the pinned source. ⚠️ high risk (clang-13 hard pin). |
| `boost`            | `pkgs.boost` (match version).                                       |
| `icu` / `icu-desktop` / `icu-wasm` | `pkgs.icu` (desktop); WASM icu built with Emscripten. |
| `openssl`          | `pkgs.openssl` (verify version/ABI).                                |
| `curl`             | `pkgs.curl`.                                                        |
| `cryptopp`         | `pkgs.cryptopp`.                                                    |
| `harfbuzz`         | `pkgs.harfbuzz`.                                                    |
| `brotli`           | `pkgs.brotli`.                                                      |
| `heif`             | `pkgs.libheif`.                                                     |
| `hunspell`         | `pkgs.hunspell` (also the vcpkg dep).                               |
| `hyphen`           | `pkgs.hyphen`.                                                      |
| `glew`             | `pkgs.glew`.                                                        |
| `libvlc`           | `pkgs.libvlc` / `pkgs.vlc` (media viewer; may be droppable).        |
| `ixwebsocket`, `socketio`, `socketrocket` | nixpkgs if present, else small source builds. |
| `pole`, `md`, `html`, `misc`, `openssl-hash` | small vendored helpers — build in-tree. |
| `apple`            | macOS-only bits (see §7).                                           |

**Decision:** do NOT port `build_3rdparty.py`. Instead, configure the CMake
projects to consume nixpkgs-provided libraries. Expect to patch
`core/Common/3dParty/*/*.pri`/cmake includes and the top-level CMake to find
system libs instead of `core/build/<arch>/...` prebuilt paths.

---

## 4. The build, in phases (derivation graph)

```text
fonts (core-fonts, dictionaries, document-templates)   [trivial: just copy]
        │
core ───┤  (C++ engine; needs Qt, boost, icu, openssl, v8, cryptopp, hunspell…)
        │      output: converter/*, x2t, allfontsgen, allthemesgen, libs
        │
desktop-sdk ── (links CEF + core)            output: SDK libs/headers
        │
desktop-common ── (sdkjs + sdkjs-forms + web-apps; Node + Grunt/Webpack;
        │          core WASM via Emscripten)   output: editors web payload
        │
desktop-apps (win-linux/CMakeLists.txt) ── (Qt app; links CEF + desktop-sdk)
        │                                     output: DesktopEditors binary
        ▼
bundle ── overlay desktop-common onto installed app tree, run allfontsgen +
          allthemesgen, assemble /opt-style tree, wrapper script (LD_PRELOAD
          libcef.so on Linux)
        ▼
package ── Linux: wrap in FHS env / install desktop file + icons
           Darwin: assemble .app bundle (see §7)
```

### Phase order (each is a separate `.nix` here)

1. **`sources.nix`** — pinned `{owner,repo,rev,hash}` for all §2 repos.
2. **`fonts.nix`** — `core-fonts` + `dictionaries` + `document-templates`
   (pure copy derivations; no compilation). Lowest risk; do first to validate
   the wiring and `callPackage` plumbing.
3. **`third-party.nix`** — resolve §3 to nixpkgs; produce the set of buildInputs
   and any patched cmake find-modules. Spike CEF + Qt + V8 **here** before
   committing to the rest (these gate everything).
4. **`core.nix`** — build `Euro-Office/core` against phase 3. Produces
   `x2t`, `allfontsgen`, `allthemesgen`, converter libs.
5. **`desktop-common.nix`** — Node/Emscripten build of the web+WASM payload
   (`sdkjs`, `sdkjs-forms`, `web-apps`). Uses core's WASM build.
   Heaviest JS toolchain piece; can proceed in parallel with phase 4.
6. **`desktop-sdk.nix`** — build the SDK (links CEF + core).
7. **`desktop-app.nix`** — `cmake` the `desktop-apps/win-linux` project against
   phases 4+6; produce the `DesktopEditors` binary. Honour `ABOUT_PAGE_APP_NAME`,
   `PRODUCT_VERSION`, `BUILD_NUMBER`, `COMPANY_NAME=Euro-Office`.
8. **`bundle.nix`** — overlay payload, run `allfontsgen`/`allthemesgen`,
   strip the generators, write the start script.
9. **`default.nix`** — platform gate: Linux → FHS-wrapped bundle + desktop
   file/icons (reuse logic from the old `.deb` package); Darwin → §7.

---

## 5. Toolchain notes / gotchas

- **clang-13 hard pin** for V8 9.x. nixpkgs has `llvmPackages_13`. If V8 refuses
  newer clang, pin the V8 build's stdenv to `llvmPackages_13.stdenv`.
- **`gn` + `ninja`** needed for V8. nixpkgs has `gn` and `ninja`.
- **glibc floor:** upstream targets glibc 2.35 with static libstdc++/libgcc for
  portability. In Nix we don't need portability — link against the nixpkgs
  stdenv normally and let the closure pin glibc. Drop the static flags unless
  they're load-bearing for CEF compatibility.
- **CEF at runtime:** the app is launched via `LD_PRELOAD=libcef.so`. The FHS
  wrapper / `makeWrapper` must set `LD_LIBRARY_PATH` to the CEF dir and preload
  `libcef.so` (mirror upstream `start_desktop.sh`).
- **Qt platform:** bundled Qt is X11-only (the old package set `QT_QPA_PLATFORM=xcb`).
  Keep that unless we move to a Wayland-capable Qt.
- **No Nextcloud secret:** `build_3rdparty.py` pulls some prebuilt deps from a
  private Nextcloud with credentials. By replacing 3dParty with nixpkgs we avoid
  needing those secrets entirely — confirm nothing essential is *only* available
  from that server (notably the custom Qt; if so, build Qt from the pinned
  source in `Common/3dParty/qt`).

---

## 6. Branding

Set via CMake/env, no code changes needed:
`COMPANY_NAME=Euro-Office`, `PRODUCT_NAME="Desktop Editors"`,
`ABOUT_PAGE_APP_NAME="Euro-Office Desktop Editors"`, plus `PRODUCT_VERSION` /
`BUILD_NUMBER`. The `euro-office` brand has no extra brand overlay repo (only
`nextcloud-office` does, and it's commented out in CI), so the base
`desktop-apps` tree is the branding.

---

## 7. macOS (aarch64-darwin) — stretch goal

`desktop-apps/macos/ONLYOFFICE.xcodeproj` + `fastlane/Fastfile` drive the mac
build. Problems for a hermetic Nix build:

- **Requires Xcode** (`gym`/`xcodebuild`) — not in the Nix sandbox. Best case is
  an *impure* derivation using the host Xcode (`__noChroot` / `sandbox = false`),
  which is non-reproducible and CI-hostile.
- **Code signing + notarization** (`developer-id`, `notarize`) need an Apple
  Developer cert; skip signing for local dev (`CODE_SIGNING_ALLOWED=NO`) — the
  app will run locally but Gatekeeper will warn.
- It still consumes the **`desktop-common`** payload (built on Linux per §5) and
  a macOS build of `core` (the Xcode project expects `Vendor/ONLYOFFICE`).

**Plan for macOS:** defer until the Linux build works. Then attempt an impure
`xcodebuild`-based derivation that (a) takes the Nix-built `desktop-common`
payload, (b) builds `core` for arm64 via the same CMake project (Darwin stdenv),
(c) invokes `xcodebuild -project desktop-apps/macos/ONLYOFFICE.xcodeproj
-scheme <ASCDocumentEditor?> -configuration Release CODE_SIGNING_ALLOWED=NO`,
(d) copies the resulting `.app` into `$out/Applications`. Mark it
`sourceProvenance = fromSource` but document the impurity. If `xcodebuild`
proves intractable, leave Darwin gated with a `throw` explaining the limitation
(do NOT substitute an ONLYOFFICE binary).

---

## 8. Open questions to resolve while implementing

1. Does nixpkgs `cef-binary` match the CEF version `desktop-sdk` expects? If not,
   pin a `cef_binary_*` tarball from Spotify's CDN (as nixpkgs does) at the right
   version. **(Gates phases 6–7.)**
2. Can we use `pkgs.qt5` or must we build the custom Qt from `Common/3dParty/qt`?
   Diff the upstream Qt version/patches first.
3. Is `v8` actually required for the *desktop* build, or only for server-side
   document conversion? If the desktop app doesn't need V8, we drop the hardest
   dependency. **(Check `desktop-sdk`/`core` link lines.)**
4. What exact Node toolchain does `desktop-common` use (Grunt? Webpack? Node
   version)? Mirror it with `buildNpmPackage` / `fetchNpmDeps`.
5. Emscripten version for the WASM `core` build — pin `pkgs.emscripten`.

---

## 10. Go/no-go on the native-shell phases (3,4,6,7)

After the phase-3 spike, the situation is:

- ✅ **Data (phase 2)** and ✅ **editors payload (phase 5, official EO build)**
  are done and building. The payload already contains the *real* editors and a
  working `converter/` — i.e. all the JS/WASM that does the actual editing.
- 🔴 The **native shell** (the `DesktopEditors` CEF/Qt window that hosts the
  payload) needs `core` libs + `desktop-sdk` + the app, all gated behind
  **Qt 5.9.9 + CEF 5414** via the `THIRD_PARTY_PREPARED` pre-population.

The honest assessment: phases 3/4/6/7 are a **multi-day, high-risk** effort
dominated by the Qt 5.9.9 / CEF 5414 mismatch. Options:

1. **Pre-populate + nixpkgs Qt5.15/CEF spike** — set `THIRD_PARTY_PREPARED`,
   feed nixpkgs boost/icu/openssl, build `core` *without* desktop libs and
   *without* V8 first (smallest unit that can compile) to validate the escape
   hatch, then tackle Qt/CEF. Incremental; next concrete step.
2. **Build Qt 5.9.9 + fetch CEF 5414 from source** — faithful but very large.
3. **Stop at the payload** — ship phases 2+5 (genuinely useful, 100% EO) and
   leave the native shell as documented future work behind the honest gate.

The single most valuable *next* experiment is **option 1's first half**: prove
that `-DTHIRD_PARTY_PREPARED=TRUE` + nixpkgs libs lets `core`'s non-desktop,
V8-free libraries (`kernel`, `graphics`, `UnicodeConverter`, `PdfFile`, …)
compile. If that works, the path to the app is unblocked (modulo Qt/CEF). If it
fights us, option 3 is the pragmatic stopping point.

### Option 1 — DONE for the first core libs (reproducible Nix build) ✅

The experiment succeeded and is now codified in `core.nix` + `sources.nix` +
`patches/`. On aarch64-darwin, `nix build .#euro-office.core` builds, **from
source, against newer nixpkgs deps (ICU 76, Boost 1.89, OpenSSL 3.6)**:

- `libkernel.dylib`, `libUnicodeConverter.dylib`, `libgraphics.dylib` — all
  native arm64 Mach-O, linking nixpkgs ICU/Boost and the macOS CoreText/
  Foundation frameworks.

How the blockers fell:

- **macOS support added to `common.cmake`** faithfully to the qmake `core_mac`
  scope (Linux defines + `MAC`/`_MAC`, `@loader_path` RPATH, no GNU-ld flags) —
  `patches/0001`.
- **`EO_USE_SYSTEM_LIBS`** option resolves ICU/Boost/OpenSSL via `find_package`
  (so newer nixpkgs versions work; ICU 74 vs 76 was a non-issue) — `patches/0001`.
- **Per-target `APPLE` source branches**: `graphics` now compiles its existing
  `ApplicationFonts_mac.mm` and links CoreText — `patches/0002`.
- **Third-party source deps** (katana, gumbo, harfbuzz, brotli, hyphen) are
  pinned in `sources.nix` and assembled by `core.nix` with the upstream in-tree
  patches, replacing `build_3rdparty.py`.

The patches apply cleanly against pristine upstream `Euro-Office/core` and are
written to be submitted there. **Remaining** (grow `core.nix`'s `buildTargets`):
`PdfFile`, `DjVuFile`, `XpsFile`, `kernel_network`, the `x2t`/`allfontsgen`
tools, then the Qt 5.9.9 / CEF 5414 desktop shell (still the dominant unknown).

### Phase-3 spike outcome (2026-06-29) — decisive blockers found

Investigation settled the feasibility for **this aarch64-darwin machine**:

- **`core`'s CMake is Linux-only.** `common.cmake`'s non-MSVC branch references
  ICU as `libicuuc.so.NN`, sets `_LINUX`, and uses GNU-ld-isms
  (`-Wl,--start-group`, `-Wl,--disable-new-dtags`). There is **no `APPLE`/
  `.dylib` path** — the macOS build is the *separate Xcode project*, which in
  turn consumes a prebuilt `Vendor/ONLYOFFICE` core. So the from-source CMake
  path **cannot target macOS**; on this machine it could at best cross/emulate a
  *Linux* build, which then wouldn't run natively here anyway.
- **Version mismatches are severe.** Required vs. nixpkgs-here: Qt **5.9.9** vs
  5.15.18; ICU **74 + 60** vs 76.1; CEF **5414** (custom). None are drop-ins.
- **Net:** a pure native-shell build that is *runnable on this Mac* is **not
  attainable** without (a) the Xcode project + a prebuilt core (the path we
  rejected, since its only public input is the ONLYOFFICE binary), or (b) a
  large source port of Qt 5.9.9 + CEF 5414 + ICU 60/74 + macOS support added to
  `core`'s CMake — weeks of work with low confidence.

**Resulting decision: stop at option 3.** Ship the genuinely-useful, 100%
Euro-Office, building pieces (phases 2 + 5) and keep the native shell behind the
honest gate with this analysis recorded. On a **Linux** host, option 1 remains
the sane continuation (Linux CMake path works there); that is documented for a
future Linux-based attempt.

### Hands-on Ninja-on-darwin configure spike (2026-06-29) — concrete results

Actually ran `cmake -G Ninja` on `core` on this aarch64-darwin machine
(clang 21, cmake 4.1, ninja 1.13), with `-DTHIRD_PARTY_PREPARED=TRUE` to stub
the third-party step. Findings, in the order configure hit them:

1. ✅ **The `THIRD_PARTY_PREPARED=TRUE` escape hatch works** — `build_3rdparty.py`
   is skipped entirely; no network/compile during configure. This validates the
   core strategy for feeding nixpkgs libs.
2. ✅ **Ninja is a non-issue on darwin** — the generator drove configure fine;
   every failure below is a *content* problem, not a generator problem.
3. 🟠 **Boost finding** — the legacy `FindBoost` + `Boost_USE_STATIC_LIBS ON`
   doesn't locate nixpkgs boost cleanly (version-header layout). Fixable with
   a proper boost config package / layout; stubbed past it for the spike.
4. 🟠 **A genuine macOS code path EXISTS but is bit-rotted.**
   `doctrenderer/CMakeLists.txt:265` has an `if(APPLE)` branch (using
   **JavaScriptCore**, not V8!) with a CMake syntax bug: `target_compile_options`
   missing the `PRIVATE` keyword. So (a) core *does* contemplate macOS, (b) on
   macOS the JS engine is JavaScriptCore — **V8 may not be needed on darwin at
   all**, and (c) the macOS path clearly isn't exercised by this CMake (it's
   normally built via Xcode), so it has rotted.
5. 🔴 **Many targets have no sources on darwin.** After fixing #4, configure
   failed with `No SOURCES given to target` for `kernel`, `kernel_network`,
   `graphics`, `Fb2File`, `HtmlFile2`, `Apple/IWorkFile`, … — their source-list
   includes are gated on Linux/Windows and lack an `APPLE` branch. Each needs a
   macOS source-globbing branch added.

**Verdict (sharpened):** it's not a hard "impossible" wall — it's a **broad,
shallow port**. The macOS CMake path is present but unmaintained: dozens of
targets need `APPLE` source branches, plus the boost/icu plumbing, plus the
Qt 5.9.9 / CEF 5414 gate for the app itself. That is a real multi-day porting
effort (and arguably belongs upstream in `Euro-Office/core`, not in a packaging
repo). The spike confirms the *approach* is sound (escape hatch + nixpkgs libs +
Ninja all work) but the **darwin source coverage is the blocker**, not the
toolchain. Stop at option 3 here; revisit on Linux (where these targets already
have sources) or once upstream's macOS CMake path is maintained.

### Deep iteration: `graphics` library compiled on aarch64-darwin (2026-06-29)

Follow-up time-boxed push (option a): drive the **`graphics`** library — the
central rendering lib the desktop app links, which vendors freetype, agg, brotli,
harfbuzz, katana and pulls in `kernel` + `UnicodeConverter` — all the way to a
compile on this machine. Result: **it builds.** 77/77 translation units compiled
(graphics + harfbuzz + freetype + agg + brotli) for arm64; only the final link
failed on a single trivial libc symbol. The concrete fixes needed, in order:

1. **Boost imported targets.** nixpkgs boost (1.89) doesn't ship a separate
   `libboost_system` (header-only now) and the legacy `FindBoost` mis-detects
   it. Fix: define `Boost::{system,filesystem,regex,date_time}` imported
   targets ourselves, making `system` an INTERFACE (header-only) target.
2. **Source-form 3rd-party deps** must be pre-placed in the install dir (the
   `THIRD_PARTY_PREPARED` stub): `html/katana-parser`
   (`be6df45…`), `html/gumbo-parser` (`aa91b27…`), `socketio`
   (`da77914…`, incl. a `src_no_tls` copy), `harfbuzz` (`894a1f7…`),
   `brotli` (`a47d747…`), `hyphen` (hunspell/hyphen) — headers at
   `install/hyphen/` (the build adds `EO_CORE_3RD_PARTY_INSTALL_DIR` itself to
   the include path). The heavy `apple/` set (glm, mdds, librevenge, libodfgen,
   libetonyek) is only needed by the `Apple/IWorkFile` filter — skippable for
   the libs the app links.
3. **ICU dylib naming.** Code hard-codes `libicuuc.so.74` / `.so.60`
   (Linux). On macOS, symlink nixpkgs ICU (`libicuuc.76.dylib`, etc.) to those
   `.so.NN` names; macOS `ld` links a `.so`-named Mach-O fine. A clean fix would
   add an `APPLE` branch to `common.cmake`'s ICU path block.
4. **GNU-ld flag.** `common.cmake` set `-Wl,--disable-new-dtags`
   unconditionally; Apple ld64 rejects it (`ld: unknown option`). Fix: gate it
   behind `if(NOT APPLE)`. **(This was the predicted big blocker — it's a
   one-liner.)**
5. **CMake syntax bug** in `doctrenderer/CMakeLists.txt` `if(APPLE)` branch
   (`target_compile_options` missing `PRIVATE`). One-word fix.

Remaining at the point of stopping: link fails with `Undefined symbols: __lfind`
(libtiff `tif_dirinfo.c` expecting `lfind`; macOS has it in `<search.h>` — a
small shim/define away). i.e. `graphics` is ~1 trivial fix from a complete
arm64 dylib.

**Upgraded verdict:** the port is **mechanical and tractable per-library**, not
mysterious. `graphics` (the biggest single lib) went from "no sources" to
"compiles, 1 symbol from linking" with ~5 small, well-understood fixes + placing
pinned 3rd-party sources. The genuinely *open-ended* risk is still concentrated
in: (i) repeating this for each remaining lib the app links (`PdfFile`,
`DjVuFile`, `XpsFile`, `kernel_network`, … each may need its own `APPLE` source
branches), and (ii) **Qt 5.9.9 + CEF 5414** for the app shell itself, which the
spike did not touch and which remain the dominant unknowns. Packaging all of
this as a *reproducible* Nix derivation (vs. the manual gcroot/symlink scaffold
used here) is additional work. This belongs upstream or in a dedicated,
Linux-first effort; for this repo we record the recipe and stop.

---

## 11. Phase 6 — CEF + Qt desktop integration on macOS (2026-06-30) ✅

The native CEF/Qt integration now builds from source on aarch64-darwin.

- **CEF as its own package** (`cef.nix`): fetches the official upstream
  prebuilt Chromium Embedded Framework from the Spotify CDN, pinned to the
  EXACT branch the desktop-sdk vendored wrapper is ABI-locked to —
  `109.1.18+gf1c41e4+chromium-109.0.5414.120` (read from
  `desktop-sdk/.../src/cef/mac/include/cef_version.h`). Exposes
  `Release/Chromium Embedded Framework.framework` + headers. `binaryNativeCode`,
  genuine CEF (not an ONLYOFFICE binary). nixpkgs `cef-binary` is unusable
  (wrong branch + Linux-only).
- **`desktop-sdk.nix`** builds `libascdocumentscore.dylib` (arm64) — the CEF
  client library the macOS Cocoa app links — from source against nixpkgs Qt
  5.15 + the CEF package + the from-source `core`. It correctly links the CEF
  framework via `@executable_path/../Frameworks/...` (verified with `otool -L`).
- **Key finding:** the `00827af` desktop-sdk commit is already a working CMake
  conversion with full APPLE branches (the earlier "mac wrapper missing"
  analysis was against an older tree). macOS uses `ascdocumentscore` + the mac
  `.mm` CEF wrappers (NSView-based, `CCefViewWrapper`/`NSCefView.mm`), NOT the
  Qt `qtascdocumentscore`/QCefView host (that is the Linux/Windows path; its
  QCefView platform impl is X11/Win32-only).
- **Patches (all apply cleanly on pristine upstream, upstreamable):**
  - core `0008`: `common.cmake` BUILD_DESKTOP path resolves Qt/CEF/ICU from
    the system on `EO_USE_SYSTEM_LIBS` (nixpkgs Qt 5.15 + `CEF_ROOT`).
  - desktop-sdk `0001`: link the CEF framework from `${CEF_ROOT}/Release` on
    macOS (the legacy `Common/3dParty/cef/linux_64/build` path is Linux-only).
  - desktop-sdk `0002`: set `CORE_ROOT_DIR` before `include(common.cmake)` in
    qt_wrapper (ordering bug left QT_VERSION_MAJOR empty).
  - desktop-sdk `0003`: `boost::filesystem::wpath` → `path` (wpath removed in
    modern Boost).
  - desktop-sdk `0004`: guard X11-only qt_wrapper code with
    `defined(_LINUX) && !defined(_MAC)`.

## 9. Definition of done

### darwin (aarch64-darwin) — DONE from source ✅; runtime blocked on Xcode `ibtool`

- `nix build .#euro-office` builds (resolves to the data bundle) and
  `.#euro-office.{data,desktop-common,core,cef,desktop-sdk,app}` all build
  natively from source — verified on the aarch64-darwin machine (2026-06-30).
- The hard CEF/Qt integration (`ascdocumentscore` linking CEF 109 + nixpkgs Qt
  5.15 + from-source core) builds end-to-end as `.#euro-office.desktop-sdk`.
- `.#euro-office.app` (`app.nix`, phase 7) compiles the 72 Obj-C++ app sources
  with clang, builds the asset catalog with `actool`, assembles
  `Euro-Office.app` with the embedded frameworks (CEF, ascdocumentscore + core
  dylibs), the CEF `editors_helper.app`, the editors payload + converter + data,
  and ad-hoc code-signs it. Sparkle (the upstream auto-updater) is skipped
  (unreferenced from source). No Xcode/xcodebuild used.
- One package entry point (`pkgs/euro-office/default.nix`) gates per platform.

#### The one remaining blocker: storyboard compilation

- The app's UI lives in macOS storyboards (`Main.storyboard`, 1582 lines).
  Turning `.storyboard` → `.storyboardc` (the binary NIB bundle AppKit loads at
  launch) requires Apple's `ibtool`, which ships ONLY with full Xcode — not the
  CommandLineTools on this host. There is no working open-source `ibtool` for
  *macOS* storyboards (`davidquesada/ibtool` is iOS-only and very limited).
- `app.nix` already invokes `/usr/bin/ibtool` (with a fallback) and only warns
  when it is absent, so **installing Xcode is the last mile** to a launchable
  app — no code changes needed. After that the next runtime hurdle is debugging
  the CEF subprocess (`editors_helper.app`) startup.
- The app is too storyboard-dependent (it instantiates window/view controllers
  by storyboard identifier — `ASCTitleWindowControllerId`, the
  `ASCCommonViewController` CEF host, About/License windows) to bypass without a large
  programmatic-UI rewrite, which would diverge from upstream.
- We refuse to ship an ONLYOFFICE binary.

### linux (x86_64-linux) — remaining

- `nix build .#euro-office` produces a runnable editor on `x86_64-linux`,
  launching the Euro-Office-branded DesktopEditors with working document, sheet,
  slide and PDF editors.
- `meta.sourceProvenance = [ fromSource ]` on Linux (no prebuilt blobs except
  CEF, which is `binaryNativeCode` and called out).

---

## 12. Phase 7 — the macOS GUI application bundle (2026-06-30)

`app.nix` builds `Euro-Office.app` from `desktop-apps/macos` WITHOUT Xcode /
xcodebuild. The app is pure Objective-C++ (no Swift), so:

- All 72 app + Vendor `.mm`/`.m` sources are compiled with clang (`-fobjc-arc`,
  `-D_MAC -D_ARM_ONLY -D_PRODUCT_ONLYOFFICE`; `PFMoveApplication.m` is the one
  non-ARC source, matching the upstream Xcode per-file flag).
- Include paths cover the app's own header dirs, the `desktop-sdk` public
  headers, and the `core` source headers it references (`OfficeFileFormats.h`,
  `CertificateCommon.h`). The `core` <-> `desktop-sdk` sibling layout is
  recreated so the sdk headers' relative `../../../../core/...` includes resolve.
- It links `libascdocumentscore.dylib` + `libooxmlsignature*.dylib` from the
  `desktop-sdk` package and the system frameworks (Cocoa, WebKit, Security,
  QuickLookUI, Carbon, QuartzCore, …); the CEF framework via `-F`.
- The asset catalog is compiled with `actool`; the bundle is assembled with the
  Info.plist (Xcode build-var substitutions applied), the editors payload
  (`desktop-common`), the converter (`core`'s `x2t` + format dylibs), the
  dictionaries/fonts (`data`/`desktop-common`), the CEF framework, the core
  dylibs, and the CEF `editors_helper.app` (the single helper binary serves all
  CEF process types — it detects `--type=` at runtime). Finally everything is
  ad-hoc code-signed (`codesign -s -`), enough to launch locally on Apple
  Silicon.
- The derivation is impure (`__noChroot`): it reaches `/usr/bin/{xcrun,ibtool,
  actool,codesign}` (the host has `sandbox = false`). This is acceptable for a
  local macOS app build.

The sole gap is storyboard compilation (`ibtool`); see §9.

---

## Appendix A — `core` aarch64-darwin from-source port notes (phase 4 spike)

_(Merged from the former `darwin-core-port.md`. This records the original
2026-06-29 hand spike that proved the `core` port tractable; the authoritative
artifacts are now `core.nix`, `sources.nix` and `patches/0001`–`0008`. The spike
originally compiled `graphics`, `kernel`, `UnicodeConverter` as native arm64
dylibs with nixpkgs deps and `cmake -G Ninja`; the full non-desktop `core`
— `x2t` + all format libraries — now builds reproducibly via `core.nix`.)_

### Build invocation (spike)

```sh
cmake -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DTHIRD_PARTY_PREPARED=TRUE \
  -DEO_CORE_3RD_PARTY_INSTALL_DIR=<prepopulated install dir> \
  -DEO_CORE_OUTPUT_DIR=<out> -DEO_CORE_TOOLS_DIR=<out> \
  <core>/DesktopEditor/graphics/cmake   # (a minimal wrapper add_subdirectory)
# env: EO_BOOST_INC / EO_BOOST_LIB point at nixpkgs boost dev/out
```

`-DTHIRD_PARTY_PREPARED=TRUE` is the key: it makes `common.cmake` skip
`Common/3dParty/build_3rdparty.py` (network + compile mid-configure, sandbox-
incompatible) and instead consume a pre-populated install dir.

### Source patches to `core` (the spike edits, now upstreamed as patches/)

1. **Boost imported targets** — nixpkgs boost 1.89 has no separate
   `libboost_system` (header-only) and trips the legacy `FindBoost`; define
   `Boost::{system,filesystem,regex,date_time}` ourselves (`system` INTERFACE).
   (Now: `common.cmake`'s `EO_USE_SYSTEM_LIBS` uses `find_package(Boost CONFIG)`
   and aliases `Boost::system` to `Boost::headers` when absent — patch 0001.)
2. **Don't pass `-Wl,--disable-new-dtags` to Apple ld64** — gate it behind
   `if(NOT APPLE)` (the predicted "big" blocker; a one-liner) — patch 0001.
3. **`doctrenderer/CMakeLists.txt` `if(APPLE)` CMake syntax bug** —
   `target_compile_options(doctrenderer ...)` missing the `PRIVATE` keyword.
   Note: this branch uses **JavaScriptCore**, not V8 — V8 is unnecessary on
   darwin — patch 0005.
4. **`cximage/tiff/tif_config.h` `lfind`** — the Windows `#define lfind _lfind`
   leaked onto macOS (undefined `__lfind` at link); gate with
   `#if !defined(_LINUX) && !defined(__APPLE__)`.

### Pre-populated third-party install dir (`THIRD_PARTY_PREPARED` stub)

Source-form deps the libs compile in (pinned commits now in `sources.nix`,
assembled by `core.nix`/`desktop-sdk.nix`):

| install path                   | repo / commit                              |
| ------------------------------ | ------------------------------------------ |
| `html/katana-parser/`          | jasenhuang/katana-parser @ `be6df45…`      |
| `html/gumbo-parser/`           | google/gumbo-parser @ `aa91b27…`           |
| `socketio/src/`, `src_no_tls/` | socketio/socket.io-client-cpp @ `da77914…` |
| `harfbuzz/src/`                | harfbuzz/harfbuzz @ `894a1f7…`             |
| `brotli/c/`                    | google/brotli @ `a47d747…`                 |
| `hyphen/` (headers at root)    | hunspell/hyphen                            |

Binary deps come from nixpkgs (`boost` 1.89, `icu` 76, `openssl` 3.6) via
`find_package`. The spike's ICU note (hard-coded `libicuuc.so.{74,60}` Linux
names, symlinked from nixpkgs `.dylib`s) is superseded: `common.cmake`'s
`EO_USE_SYSTEM_LIBS` resolves ICU as imported targets (`ICU::uc/i18n/data`).

### What the spike verified (all since superseded by the reproducible build)

- ✅ `graphics`, `kernel`, `UnicodeConverter` build as arm64 dylibs from source.
- ✅ (now) the full non-desktop `core` — `x2t` + every format library — builds
  via `core.nix`; the desktop CEF/Qt integration via `desktop-sdk.nix`; the
  GUI bundle via `app.nix`.
- The spike's open items (Qt 5.9.9 + CEF 5414) were resolved differently than
  feared: the multi-Qt-aware sources build against nixpkgs **Qt 5.15**, and CEF
  is the **branch-109** prebuilt the desktop-sdk wrapper is ABI-locked to.
