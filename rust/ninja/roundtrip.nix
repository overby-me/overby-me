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
#   depfile-header-change  : modifying the shared header rebuilds both
#                            .o files via gcc -MMD depfile parsing —
#                            same observable outcome on both runners
#                            (this currently EXPECTS rust-ninja to
#                            differ from reference, since depfile
#                            parsing isn't implemented yet; the test
#                            documents that gap rather than gating CI
#                            on it)
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-ninja-roundtrip-${name}" {
  nativeBuildInputs = [
    pkgs.rust-ninja-dev
    pkgs.ninja
    pkgs.gcc
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
        stat_before_ref=$(stat -c '%Y' "$REF_DIR/obj/greet.o")
        cat >> "$RUST_DIR/inc/greet.h" <<'EOF'
  /* trivial change */
  EOF
        cat >> "$REF_DIR/inc/greet.h" <<'EOF'
  /* trivial change */
  EOF
        ( cd "$RUST_DIR" && $RUST_NINJA )
        ( cd "$REF_DIR"  && $REF_NINJA  )
        # Reference ninja parses depfiles — both .o files must rebuild.
        stat_after_ref=$(stat -c '%Y' "$REF_DIR/obj/greet.o")
        [ "$stat_before_ref" != "$stat_after_ref" ] || {
          echo "FAIL: reference ninja did not rebuild greet.o after header change";
          exit 1; }
        assert_app_works "$RUST_DIR"
        assert_app_works "$REF_DIR"
        # NOTE: rust-ninja is intentionally NOT asserted to rebuild
        # greet.o here — depfile parsing is a future phase. Once
        # implemented, tighten this scenario to require a fresh greet.o
        # mtime on both runners.
        ;;

      *)
        echo "Unknown scenario: ${name}"
        exit 1
        ;;
    esac

    echo "PASS: ${name}"
    touch $out
''
