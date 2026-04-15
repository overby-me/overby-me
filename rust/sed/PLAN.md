# rust-sed: Plan to Pass Upstream GNU sed Tests

## Current Status

**17/60 tests passing** (28%) — shell tests from the GNU sed 4.9 test suite.

### Test infrastructure approach

GNU sed's test suite differs from gawk's: tests are **shell scripts** (using gnulib's `init.sh` framework) and a **Perl test harness** (`misc.pl`), rather than simple program+input file pairs. Each test script defines its own expected output and checks internally.

Tests run in a Nix sandbox (`testsuite.nix`) with rust-sed as `sed` in PATH, using the upstream test framework directly. The harness sets `abs_top_srcdir`, `srcdir`, `LC_ALL=C`, and other TESTS_ENVIRONMENT variables from the GNU sed Makefile.

Run a test: `nix build .#checks.x86_64-linux.rust-sed-test-{name}`
View failure: `nix log .#checks.x86_64-linux.rust-sed-test-{name}`

---

## Phase 1: Test Infrastructure

### 1a: Nix package updates (`default.nix`)

Add a `rust-sed-dev` debug build (fast compile for testing), matching rust-awk's pattern:

```nix
rust-sed-dev = { lib, rustPlatform }:
  rustPlatform.buildRustPackage {
    pname = "rust-sed-dev";
    version = "0.1.0";
    src = ...;
    cargoLock.lockFile = ./Cargo.lock;
    buildType = "debug";
    meta = { ... mainProgram = "sed"; };
  };
```

### 1b: Shell test harness (`testsuite-sh.nix`)

Create a Nix derivation that runs a single `.sh` test from the GNU sed 4.9 source:

```nix
{ pkgs, name }:
pkgs.runCommand "rust-sed-test-${name}" {
  nativeBuildInputs = [
    pkgs.rust-sed-dev pkgs.coreutils pkgs.diffutils
    pkgs.gnused pkgs.gnugrep pkgs.bash pkgs.perl
  ];
  gnusedSrc = pkgs.gnused.src;
} ''
  tar xf $gnusedSrc
  SED_SRC=$(echo sed-*)

  # Create a wrapper directory with rust-sed as "sed"
  mkdir -p bin
  ln -s ${pkgs.rust-sed-dev}/bin/sed bin/sed
  export PATH="$PWD/bin:$PATH"

  # Set up the test framework
  export srcdir="$SED_SRC"
  cd $(mktemp -d)

  # Run the test script
  fail=0
  . "$SED_SRC/testsuite/init.sh"
  . "$SED_SRC/testsuite/${name}.sh" && touch $out || exit 1
''
```

**Challenge**: The `init.sh` framework uses `path_prepend_`, `compare_`, `returns_`, `skip_`, `fail_`, and `Exit` helpers. The scripts source `init.sh` themselves, so we need to ensure the framework is available and `sed` resolves to rust-sed.

**Alternative approach** (simpler, more robust): Instead of running `init.sh` scripts directly, create a wrapper that:

1. Extracts the GNU sed source
2. Places rust-sed as `sed` first in PATH
3. Runs the test script as a standalone shell script
4. Checks the exit code (0 = pass, 77 = skip, other = fail)

### 1c: Perl test harness (`testsuite-pl.nix`)

For `misc.pl` (and `debug.pl`), create a derivation that runs the Perl harness:

```nix
{ pkgs, name }:
pkgs.runCommand "rust-sed-test-misc" {
  nativeBuildInputs = [
    pkgs.rust-sed-dev pkgs.perl pkgs.coreutils pkgs.diffutils
  ];
  gnusedSrc = pkgs.gnused.src;
} ''
  tar xf $gnusedSrc
  SED_SRC=$(echo sed-*)

  mkdir -p bin
  ln -s ${pkgs.rust-sed-dev}/bin/sed bin/sed
  export PATH="$PWD/bin:$PATH"

  cd $SED_SRC
  perl -w -Itestsuite -MCuSkip -MCoreutils \
    -M"CuTmpdir qw(misc)" testsuite/misc.pl \
    && touch $out || exit 1
''
```

### 1d: Test list in `default.nix`

Generate checks from the test name list:

```nix
checks = let
  shTests = [ "bug32082" "compile-errors" "compile-tests" ... ];
  plTests = [ "misc" "debug" ];
in
  builtins.listToAttrs (
    (map (name: {
      name = "rust-sed-test-${name}";
      value = pkgs: import ./testsuite-sh.nix { inherit pkgs name; };
    }) shTests)
    ++
    (map (name: {
      name = "rust-sed-test-${name}";
      value = pkgs: import ./testsuite-pl.nix { inherit pkgs name; };
    }) plTests)
  );
```

---

## Phase 2: Initial Test Run and Triage

### 2a: Shell tests (~60 tests)

Run all shell-based tests and categorise results. Expected test list from `local.mk`:

**Core functionality tests:**

- `compile-tests` - Label parsing, address regex, bracket expressions, unterminated strings
- `compile-errors` - Error messages for invalid programs
- `execute-tests` - D, e, P, N, Q, r, W, F commands, T branch
- `command-endings` - Command termination variants
- `subst-options` - Substitution flags: `i/I`, `g`, nth occurrence, `m/M` multiline
- `subst-replacement` - Replacement string escapes, `&`, backreferences
- `nulldata` - `-z` null-delimited mode across commands (`s`, `=`, `l`, `F`)

**Bug fix regression tests:**

- `bug32082` - Specific historical bug
- `bug32271-1`, `bug32271-2` - Specific historical bugs

**Command-specific tests:**

- `cmd-l` - `l` command (visual dump)
- `cmd-0r` - `0r` address (before first line)
- `cmd-R` - `R` command (read single line from file)
- `comment-n` - Comments with `-n` flag
- `colon-with-no-label` - Empty label handling

**Regex and pattern tests:**

- `posix-char-class` - POSIX character classes (`[:alpha:]`, etc.)
- `posix-mode-addr` - POSIX mode address handling
- `posix-mode-bad-ref` - POSIX mode invalid backreferences
- `posix-mode-ERE` - Extended regex in POSIX mode
- `posix-mode-s` - POSIX mode substitution
- `posix-mode-N` - POSIX mode N command (exit on last line)
- `range-overlap` - Overlapping address ranges
- `regex-errors` - Regex error handling
- `regex-max-int` - Large line number addresses
- `newline-dfa-bug` - DFA regex newline handling
- `word-delim` - `\b`, `\w` word delimiters

**In-place editing tests:**

- `in-place-hyphen` - In-place editing of file named `-`
- `in-place-suffix-backup` - Backup suffix handling
- `inplace-hold` - Hold space preservation during in-place edit
- `inplace-selinux` - SELinux context preservation (likely skip)
- `temp-file-cleanup` - Temp file cleanup on error

**Multi-byte / encoding tests:**

- `mb-bad-delim` - Multi-byte bad delimiter
- `mb-charclass-non-utf8` - Character classes in non-UTF-8 locales
- `mb-match-slash` - Multi-byte slash matching
- `mb-y-translate` - Multi-byte y command
- `subst-mb-incomplete` - Incomplete multi-byte sequences
- `8bit` - 8-bit character handling
- `8to7` - 8-to-7 bit conversion
- `badenc` - Bad encoding handling
- `utf8-ru` - Russian UTF-8 text
- `newjis` - Japanese encoding

**I/O and misc tests:**

- `stdin` - Reading from stdin
- `stdin-prog` - Program from stdin
- `unbuffered` - Unbuffered I/O (`-u` flag)
- `follow-symlinks` - `--follow-symlinks` option
- `follow-symlinks-stdin` - Follow symlinks with stdin
- `missing-filename` - Missing filename error
- `sandbox` - `--sandbox` mode
- `normalize-text` - Text normalization
- `convert-number` - Number conversion in substitution
- `title-case` - Title case conversion
- `recursive-escape-c` - Recursive escape in `c` command
- `panic-tests` - Crash/panic regression tests
- `help` - `--help` output
- `help-version` - `--help` and `--version` output

**Classic sed programs:**

- `binary` - Binary file handling
- `bsd` - BSD sed compatibility
- `bsd-wrapper` - BSD wrapper compatibility
- `dc` - dc calculator in sed
- `distrib` - Distribution test
- `eval` - Eval functionality
- `mac-mf` - Mac makefile processing
- `madding` - Complex text processing
- `uniq` - uniq implementation in sed
- `xemacs` - XEmacs config processing
- `invalid-mb-seq-UMR` - Invalid multi-byte sequence
- `obinary` - Binary output mode

### 2b: Perl tests (~50+ test cases in `misc.pl`)

`misc.pl` contains ~50 individual test cases covering:

- Basic operations: `empty`, `head`, `allsub`
- Regex features: `space` (`\S`, `\s`), `zero-anchor`, `case-insensitive`
- Edge cases: `preserve-missing-EOL-at-EOF`, `y-bracket`, `y-zero`, `y-newline`
- Address modes: `0range`, `dollar`
- Substitution: `amp-escape`, `fasts`, `recall`, `recall2`
- Control flow: `appquit`, `brackets`, `bkslashes`
- Character classes: `classes`
- Variables: `cv-vars`
- I/O: `file`, `quiet`, `enable`
- And ~30 more

---

## Phase 3: Expected Failure Categories

Based on the rust-sed implementation (1790 lines in `src/main.rs`), likely failure areas:

### Category 1: Error messages and diagnostics

Tests like `compile-errors`, `regex-errors`, `missing-filename`, `panic-tests` expect exact GNU sed error message strings (e.g., `sed: -e expression #1, char N: unterminated 's' command`). Rust-sed likely produces different error formats.

**Estimated tests:** 5-10
**Fix:** Match GNU sed error message format strings exactly.

### Category 2: POSIX mode behaviour

Tests `posix-mode-addr`, `posix-mode-bad-ref`, `posix-mode-ERE`, `posix-mode-s`, `posix-mode-N` test `--posix` flag semantics. Rust-sed has the flag but behaviour may differ.

**Estimated tests:** 5
**Fix:** Audit POSIX mode differences (N command exit-on-last-line, ERE vs BRE, backreference limits).

### Category 3: In-place editing (`-i`)

Tests `in-place-hyphen`, `in-place-suffix-backup`, `inplace-hold`, `temp-file-cleanup` test edge cases of `-i` flag. Common issues: backup file naming, temp file atomicity, hold space across files.

**Estimated tests:** 3-5
**Fix:** Verify atomic rename, suffix handling, edge cases.

### Category 4: Multi-byte / encoding

Tests `mb-*`, `8bit`, `badenc`, `utf8-ru`, `newjis`, `subst-mb-incomplete` test locale-aware multi-byte handling. Rust's regex crate handles UTF-8 natively but may differ on non-UTF-8 locales and invalid sequences.

**Estimated tests:** 8-10
**Fix:** Handle `LC_ALL=C` (byte-mode), invalid UTF-8 passthrough, locale-dependent character classes.

### Category 5: GNU extensions

Commands like `e` (execute), `R` (read line), `W` (write first line), `F` (filename), `Q` (quit with code), `0` address, `first~step` addresses — these GNU extensions may have subtle differences.

**Estimated tests:** 5-8
**Fix:** Verify each GNU extension against test expectations.

### Category 6: Regex BRE/ERE edge cases

Tests `posix-char-class`, `word-delim`, `newline-dfa-bug`, `regex-max-int` test regex corner cases. Rust's regex crate differs from POSIX BRE/ERE in some areas (backreferences, character classes).

**Estimated tests:** 5-7
**Fix:** BRE-to-ERE conversion edge cases, POSIX character class support, `\b`/`\w` extensions.

### Category 7: Substitution edge cases

Tests `subst-options`, `subst-replacement`, `subst-mb-incomplete` cover nth-occurrence, multiline `m/M` flag, replacement escapes.

**Estimated tests:** 3-5
**Fix:** Verify nth-occurrence substitution, `m`/`M` multiline flag, replacement string escapes.

### Category 8: Classic sed programs

Tests `dc`, `uniq`, `madding`, `xemacs`, `distrib`, `mac-mf` run complex sed programs. These are integration tests that stress the full command set.

**Estimated tests:** 5-6
**Fix:** These will pass once individual command bugs are fixed.

---

## Phase 4: Implementation Priorities

### Priority 1: Get test infrastructure working (Phase 1)

- Create `rust-sed-dev` package
- Create `testsuite-sh.nix` harness
- Create `testsuite-pl.nix` harness (if feasible)
- Add test list to `default.nix`
- Run all tests, record initial pass/fail baseline

### Priority 2: Error message compatibility (high impact, many tests)

- Match GNU sed error format: `sed: -e expression #N, char M: <message>`
- Match file-based error format: `sed: file <name> line N: <message>`

### Priority 3: Substitution correctness

- nth-occurrence (`s/./x/3`)
- Multiline mode (`m`/`M` flag)
- Replacement escapes

### Priority 4: POSIX mode compliance

- `N` command behaviour on last line
- ERE vs BRE mode
- Backreference validation

### Priority 5: In-place editing edge cases

- Backup suffix, temp file handling
- Hold space preservation across files

### Priority 6: Multi-byte / locale support

- Non-UTF-8 locale handling
- Invalid byte sequence passthrough
- Locale-dependent character classes

### Priority 7: GNU extension completeness

- `e`, `R`, `W`, `F`, `Q` commands
- `0` and `first~step` addresses

---

## Test Inventory

### Shell tests (60)

```text
8bit            8to7            badenc          binary
bsd             bsd-wrapper     bug32082        bug32271-1
bug32271-2      cmd-0r          cmd-l           cmd-R
colon-with-no-label             command-endings comment-n
compile-errors  compile-tests   convert-number  dc
distrib         eval            execute-tests   follow-symlinks
follow-symlinks-stdin           help            help-version
in-place-hyphen in-place-suffix-backup          inplace-hold
inplace-selinux invalid-mb-seq-UMR              mac-mf
madding         mb-bad-delim    mb-charclass-non-utf8
mb-match-slash  mb-y-translate  missing-filename
newjis          newline-dfa-bug normalize-text  nulldata
obinary         panic-tests     posix-char-class
posix-mode-addr posix-mode-bad-ref              posix-mode-ERE
posix-mode-N    posix-mode-s    range-overlap
recursive-escape-c              regex-errors    regex-max-int
sandbox         stdin           stdin-prog      subst-mb-incomplete
subst-options   subst-replacement               temp-file-cleanup
title-case      unbuffered      uniq            utf8-ru
word-delim      xemacs
```

### Perl tests (2 files, ~50+ cases)

```text
misc.pl    (~50 test cases)
debug.pl   (debug/trace output tests)
```

### Tests likely to skip (environment-dependent)

- `inplace-selinux` - Requires SELinux
- `follow-symlinks`, `follow-symlinks-stdin` - Requires `--follow-symlinks` (Linux-only GNU extension)
- `help-version` - Exact version string matching
- `help` - Exact help text matching
- `sandbox` - `--sandbox` flag (GNU extension, may not implement)
- `invalid-mb-seq-UMR` - Requires specific locale setup

### Current Status

**17/60 tests passing** (28%) — shell tests from the GNU sed 4.9 test suite.

Note: Some tests excluded from the Nix harness (`8bit`, `badenc`, `newjis`, `utf8-ru`, `invalid-mb-seq-UMR` — require specific locale setups or non-UTF-8 binary input files; `help-version` — requires exact GNU version strings). `inplace-selinux` passes by skipping (no SELinux in sandbox).

### Passing (17 tests)

bsd, bug32082, bug32271-1, bug32271-2, inplace-selinux (skip), madding,
mb-bad-delim, mb-charclass-non-utf8, mb-match-slash, mb-y-translate,
newline-dfa-bug, posix-mode-ERE, regex-max-int, stdin, subst-mb-incomplete,
title-case, word-delim

### Failing (43 tests)

8to7, binary, bsd-wrapper, cmd-0r, cmd-l, cmd-R, colon-with-no-label,
command-endings, comment-n, compile-errors, compile-tests, convert-number,
dc, distrib, eval, execute-tests, follow-symlinks, follow-symlinks-stdin,
help, in-place-hyphen, in-place-suffix-backup, inplace-hold, mac-mf,
missing-filename, normalize-text, nulldata, obinary, panic-tests,
posix-char-class, posix-mode-addr, posix-mode-bad-ref, posix-mode-N,
posix-mode-s, range-overlap, recursive-escape-c, regex-errors, sandbox,
stdin-prog, subst-options, subst-replacement, temp-file-cleanup, unbuffered,
uniq, xemacs
