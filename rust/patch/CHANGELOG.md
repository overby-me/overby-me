# Changelog

All notable changes to `rust-patch`. Tests refer to the GNU patch 2.8 suite.

## Unreleased

### Filename handling

- Reject target filenames with embedded `\n`; emit GNU's exact
  `patch: **** Can't rename file ab.XXXXXX to 'X' : Invalid byte sequence`
  (or `Can't create file 'X' : ...` under `-o`) and exit 2.
- Single-quote filenames with whitespace, quotes, or shell meta-chars in
  `patching file …` announcements and error messages.
- Reject patch input containing NUL bytes with a line-number diagnostic.
- `--merge` reports "not implemented" and exits 2 so the upstream merge
  test detects the absence and skips.

### Git-format diffs

- Parse `diff --git a/X b/Y`, keeping the `a/`/`b/` prefixes so `-pN`
  counts them as real path components.
- Parse `new file mode`, `deleted file mode`, `old mode`, `new mode`,
  `rename from/to`, `copy from/to`.
- `index aaa..bbb NNN` picks up the shared mode for classification
  (e.g. symlink detection) without applying it as a mode change.
- `Binary files X and Y differ` marker tracked; a deleted-binary patch
  refuses with "Not deleting file X as content differs from patch".
- Symlink-mode patches (`120000`): create/modify/delete via
  `os::unix::fs::symlink`; hunk body is read as the link target.
- `--backup` of a symlink mirrors the original link in `.orig` (so
  `echo x > f.orig` writes through the backed-up target like GNU).
- `-R` flips git rename/copy direction; `resolve_target_file` picks the
  reversed source/destination.

### Rename / copy

- Pre-scan patches and seed an `original_cache` with every
  rename/copy source and destination, so later patches see the pre-run
  content instead of whatever an earlier patch wrote.
- Criss-cross renames handled by writing cached originals rather than
  using `fs::rename`; the source is left in place when a later patch
  will overwrite it.
- Copy-after-modify uses the cached original of the source (POSIX
  concatenation rule).
- `--backup` for rename/copy creates destination backups (empty file
  when the destination was absent).

### Output rules

- `-o FILE` with multiple patches targeting the same source concatenates
  the intermediate versions (POSIX "concatenated versions" rule).
- Source cache feeds subsequent patches their in-memory intermediate
  content; creation patches (`--- /dev/null`) skip the cache so
  delete → create → edit chains start fresh.
- Preserve explicit `/dev/null` on the `+++`/`---` line over git header
  paths so deletion semantics kick in on `diff --git` + `+++ /dev/null`.
- Epoch timestamp (1970-01-01 00:00:00) on a header line is treated
  as a deletion marker.

### File create / delete

- Detect `/dev/null` on the old side → create the file; on the new side
  → delete when all hunks empty the target.
- Deletion patch that leaves non-empty content prints
  "Not deleting file X as content differs from patch" and preserves the
  partial result.
- `-E`/`--remove-empty-files` removes files that become empty (plus the
  now-empty parent directories).
- `--posix` deletion leaves the now-empty file in place.
- "Would create the file X, which already exists! Applying it anyway."
  pre-message for creation patches whose target already exists.
- "Would empty out the file X, which is already empty!" pre-message.
- `-t` (batch) triggers "Assume -R" for the would-empty-already-empty
  case and flips the hunk direction.
- `-R` creation no longer emits "would create, already exists" fallout.

### Fuzz / offset reporting

- Cumulative line-delta tracked across hunks so offsets are reported
  relative to the cumulative drift from prior hunks.
- Asymmetric-context fuzz heuristic: when leading > trailing context
  and the hunk is not at line 1, report the asymmetry as fuzz
  (matches upstream `asymmetric-hunks` test 4).
- Pre-apply file length used for the "past EOF" check so add-at-EOF
  hunks no longer falsely fuzz.
- Reject files written in the matching format (or explicit
  `--reject-format`); `-r FILE` redirects all rejects to one file.

### CRLF handling

- Per-file decision: if the target lacks CRs but hunk lines have them,
  strip CRs at match time and emit
  `(Stripping trailing CRs from patch; use --binary to disable.)` once
  per run.
- `--binary` preserves CRs in both hunks and file_lines so literal CR
  content matches exactly; hunk failure gets the
  `(different line endings)` suffix when a CR-mismatch is the cause.
- Normal-diff parser is CR-tolerant on the `---` separator.
- ed-style scripts split on `\n` only so CR-carrying content reaches
  `ed` unchanged.

### Path / filename resolution

- `-p0` and unspecified `-p` behave correctly: `strip_path_opt`
  distinguishes "explicit `-pN`" from "auto" (take basename).
- `-pN` on a path with no prefix components strips to empty (fixes
  `--- f / +++ f` + `-p1` case).
- Absolute paths inside the current working directory (including
  cwd=`/`) no longer flagged as dangerous.
- Path walker resolves chained symlinks iteratively (up to 40 hops)
  to detect "Invalid file name" when a parent symlink's resolved
  target falls outside cwd.
- ELOOP (OS error 40) reported as
  `Invalid file name X -- skipping patch`.
- `--follow-symlinks` replaces the symlink with a regular file at the
  link path rather than writing through to the target.

### Options / flags

- `-t`/`--batch`, `--posix`, `--binary`, `--follow-symlinks`,
  `--set-utc`, `--set-time`, `--read-only={ignore,warn,fail}` parsed.
- Combined short flags (`-sNb`, `-p0`) handled via per-char loop.
- `-e`/`--ed` + auto-detection: positional file plus ed-shaped input
  (no unified/context/normal markers) is piped to `ed` silently.
- Reject-file write unlinks any existing file first so a fresh umask
  applies to the new mode (fixes the umask=027 reject test).
- `--set-utc` parses `YYYY-MM-DD HH:MM:SS` (with optional fractional
  seconds and `±HH:MM` offset) and calls `File::set_modified` on the
  target when all hunks succeed.
- "Not setting time of file X (time mismatch)" emitted when any hunk
  fails under `--set-utc`/`--set-time`.

### Output formatting

- Strip leading `./` from filenames in user-facing messages.
- `checking file` under `--dry-run`, `patching file` otherwise; symlinks
  announce as `patching symbolic link X`.
- Hunk-success messages: `Hunk #N succeeded at L.`, plus combinations
  with `(offset X line[s])` and/or `with fuzz F` as appropriate.
- Hunk-failure messages: `Hunk #N FAILED at L.` and the final
  `N out of M hunks FAILED -- saving rejects to file X.rej` (singular
  `hunk` when M == 1).

### Infrastructure

- `testsuite.nix` runs each upstream script in a per-test Nix sandbox
  with `$PATCH` pointed at the rust-patch binary; 60s timeout.
- `default.nix` lists all 49 upstream test names as flake checks.
