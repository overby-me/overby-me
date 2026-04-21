#!/bin/bash
# Minimal test harness for rust-flatpak VM tests.
set -euo pipefail
export LC_ALL=C

FLATPAK="${FLATPAK:-flatpak}"
ARCH="$(uname -m)"
TEST_DATA_DIR="${WORK}/testdata"
FL_DIR="${HOME}/.local/share/flatpak"

mkdir -p "$TEST_DATA_DIR" "$FL_DIR"

TESTS_PASSED=0
TESTS_FAILED=0

ok() { echo "ok - $1"; TESTS_PASSED=$((TESTS_PASSED + 1)); }
skip() { echo "skip - $1"; }

assert_has_file() { [ -f "$1" ] || { echo "FAIL: expected file: $1"; exit 1; }; }
assert_has_dir() { [ -d "$1" ] || { echo "FAIL: expected dir: $1"; exit 1; }; }
assert_not_has_file() { [ ! -f "$1" ] || { echo "FAIL: unexpected file: $1"; exit 1; }; }
assert_not_has_dir() { [ ! -d "$1" ] || { echo "FAIL: unexpected dir: $1"; exit 1; }; }
assert_file_has_content() {
  grep -qE "$2" "$1" || { echo "FAIL: '$1' missing content matching '$2'"; cat "$1"; exit 1; }
}
assert_not_file_has_content() {
  if grep -qE "$2" "$1" 2>/dev/null; then echo "FAIL: '$1' unexpectedly contains '$2'"; cat "$1"; exit 1; fi
}
assert_file_empty() { [ ! -s "$1" ] || { echo "FAIL: expected empty: $1"; cat "$1"; exit 1; }; }
assert_streq() { [ "$1" = "$2" ] || { echo "FAIL: expected '$1' == '$2'"; exit 1; }; }
assert_not_streq() { [ "$1" != "$2" ] || { echo "FAIL: expected '$1' != '$2'"; exit 1; }; }

make_test_app() {
  local app_id="${1:-org.test.Hello}"
  local branch="${2:-stable}"
  local build_dir="$TEST_DATA_DIR/app-build-${app_id}"
  rm -rf "$build_dir"
  $FLATPAK build-init "$build_dir" "$app_id" org.test.Sdk org.test.Platform "$branch"
  mkdir -p "$build_dir/files/bin"
  cat > "$build_dir/files/bin/hello.sh" << 'SCRIPT'
#!/bin/sh
echo "Hello world, from a sandbox"
SCRIPT
  chmod +x "$build_dir/files/bin/hello.sh"

  # Desktop file
  mkdir -p "$build_dir/files/share/applications"
  cat > "$build_dir/files/share/applications/${app_id}.desktop" << DESKTOP
[Desktop Entry]
Name=Hello
Exec=hello.sh
Type=Application
Icon=${app_id}
DESKTOP

  # Icon
  mkdir -p "$build_dir/files/share/icons/hicolor/64x64/apps"
  echo "PNG" > "$build_dir/files/share/icons/hicolor/64x64/apps/${app_id}.png"

  $FLATPAK build-finish "$build_dir" --command hello.sh \
    --share network --share ipc \
    --socket x11 --socket wayland --socket pulseaudio \
    --device dri --filesystem home

  echo "$build_dir"
}

make_test_runtime() {
  local rt_id="${1:-org.test.Platform}"
  local branch="${2:-stable}"
  local build_dir="$TEST_DATA_DIR/runtime-build-${rt_id}"
  rm -rf "$build_dir"
  mkdir -p "$build_dir/files/bin" "$build_dir/files/lib" "$build_dir/files/etc"

  # Copy real binaries into the runtime so commands work in sandbox
  for cmd in sh bash cat echo ls env mkdir rm cp mv ln test readlink wc grep sed awk head tail sort tr cut tee id uname hostname date sleep true false pwd dirname basename; do
    local path
    path="$(command -v "$cmd" 2>/dev/null || true)"
    if [ -n "$path" ] && [ -f "$path" ]; then
      cp "$path" "$build_dir/files/bin/" 2>/dev/null || true
    fi
  done

  # Copy required shared libraries
  for lib_dir in /lib /lib64 /usr/lib /usr/lib64; do
    if [ -d "$lib_dir" ]; then
      cp -a "$lib_dir"/ld-linux*.so* "$build_dir/files/lib/" 2>/dev/null || true
      cp -a "$lib_dir"/libc.so* "$build_dir/files/lib/" 2>/dev/null || true
      cp -a "$lib_dir"/libdl.so* "$build_dir/files/lib/" 2>/dev/null || true
      cp -a "$lib_dir"/libpthread.so* "$build_dir/files/lib/" 2>/dev/null || true
      cp -a "$lib_dir"/libm.so* "$build_dir/files/lib/" 2>/dev/null || true
    fi
  done

  # On NixOS (and other distros), the dynamic linker is often at /lib64/ld-linux-x86-64.so.2.
  # Binaries have this path hardcoded in their ELF header. Inside the bwrap sandbox,
  # /lib64 -> usr/lib64, so we need files/lib64/ to contain the linker too.
  mkdir -p "$build_dir/files/lib64"
  cp -a "$build_dir/files/lib"/ld-linux*.so* "$build_dir/files/lib64/" 2>/dev/null || true
  # Also try copying from the host /lib64 directly (handles cases where /lib didn't have it)
  for f in /lib64/ld-linux*.so*; do
    [ -e "$f" ] && cp -a "$f" "$build_dir/files/lib64/" 2>/dev/null || true
  done
  # On NixOS, the real interpreter is in /nix/store; find it from any copied binary
  local interp
  interp=$(readelf -l "$build_dir/files/bin/sh" 2>/dev/null | grep -oP '/nix/store/\S+/lib/ld-linux[^\]]*' || true)
  if [ -n "$interp" ] && [ -f "$interp" ]; then
    cp -a "$interp" "$build_dir/files/lib64/" 2>/dev/null || true
    # Also create the standard name as a symlink
    local interp_name
    interp_name=$(basename "$interp")
    if [ ! -e "$build_dir/files/lib64/ld-linux-x86-64.so.2" ] && [ -f "$build_dir/files/lib64/$interp_name" ]; then
      ln -sf "$interp_name" "$build_dir/files/lib64/ld-linux-x86-64.so.2" 2>/dev/null || true
    fi
  fi

  cat > "$build_dir/metadata" << META
[Runtime]
name=${rt_id}
runtime=${rt_id}/${ARCH}/${branch}
sdk=org.test.Sdk/${ARCH}/${branch}
META

  echo "$build_dir"
}

setup_repo() {
  local branch="${1:-stable}"

  make_test_app org.test.Hello "$branch"
  make_test_runtime org.test.Platform "$branch"

  # Install app from build dir
  $FLATPAK --user install "$TEST_DATA_DIR/app-build-org.test.Hello" 2>&1 || true

  # Install runtime manually (direct file copy to installation)
  local rt_src="$TEST_DATA_DIR/runtime-build-org.test.Platform"
  local rt_dest="$FL_DIR/runtime/org.test.Platform/${ARCH}/${branch}/active"
  mkdir -p "$rt_dest"
  cp "$rt_src/metadata" "$rt_dest/metadata"
  cp -r "$rt_src/files" "$rt_dest/files"
}

install_repo() {
  local branch="${1:-master}"
  $FLATPAK --user install "$TEST_DATA_DIR/app-build-org.test.Hello" 2>&1 || true
}

run() {
  $FLATPAK --user run "$@"
}

run_sh() {
  local app_id="$1"
  shift
  $FLATPAK --user run --command=sh "$app_id" -c "$*"
}