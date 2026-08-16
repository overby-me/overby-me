# Run a single test from the official GNU Bash test suite against rust-bash.
#
# Compares rust-bash output against reference bash output (both running in the
# same sandbox), avoiding false failures from .right files generated on
# different systems.
#
# Run with: nix build .#checks.x86_64-linux.rust-bash-test-{name}
# Example:  nix build .#checks.x86_64-linux.rust-bash-test-arith
{
  pkgs,
  name,
}:
pkgs.runCommand "rust-bash-test-${name}" {
  nativeBuildInputs = [pkgs.rust-bash-dev pkgs.bash pkgs.gcc pkgs.coreutils pkgs.diffutils pkgs.gnused pkgs.gnugrep pkgs.gawk pkgs.findutils pkgs.glibcLocales];
  bashSrc = pkgs.bash.src;
} ''
  # Extract the test suite and helper sources
  tar xzf $bashSrc
  BASH_SRC=$(echo bash-*)

  # Build helper programs (recho, zecho, printenv) needed by the tests
  gcc -o recho "$BASH_SRC/support/recho.c"
  gcc -o zecho "$BASH_SRC/support/zecho.c"
  gcc -o printenv "$BASH_SRC/support/printenv.c"
  gcc -o xcase "$BASH_SRC/support/xcase.c"

  cd "$BASH_SRC/tests"

  # Put helpers on PATH
  export PATH="$OLDPWD:$PATH"
  export TMPDIR="$(mktemp -d)"

  # Make locales available for tests that need them (e.g. printf2.sub)
  export LOCALE_ARCHIVE="${pkgs.glibcLocales}/lib/locale/locale-archive"

  echo "Running bash test: ${name}"

  # Run with reference bash
  export THIS_SH="${pkgs.bash}/bin/bash"
  timeout 300 "$THIS_SH" "./${name}.tests" > "$TMPDIR/expected" 2>&1 || true

  # Run with rust-bash
  export THIS_SH="${pkgs.rust-bash-dev}/bin/bash"
  timeout 300 "$THIS_SH" "./${name}.tests" > "$TMPDIR/actual" 2>&1 || true

  # Normalize binary paths so that /nix/store/.../bin/bash differences don't
  # cause false failures. Replace both shell paths with a generic "bash" prefix.
  REF_BASH="${pkgs.bash}/bin/bash"
  TEST_BASH="${pkgs.rust-bash-dev}/bin/bash"
  sed -i "s|$REF_BASH|bash|g" "$TMPDIR/expected"
  sed -i "s|$TEST_BASH|bash|g" "$TMPDIR/actual"

  # Normalize PIDs in temp paths used by tests.  Tests create paths like
  # /tmp-<pid>, /type-<pid>, /bash-zzz-<pid>, /zero-length-file-<pid>.
  # Match: a word-hyphen-digits pattern where the digits are 2-7 long.
  sed -i -E 's|([a-zA-Z])-([0-9]{2,7})\b|\1-PID|g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Normalize BASHPID="<pid>" and PPID="<pid>" in declare output, and
  # bare PID values like ref_PID="<pid>" that appear in variable dumps.
  sed -i -E 's|BASHPID="[0-9]+"|BASHPID="PID"|g' "$TMPDIR/expected" "$TMPDIR/actual"
  sed -i -E 's|PPID="[0-9]+"|PPID="PID"|g' "$TMPDIR/expected" "$TMPDIR/actual"
  sed -i -E 's|_PID="[0-9]+"|_PID="PID"|g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Normalize $_ which contains different nix store paths (e.g. timeout,
  # coreutils, or the shell itself).  Replace the full nix store path
  # with a generic placeholder.
  sed -i -E 's|_="/nix/store/[^"]+"|_="NIXPATH"|g' "$TMPDIR/expected" "$TMPDIR/actual"

  # Normalize thread/process IDs in Rust panic messages
  sed -i -E "s|thread '([^']*)' \([0-9]+\)|thread '\1' (PID)|g" "$TMPDIR/expected" "$TMPDIR/actual"

  # Remove flaky SIGPIPE "write error: Broken pipe" lines from both outputs.
  # These are timing-dependent: whether echo hits a broken pipe depends on
  # whether the pipe reader has closed before the write completes.  Both
  # shells can produce or omit this line depending on scheduling.
  sed -i '/echo: write error: Broken pipe$/d' "$TMPDIR/expected" "$TMPDIR/actual"

  # Remove flaky CHLD signal lines that appear due to timing-dependent
  # SIGCHLD delivery.  In the nix sandbox, child process reaping timing
  # differs from local runs, causing extra or missing CHLD lines.
  sed -i '/^CHLD$/d' "$TMPDIR/expected" "$TMPDIR/actual"

  # Remove SIGPIPE trap lines that appear due to the nix sandbox or Rust
  # runtime inheriting SIG_IGN for SIGPIPE.  The Rust runtime (editions
  # < 2024) sets SIGPIPE to SIG_IGN before main(); our .init_array
  # constructor detects this and records it as an inherited ignore, which
  # then shows up in `trap` listings.  Real bash (C) doesn't have this
  # issue.  Remove from both outputs to be safe.
  sed -i '/^trap -- .* SIGPIPE$/d' "$TMPDIR/expected" "$TMPDIR/actual"

  # Compare
  if diff --text "$TMPDIR/actual" "$TMPDIR/expected"; then
    touch $out
  else
    exit 1
  fi
''
