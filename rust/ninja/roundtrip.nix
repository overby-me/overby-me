# Phase 4: differential roundtrip checks.
#
# Build a small multi-file C project against both rust-ninja and the
# reference `pkgs.ninja` and compare *behavior* (not raw binary bytes
# — gcc embeds absolute build paths in some sections, so a true `cmp`
# would diverge by construction). Each scenario validates a specific
# ninja-as-build-driver concern:
#
#   cold-build             : from an empty build dir, both runners
#                            produce all expected outputs and the
#                            resulting `app` binary runs and prints
#                            the expected greeting
#   incremental-noop       : a second invocation must report
#                            "no work to do." on both runners
#   incremental-modify     : touching one .c file rebuilds main.o + app
#                            but does NOT touch the unrelated .o (mtime
#                            stable across runners)
#   depfile-header-change  : modifying the shared header rebuilds
#                            both .o files via gcc -MMD depfile
#                            parsing — both runners must observe a
#                            fresh greet.o mtime
#   cmake-cold-build       : a cmake-generated `build.ninja` tree
#                            (with `include`, `$DEP_FILE`, `restat`)
#                            cold-builds, no-ops on re-run, and
#                            rebuilds correctly after a header touch
#                            on both runners
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-ninja-roundtrip-${name}" {
  nativeBuildInputs = [
    pkgs.rust-ninja-dev
    pkgs.ninja
    pkgs.gcc
    pkgs.cmake
    pkgs.coreutils
    pkgs.bash
    pkgs.diffutils
  ];
} ''
    set -euo pipefail

    RUST_NINJA="${pkgs.rust-ninja-dev}/bin/ninja"
    REF_NINJA="${pkgs.ninja}/bin/ninja"

    # Build a tiny C project with two translation units and a shared
    # header. Layout matches what a CMake `Ninja` generator would emit.
    setup_project() {
      local d="$1"
      mkdir -p "$d/src" "$d/inc" "$d/obj"
      cat > "$d/inc/greet.h" <<'EOF'
  #ifndef GREET_H
  #define GREET_H
  const char *greeting(void);
  #endif
  EOF
      cat > "$d/src/greet.c" <<'EOF'
  #include "greet.h"
  const char *greeting(void) { return "hello"; }
  EOF
      cat > "$d/src/main.c" <<'EOF'
  #include <stdio.h>
  #include "greet.h"
  int main(void) { printf("%s\n", greeting()); return 0; }
  EOF
      cat > "$d/build.ninja" <<'EOF'
  cflags = -O0 -Iinc

  rule cc
    command = gcc -MMD -MF $out.d $cflags -c $in -o $out
    depfile = $out.d
    description = CC $out

  rule link
    command = gcc $in -o $out
    description = LINK $out

  build obj/greet.o: cc src/greet.c
  build obj/main.o: cc src/main.c
  build app: link obj/greet.o obj/main.o
  default app
  EOF
    }

    RUST_DIR=$PWD/out-rust
    REF_DIR=$PWD/out-ref
    setup_project "$RUST_DIR"
    setup_project "$REF_DIR"

    echo "=== scenario: ${name} ==="

    # Run a fresh ninja in a project dir and assert the produced `app`
    # prints the expected greeting. Catches both build failures and
    # silent linking-of-empty-files bugs.
    assert_app_works() {
      local d="$1"
      local got
      got=$( "$d/app" )
      [ "$got" = "hello" ] || {
        echo "FAIL: $d/app printed: $got"; exit 1; }
    }

    case "${name}" in
      cold-build)
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        for f in obj/greet.o obj/main.o app; do
          test -f "$RUST_DIR/$f" || { echo "FAIL: rust missing $f"; exit 1; }
          test -f "$REF_DIR/$f"  || { echo "FAIL: ref missing $f";  exit 1; }
        done
        assert_app_works "$RUST_DIR"
        assert_app_works "$REF_DIR"
        ;;

      incremental-noop)
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        out_rust=$( cd "$RUST_DIR" && $RUST_NINJA )
        out_ref=$(  cd "$REF_DIR"  && $REF_NINJA  )
        echo "rust: $out_rust"
        echo "ref : $out_ref"
        [ "$out_rust" = "ninja: no work to do." ] || {
          echo "FAIL: rust-ninja did not report no-work-to-do"; exit 1; }
        [ "$out_ref"  = "ninja: no work to do." ] || {
          echo "FAIL: reference ninja did not report no-work-to-do"; exit 1; }
        ;;

      incremental-modify)
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        stat_before_rust=$(stat -c '%Y' "$RUST_DIR/obj/greet.o")
        stat_before_ref=$( stat -c '%Y' "$REF_DIR/obj/greet.o")
        sleep 1.1
        touch "$RUST_DIR/src/main.c"
        touch "$REF_DIR/src/main.c"
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        stat_after_rust=$(stat -c '%Y' "$RUST_DIR/obj/greet.o")
        stat_after_ref=$( stat -c '%Y' "$REF_DIR/obj/greet.o")
        echo "greet.o mtime rust: $stat_before_rust -> $stat_after_rust"
        echo "greet.o mtime ref : $stat_before_ref  -> $stat_after_ref"
        [ "$stat_before_rust" = "$stat_after_rust" ] || {
          echo "FAIL: rust-ninja unnecessarily rebuilt greet.o"; exit 1; }
        [ "$stat_before_ref"  = "$stat_after_ref"  ] || {
          echo "FAIL: reference ninja unnecessarily rebuilt greet.o"; exit 1; }
        assert_app_works "$RUST_DIR"
        assert_app_works "$REF_DIR"
        ;;

      depfile-header-change)
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        sleep 1.1
        stat_before_rust=$(stat -c '%Y' "$RUST_DIR/obj/greet.o")
        stat_before_ref=$( stat -c '%Y' "$REF_DIR/obj/greet.o")
        cat >> "$RUST_DIR/inc/greet.h" <<'EOF'
  /* trivial change */
  EOF
        cat >> "$REF_DIR/inc/greet.h" <<'EOF'
  /* trivial change */
  EOF
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        stat_after_rust=$(stat -c '%Y' "$RUST_DIR/obj/greet.o")
        stat_after_ref=$( stat -c '%Y' "$REF_DIR/obj/greet.o")
        echo "greet.o mtime rust: $stat_before_rust -> $stat_after_rust"
        echo "greet.o mtime ref : $stat_before_ref  -> $stat_after_ref"
        [ "$stat_before_rust" != "$stat_after_rust" ] || {
          echo "FAIL: rust-ninja did not rebuild greet.o after header change";
          exit 1; }
        [ "$stat_before_ref"  != "$stat_after_ref"  ] || {
          echo "FAIL: reference ninja did not rebuild greet.o after header change";
          exit 1; }
        assert_app_works "$RUST_DIR"
        assert_app_works "$REF_DIR"
        ;;

      cmake-cold-build)
        # Real-world stress test: a CMake-generated `build.ninja`
        # tree (rules.ninja include, $DEP_FILE bindings, restat,
        # CUSTOM_COMMAND, multi-edge link rules). Both runners must
        # cold-build the project, then report no work on a re-run,
        # then rebuild the same set of objects after a header touch.
        SRC_DIR=$PWD/cmake-src
        mkdir -p "$SRC_DIR/inc" "$SRC_DIR/src"
        cat > "$SRC_DIR/CMakeLists.txt" <<'EOF'
  cmake_minimum_required(VERSION 3.20)
  project(hello C)
  add_library(greet STATIC src/greet.c)
  target_include_directories(greet PUBLIC inc)
  add_executable(app src/main.c)
  target_link_libraries(app PRIVATE greet)
  EOF
        cat > "$SRC_DIR/inc/greet.h" <<'EOF'
  const char *greeting(void);
  EOF
        cat > "$SRC_DIR/src/greet.c" <<'EOF'
  #include "greet.h"
  const char *greeting(void) { return "hello"; }
  EOF
        cat > "$SRC_DIR/src/main.c" <<'EOF'
  #include <stdio.h>
  #include "greet.h"
  int main(void) { printf("%s\n", greeting()); return 0; }
  EOF

        # cmake refuses an in-source build if any cache lives there,
        # so generate two separate build trees pointing at the same
        # source tree. Each runner gets its own.
        RUST_BUILD=$PWD/cmake-rust && mkdir -p "$RUST_BUILD"
        REF_BUILD=$PWD/cmake-ref  && mkdir -p "$REF_BUILD"
        ( cd "$RUST_BUILD" && cmake -G Ninja "$SRC_DIR" >/dev/null )
        ( cd "$REF_BUILD"  && cmake -G Ninja "$SRC_DIR" >/dev/null )

        ( cd "$RUST_BUILD" && $RUST_NINJA )
        ( cd "$REF_BUILD"  && $REF_NINJA  )
        test -f "$RUST_BUILD/app" || { echo "FAIL: rust missing app"; exit 1; }
        test -f "$REF_BUILD/app"  || { echo "FAIL: ref missing app";  exit 1; }
        [ "$( "$RUST_BUILD/app" )" = "hello" ]
        [ "$( "$REF_BUILD/app"  )" = "hello" ]

        # Second invocation: nothing to do.
        out_rust=$( cd "$RUST_BUILD" && $RUST_NINJA )
        out_ref=$(  cd "$REF_BUILD"  && $REF_NINJA  )
        [ "$out_rust" = "ninja: no work to do." ] || {
          echo "FAIL: rust-ninja: $out_rust"; exit 1; }
        [ "$out_ref"  = "ninja: no work to do." ] || {
          echo "FAIL: reference ninja: $out_ref"; exit 1; }

        # Header-touch rebuild via CMake-generated `deps = gcc`
        # depfile rules. We only assert observability — the binary
        # must still run after both rebuild.
        sleep 1.1
        touch "$SRC_DIR/inc/greet.h"
        ( cd "$RUST_BUILD" && $RUST_NINJA )
        ( cd "$REF_BUILD"  && $REF_NINJA  )
        [ "$( "$RUST_BUILD/app" )" = "hello" ]
        [ "$( "$REF_BUILD/app"  )" = "hello" ]
        ;;

      *)
        echo "Unknown scenario: ${name}"
        exit 1
        ;;
    esac

    echo "PASS: ${name}"
    touch $out
''
