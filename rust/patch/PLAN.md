# rust-patch: Plan to Pass All Upstream GNU patch Tests

## Current status

**2/49 tests passing** (baseline after adding upstream test integration).

Passing: `merge`, `inname`.

All others fail for a mix of output-formatting, argument-parsing, and
missing-feature reasons. Tests compare rust-patch output against a verbatim
GNU expected-output string embedded in each test script, so even cosmetic
diffs cause failure.

Run one test: `nix build .#checks.x86_64-linux.rust-patch-test-{name}`
See the failure diff: `nix log .#checks.x86_64-linux.rust-patch-test-{name}`

---

## Failure categories

The numbers in parentheses are the approximate number of tests affected —
many tests fail for multiple reasons, so the categories overlap.

### Category 1 — Output formatting (~40 tests)

Our messages diverge from GNU patch:

- We print `patching file ./a` (leading `./`); GNU prints `patching file a`.
- `--dry-run` should print `checking file <name>`, not `patching file`.
- Hunk messages: GNU says `Hunk #1 succeeded at 5.` or
  `Hunk #3 succeeded at 5 (offset -1 lines).`. We print
  `Hunk #3 succeeded at offset 1 (fuzz 0).` — wrong word order and we
  always emit fuzz/offset even when zero.
- Failure message format differs: GNU says `Hunk #2 FAILED at 3.` without
  the trailing summary line when non-verbose; we emit
  `1 out of 3 hunks FAILED` in all modes.

### Category 2 — Argument parsing (~5 tests)

- `-px` where `x` is not a number should exit 2 with
  `patch: **** strip count x is not a number`. We silently take the default
  strip and later print `patch: no valid patches found in input`.
- Combined short flags (`-pN0`) need full handling.
- `--read-only={ignore,warn,fail}` is not parsed.
- `--posix` is not parsed.
- `--remove-empty-files` / `-E` edge cases (currently partial).

### Category 3 — Reject files (~6 tests)

When a hunk fails, GNU writes the rejected hunk(s) to `<file>.rej` in the
same format as the input patch. We don't write reject files at all.

Tests: `reject-format`, `corrupt-reject-files`, `global-reject-files`,
`remember-reject-files`, `read-only-files` (partially), `false-match`.

### Category 4 — Backup files (~5 tests)

- `-b`/`--backup` creates `<file>.orig` before patching. Partial support.
- `--backup-suffix=<s>`, `--backup-prefix=<p>`, `-B <p>` not supported.
- `--no-backup-if-mismatch` interacts with reject handling.
- Numbered / simple backup via `VERSION_CONTROL` / `PATCH_VERSION_CONTROL`
  not supported.
- `-b` implies saving even when `-N` skips already-applied patches.

Tests: `backup-prefix-suffix`, `no-backup`, `remember-backup-files`,
`remember-reject-files`, parts of `create-delete`.

### Category 5 — Git-format diffs (~5 tests)

- `diff --git a/... b/...` header handling.
- `new file mode`, `deleted file mode`, `old mode`/`new mode` lines.
- `rename from`/`rename to`, `copy from`/`copy to`.
- `index <hash>..<hash>` lines.
- Binary patches (`GIT binary patch`, base85-encoded deltas).

Tests: `git-binary-diff`, `concat-git-diff`, `git-cleanup`,
`no-mode-change-git-diff`, `copy-rename`.

### Category 6 — ed-style diffs (~2 tests)

`-e`/`--ed` mode: apply an `ed(1)` script rather than a unified/context
diff. We don't implement this at all.

Tests: `ed-style`, parts of `mixed-patch-types`.

### Category 7 — File create/delete semantics (~5 tests)

- Creating a new file from a patch with `/dev/null` as the old side.
- Deleting an empty file (patch removes all content). We return
  `no valid patches found in input` instead.
- `-E` removes the file when the result is empty; we currently only check
  after writing.
- Creating intermediate directories.
- Refuse to create when the file would have only garbage.

Tests: `empty-files`, `create-delete`, `create-directory`,
`remove-directories`, `deep-directories`.

### Category 8 — Symlinks and hardlinks (~2 tests)

- Patching a symlink should follow the target (or rewrite the symlink
  target when the patch describes a symlink).
- `-i dir/l` where `l` is a symlink to a directory.
- Creating symlinks from a git-format diff.
- Hardlinks must be preserved/broken correctly.

Tests: `symlinks`, `hardlinks`.

### Category 9 — File modes / timestamps (~3 tests)

- Preserve file permissions through the patch operation.
- `file-create-modes`: newly-created files honor the mode in the patch.
- `--preserve-mode-and-timestamp` copies over mtime from original.

Tests: `file-modes`, `file-create-modes`, `preserve-mode-and-timestamp`.

### Category 10 — Fuzz / offset matching (~6 tests)

The algorithm disagrees with GNU's in corner cases:

- GNU reports `offset N lines` (signed) and `fuzz N` separately; we conflate.
- Our fuzz levels don't strip the same leading/trailing context lines.
- Line-number drift across consecutive hunks: subsequent hunks should use
  the offset from prior matches.
- Asymmetric hunks (different old/new count).

Tests: `line-numbers`, `asymmetric-hunks`, `no-newline-triggers-assert`,
`false-match`, `unmodified-files`, `criss-cross`.

### Category 11 — Input edge cases (~5 tests)

- `corrupt-patch`: detect and report `malformed patch at line N`.
- `mangled-numbers-abort`: abort on impossible hunk counts.
- `garbage`: tolerate leading garbage before the first hunk.
- `munged-context-format`: context diffs with odd spacing.
- `unusual-blanks`: blanks-only context lines.

### Category 12 — Special filenames (~3 tests)

- `quoted-filenames`: unified header `+++ "foo\tbar"` uses C-style
  escapes; must parse and un-escape.
- `bad-filenames`: refuse filenames containing `\n`, `\0`, or with
  directory traversal (`..`).
- `filename-choice`: rules for which of `---`/`+++` to pick.

### Category 13 — CRLF handling (~2 tests)

- `crlf-handling`: treat `\r\n` as a line terminator and preserve it in
  output.

### Category 14 — Miscellaneous (~5 tests)

- `-o FILE` — write patched output to `FILE` instead of in-place
  (currently only partial; fails `dash-o-append`).
- `-i FILE` — read patch from `FILE` (currently partial; fails
  `inname` interactions when combined with `-d`).
- `preserve-c-function-names` — keep `@@ ... foo() @@` hunk suffix.
- `need-filename` — prompt behaviour when the file can't be determined.
- `global-reject-files` — `-r FILE` to redirect all rejects to one file.

---

## Implementation plan

Ordered so that each phase unblocks as many tests as possible with
minimal scope creep. Tests counts are rough estimates — many tests span
multiple categories.

### Phase 1 — Output formatting (~15 tests)

Quickest wins. Change the wording without changing behaviour.

- Strip leading `./` from filenames passed through `resolve_target_file`
  for display.
- Emit `checking file <name>` under `--dry-run`, `patching file <name>`
  otherwise.
- Rewrite hunk success messages to match GNU exactly:
  - exact match: `Hunk #N succeeded at L.`
  - offset only: `Hunk #N succeeded at L (offset X lines).`
  - fuzz:                `Hunk #N succeeded at L with fuzz F.`
  - both:                `Hunk #N succeeded at L (offset X lines) with fuzz F.`
- Fix failure summary: only emit `N out of M hunks FAILED` when `N > 0`.

### Phase 2 — Argument parsing (~5 tests)

- Validate the argument to `-p` and error with
  `patch: **** strip count <arg> is not a number` → exit 2.
- Parse `--read-only=<ignore|warn|fail>` (ignore is default in most cases;
  `warn` prints a message; `fail` refuses to write).
- Parse `--posix`.
- Parse `-B <prefix>`, `--backup-suffix=<s>`, `-z <s>`.
- Parse `-r <file>` / `--reject-file=<file>`.
- Treat repeated `-p` flags as the last-wins.

### Phase 3 — Fuzz / offset reporting (~6 tests)

- Track the cumulative offset across hunks within a file; report each
  hunk's offset relative to the prior expected position.
- Report fuzz and offset independently in the success message (both
  possibly zero).
- Re-check our fuzz-level algorithm: on fuzz K, strip K context lines
  from both ends, try the match, then fall through to K+1.
- Add a test-style harness (`cargo test`) that replays a couple of the
  shell tests against the Rust binary for faster iteration.

### Phase 4 — Reject files (~6 tests)

- When a hunk fails, append it to `<target>.rej` in the same format as
  the input diff.
- Honor `-r FILE` to redirect all rejects.
- Respect `--reject-format={context,unified}` for output format.
- Exit code is still `1` if any hunk failed.

### Phase 5 — Backup files (~5 tests)

- `-b` default: save `<file>.orig` before the first write.
- `--backup-suffix=S`, `-z S`: use `<file><S>` instead of `.orig`.
- `--backup-prefix=P`, `-B P`: prepend `<P>` to the filename.
- `VERSION_CONTROL`/`--backup-if-mismatch` simple/numbered/existing.
- `--no-backup-if-mismatch`: suppress backups on failed hunks.
- `-Y <prefix>`: prepend per-file backup prefix.

### Phase 6 — File create/delete (~5 tests)

- Detect `/dev/null` on the old side → create new file.
- Detect `/dev/null` on the new side → delete the file when all hunks
  remove it.
- `-E`: remove files that become empty.
- `mkdir -p` intermediate directories as needed.

### Phase 7 — Input edge cases (~5 tests)

- Detect `malformed patch at line N` and abort with exit 2.
- Accept leading garbage before the first hunk (`garbage`).
- Reject patches with impossible hunk counts (`mangled-numbers-abort`).
- Handle unusual blank lines in context diffs.

### Phase 8 — Git-format diffs (~5 tests)

- Parse `diff --git a/... b/...` preamble.
- Parse `new file mode` / `deleted file mode` / `old mode` / `new mode`.
- Parse `rename from` / `rename to` / `copy from` / `copy to` —
  perform the rename/copy, then apply diffs.
- Parse `index <hash>..<hash>` and ignore.
- Defer binary patches — emit `patch: **** can't handle binary patches`
  and exit 2 for now.

### Phase 9 — Quoted and bad filenames (~3 tests)

- When `--- "..."`/`+++ "..."` is quoted, un-escape `\n`, `\t`, `\\`,
  `\"`, `\xHH`.
- Refuse to write to a filename containing `\n`, `\0`, or attempting
  directory traversal outside `-d`.

### Phase 10 — File modes, symlinks, hardlinks (~5 tests)

- Preserve mode (and optionally mtime) across patch writes.
- Apply git `new file mode` to newly-created files.
- Handle `-i file` where `file` is a symlink to a directory.
- Respect symlinks in the patched path.
- Break hardlinks before writing (cow semantics).

### Phase 11 — ed-style and miscellaneous (~5 tests)

- Implement `-e`/`--ed` to emit an `ed(1)` script (pipe to `ed -`).
- `-o FILE` append semantics (`dash-o-append`).
- `preserve-c-function-names`: when the hunk header has a trailing
  function-name heuristic, keep it in reject output.

### Phase 12 — CRLF (~2 tests)

- Detect `\r\n` line endings on input and preserve them on output.
- `--binary` mode.

### Phase 13 — Polishing

- Revisit each failing test after earlier phases; many tests touch
  multiple categories and will start passing incrementally.
- Add more `cargo test` unit tests once the visible test harness is
  stable.

---

## Test inventory

### Passing (2)

`merge`, `inname`.

### Failing (47)

All tests in `default.nix` except the two above. See the failure
categories above for grouping.
