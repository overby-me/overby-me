# rust-tar: Plan to Pass All Upstream GNU tar Tests

## Current status

**206/225 tests passing (92%)**.

Trajectory: 92 → 172 → 182 → 187 → 191 → 195 → 200 → 206. Each
per-test derivation is wired as a flake check via the shared
`gnutar-test-harness` (autom4te-built `testsuite` script + helper
programs).

## Running tests

```sh
nix build .#checks.x86_64-linux.rust-tar-test-{name}
nix log .#checks.x86_64-linux.rust-tar-test-{name}
```

See `default.nix` for the full list of 225 test names.

### What works

- Core ops: create / list / extract / diff / append / update /
  delete / `--test-label`, for regular files, directories,
  symlinks, and hard links.
- Positional `-C DIR` in create, append, and extract, plus inside
  `-T` files, each path carrying its own chdir context.
- Hard-link detection: a `(dev, inode)` map flags the second
  occurrence as a `Link` entry with the linkname pointing at the
  first archived path.
- Scoped `--transform` / `--xform`: `H` excludes hard-link targets,
  `S` excludes symlink targets; defaults match GNU 1.35 (transforms
  apply to every name kind).
- Volume labels: `-V LABEL` / `--label=` write a GNU `V` block as
  the first entry; extract, append/update, and `--test-label`
  fnmatch-verify.
- Options: `--ignore-failed-read`, `--keep-old-files` / `-k`,
  `--skip-old-files`, `--backup`, `--remove-files`, `--verify` /
  `-W`, `--to-stdout` / `-O`, `--one-top-level[=DIR]`,
  `--show-transformed-names`, `--no-overwrite-dir`,
  `--clamp-mtime`, `--occurrence` validation, `--index-file=FILE`.
- Excludes: `--exclude`, `--exclude-from`, `--exclude-caches`
  variants, `--exclude-tag` variants, `--exclude-backups`,
  `--exclude-vcs`.
- Match modifiers: `--wildcards` / `--no-wildcards`, `--anchored` /
  `--no-anchored`, `--ignore-case`, `--wildcards-match-slash`.
- Owner/group: `--owner-map=FILE`, `--group-map=FILE`,
  `--owner=NAME[:UID]`, `--group=NAME[:GID]`, `--numeric-owner`.
- Time/mode: `--mtime=@SECONDS|ISO`, `--mode=EXPR`,
  `--preserve-permissions`, `--no-same-permissions`.
- Positional `--no-recursion` / `--recursion` for create (and
  `--no-recurs` / `--no-recur` abbreviations).
- Positional-option warnings for `--exclude` and trailing `-C`
  with GNU's exact header and `Exiting with failure status due
  to previous errors`.
- Raw GNU header path writes bypass the tar crate's `..` /
  absolute-path validation; long names trigger GNU `LongLink`
  blocks.
- `-T -` (read from stdin), nested `-T FILE` inside a `-T` file
  with recursion detection, `--null` with auto-fallback when a
  stray NUL appears.
- Archive-can't-contain-itself check so `tar cf a.tar .` doesn't
  recurse into the growing archive.
- Fast-path exclude filter: literal patterns go through per-path
  and per-basename `HashSet`s, keeping exclude05's 1M-line file
  under the 60s harness budget.
- Non-printable bytes are octal-escaped in both `-t` and `-vc`
  listings (GNU format).
- `--remove-files` cleans up the positional `-C` chdir root when
  `.` is given as the path, and emits GNU's `Cannot rmdir .` +
  exit 2 when run with no positional `-C`.
- Extract: deferred directory-mode restore for read-only dirs;
  `--overwrite` + symlink handling honours `-h`; `--backup`
  renames to `NAME~`; mkdir failures emit the GNU `Cannot mkdir`
  / `Cannot open` pair using the archive-relative name.
- Diff: `Not linked to X`, `Symlink differs`, `Mod time differs`,
  `Contents differ` with GNU wording; directory mtime left out so
  child changes don't taint the parent.
- Compressor error translation: empty gzip / bzip2 / xz streams
  surface as `Child returned status 1` + `Error is not
  recoverable` instead of `unexpected end of file`.
- Short-file detection; `--delete NAME` + missing member;
  `--delete 'dir/'` prefix match; `--pax-option` on non-POSIX
  archive; GNU-style strerror translation for the common
  `io::ErrorKind` values.
- `--no-recursion` / `--recursion` directives inside `-T` file
  content; list/extract matching gates prefix (descendant) matches
  per user-path so `tar tf archive --no-recursion dir1 --recursion
  dir2` filters correctly.
- `--wildcards` expands filesystem globs in path arguments during
  create/append/update. Patterns that match nothing on disk are
  checked against archive members in update mode; a pattern that
  matches neither emits `Not found in archive` + exit 2.
- `-l` / `--check-links`: nlink + archived-count tracking emits
  `Missing links to 'PATH'.` when a multi-link file is archived
  without all of its peer hard links.
- Format-aware name-length enforcement: V7 rejects names > 99 chars
  (`file name is too long (max 99); not dumped`); strict ustar
  rejects unsplittable names > 100 chars (`cannot be split`).
  Posix/pax fall through (long names ride on PAX extended headers).
- `--use-compress-program PROGRAM` / `-I PROGRAM` spawns an external
  compressor (whitespace-split argv); a non-zero child status
  surfaces `Error is not recoverable: exiting now` and exit 2.
  Built-in gzip/bzip2/xz compressors tag file-open failures with a
  `tar (child):` prefix and bail before `--remove-files` runs on a
  half-built archive.
- `--keep-directory-symlink`: at a directory entry whose destination
  is a symlink-to-dir, keep the symlink and let children extract
  through it. Default now replaces symlinks with real directories
  (previously children were silently landing in the symlink's
  target). `--keep-old-files` errors now show archive-relative paths.
- WalkDir `.max_open(3)` so creation survives `ulimit -n 10`
  environments (extrac11).
- `--checkpoint=N` + `--checkpoint-action=echo=FMT` +
  `--checkpoint-action=wait=SIGNAL`: a `CheckpointStream` wraps the
  tar stream (write for create/append, read for diff), counts
  10240-byte records, and fires each action every N records. Echo
  substitutes `%u` and emits to stderr; wait installs a libc signal
  handler (at parse time, before any checkpoint races with a reply)
  and blocks in a `pause()` + atomic-flag loop. This unlocks the
  `genfile --run` synchronization used by grow/truncate/sptr*.
- `PaddedReader` keeps the archive valid when a source file shrinks
  during read: once the underlying file hits EOF we return zeros up
  to the declared size.
- `tar d` comparison now re-stats the on-disk file AFTER reading the
  archive side, so a concurrent truncation reports `Size differs`
  rather than `Contents differ`.

### Recent: `--listed-incremental`

Substantial implementation of the incremental feature:

1. **Bootstrap (+4 tests):** snapshot-file I/O (GNU format 2 header +
   time + per-dir records), level-0/level-N+1 file-level time
   filtering, GNU dumpdir ('D') directory entries carrying child
   listings as the entry body, and extract-side deletion of disk
   children not mentioned in the dumpdir (gated on `-v` for the
   message). Deferred directory mtime restore so child writes don't
   clobber parent timestamps. Unlocks listed01, incr01, incr02,
   incremental.
2. **Per-dir dumpdir state (+4 tests):** snapshot now carries the
   dumpdir each directory wrote last run. On create we look up the
   current dir's (dev, inode) in the snapshot, match each child's
   name against the previous dumpdir, and mark unchanged files 'N'
   (skipped from this run's archive) vs new/changed 'Y' (archived).
   Time comparisons use both sec + nsec so same-second creations
   don't false-positive as "changed". Snapshot path is canonicalised
   pre-chdir so `-g db -C dir` lands relative to the invocation cwd.
   Listed-incremental walks now emit all dirs first, then files
   ordered by parent dir, matching GNU's directory-first layout.
   Unlocks incr03, incr05, rename04, rename05.

### Recent: rename detection + new-dir warnings

- `--warning=no-<name>`: parse and apply to `new-dir`,
  `rename-directory`, so verbose create/append respects the
  suppression flags used by several test harnesses.
- Under `-v` on level-N+1 runs: emit `tar: PATH: Directory is new`
  when a current dir's (dev, inode) isn't in the prev snapshot, and
  `tar: PATH: Directory has been renamed from 'OLD'` when the inode
  match surfaces under a different name.
- On create, detect subdir renames by matching (dev, inode) against
  the previous-snapshot `dirs` map; when the inode surfaces under a
  new basename in this dir, emit GNU dumpdir codes `R <old>` +
  `T <new>` in the parent's dumpdir. On extract, process R/T pairs
  before the delete sweep so the on-disk source moves into place
  before any unrelated cleanup runs.

### Recent: exclude-tag + incremental interop

- Move the `Directory is new` / `Directory has been renamed from`
  diagnostics ahead of the cache-tag `contains a cache directory
  tag ...; contents not dumped` note so the stderr stream stays in
  directory-first order.
- Suppress the trailing `/` on the cache-tag path under
  listed-incremental (matches GNU) while keeping it for standalone
  `--exclude-tag` runs (those tests still expect it).

### What remains (19 failing)

| Bucket | Tests | Notes |
| --- | --- | --- |
| `--listed-incremental` — advanced | 12 | incr06/08/09/10, rename02/03/06, dirrem01/02, filerem01, remfiles08b/09b. Remaining gaps: `File removed before we read it` warnings wired to on-disk delete during walk, deeper multi-dir walk invariants (incr06 mixes subdir first then parent), and several rename corner cases. |
| Multi-volume (`-M`, `--tape-length=N`, `--new-volume-script`) | 7 | multiv03/04/05/08/09, label02, sparsemvp — continuation headers, volume boundary handling. |

## Approach

Continue chipping at the incremental edge cases. Multi-volume is a
self-contained project: continuation headers + volume switching +
`-M` on extract.

After each phase commit and rerun the suite:

```sh
for t in $(ls /path/to/tests/*.at | xargs -n1 basename | sed s/.at$//); do
  nix build ".#checks.x86_64-linux.rust-tar-test-$t" 2>/dev/null &&
    echo "PASS $t" || echo "FAIL $t"
done
```
