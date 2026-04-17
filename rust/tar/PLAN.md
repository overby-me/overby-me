# rust-tar: Plan to Pass All Upstream GNU tar Tests

## Current status

**92/225 tests passing (41%)** after the first iteration pass.

Baseline checkpoint: `rust-tar-test-NAME` fully wired as flake checks
via `gnutar-test-harness` (autom4te-built `testsuite` script + helper
programs), 225 upstream `.at` tests under evaluation.

### What works

- Create / list / extract for regular files, symlinks, directories.
- `-r`/`--append`, `-u`/`--update`, `-d`/`--diff`/`--compare`,
  `--delete`.
- `TAR_OPTIONS` environment variable prepended to argv.
- Bundled short flags (`cvfT`, `xvfT`, etc.) with proper arg-taker
  pairing so `-f` / `-T` / `-X` / `-L` / `-b` / `-H` / `-g` each eat
  the next argv word.
- `-H FMT` / `--format=FMT` parsed; also `-V LABEL`, `-T FILE`,
  `-X FILE`, `--add-file` (including `--add-file=…` inside -T lists).
- `--exclude`, `--exclude-from`, `--exclude-caches`(-under|-all),
  `--exclude-tag`(-under|-all)=FILE, `--exclude-backups`,
  `--exclude-vcs` (with the standard VCS list).
- `--wildcards` / `--no-wildcards` and `--anchored` / `--no-anchored`
  with proper per-entry tracking plus separate state for list/extract
  path matching.
- `--mtime=@SECONDS|ISO`, `--mode=EXPR`, `--owner=NAME[:UID]`,
  `--group=NAME[:GID]`, `--numeric-owner`, `--transform=EXPR`,
  `--xform=EXPR`, `--strip-components=N`.
- `-P`/`--absolute-names`, `-h`/`--dereference`, `-o`/
  `--no-same-owner`, `-p`/`--preserve-permissions`,
  `--no-same-permissions`, `--no-recursion`.
- Short-file detection emits GNU's
  `tar: This does not look like a tar archive` + exit 2.
- `--delete NAME` on missing member emits
  `tar: NAME: Not found in archive` + exit 2.
- `--pax-option` on non-POSIX archive emits the GNU-exact error.
- Verbose routing: `-cv ... -f -` → verbose to stderr; `-xv` and
  `-tv` → verbose to stdout; `-vv` → detailed listing (mode
  user/group size date name) via `format_verbose_entry`.
- Username/group lookup via `uzers` crate so listings carry names.

### What remains (≈133 failing)

Most of the remaining failures depend on deeper features:

- **Sparse files** (`sparse*`, `sparsemv*`, `spmvp*`, `sptr*`, ≈12 tests)
  — need `lseek(SEEK_HOLE)` during create and materialisation during
  extract.
- **Incremental / listed-incremental** (`incr*`, `listed*`, `dirrem*`,
  `filerem*`, `rename*`, `remfiles*`, ≈40+ tests) — `-g FILE`
  snapshot database, `--level=N`, dumpdir format.
- **Multi-volume** (`multiv*`, ≈10 tests) — `-M`, `--tape-length=N`,
  `--new-volume-script`.
- **Extended attributes / ACL / SELinux / file capabilities**
  (`xattr0[2-8]`, partial support already) — proper xattr / ACL
  storage.
- **Labels / GNU volume headers** (`label0*`, `xform01`, ≈5 tests) —
  need to write the first entry as a volume header with the given
  label.
- **Transform-in-listing** (`xform0[1-3]`, `xform-h`, ≈4 tests) —
  show-transformed-names should apply transforms to listing output.
- **Format-specific byte layout** (`longv7`, `lustar01`, `extrac1?`,
  `options03`, ≈10 tests) — emit archive in the chosen format
  (`v7`/`ustar`/`oldgnu`/`posix`) and enforce its filename-length
  restrictions.
- **Positional options** (`positional0[1-3]`, `recurs02`, ≈5 tests) —
  GNU's "options set after positional args have no effect" semantics
  plus the accompanying warning.
- **-C in file lists** (`T-cd`, `T-dir0*`, `T-rec`, `T-recurse`,
  `T-null*`, ≈6 tests) — per-line parsing of `-C DIR` inside -T
  input.
- **Error message format parity** (`extrac15`, `extrac07`, `gzip`,
  `comperr`, `ignfail`, ≈10 tests) — GNU's exact wording with path
  context.
- **`--backup` on extract** (`backup01`) — rename existing file
  before overwriting.
- **File-descriptor frugality** (`extrac11`) — work within a strict
  `ulimit -n` by not holding directory handles while walking.
- **Comparison differences detection** (`difflink`, `verify`,
  `truncate`) — detailed per-member diff reporting.
- **`--use-compress-program`, `--checkpoint`** — not implemented.

## Approach

Each phase from here on adds one of the above features and its
diagnostic output. After each phase commit and rerun the suite:

```sh
for t in $(ls /path/to/tests/*.at | xargs -n1 basename | sed s/.at$//); do
  nix build ".#checks.x86_64-linux.rust-tar-test-$t" 2>&1 |
    grep -q "error:" && echo "FAIL $t" || echo "PASS $t"
done
```

## Running tests

```sh
nix build .#checks.x86_64-linux.rust-tar-test-{name}
nix log .#checks.x86_64-linux.rust-tar-test-{name}
```

See `default.nix` for the full list of 225 test names.
