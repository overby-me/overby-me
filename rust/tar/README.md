# rust-tar

A pure-Rust reimplementation of GNU `tar(1)` that aims to be output-
compatible with the upstream tool. Passes **224/224** tests from the
GNU tar 1.35 test suite.

## Building

```sh
nix build .#rust-tar
./result/bin/tar --help
```

A debug build is also available as `.#rust-tar-dev` for quick iteration.

## Running the test suite

Tests are run in a Nix sandbox. Each test comes from the GNU tar 1.35
source tarball; the upstream `tests/testsuite` (autom4te-built) runs the
selected test ID with `TAR` pointed at rust-tar. A shared
`gnutar-test-harness` derivation prebuilds the harness once.

```sh
# Run a single test
nix build .#checks.x86_64-linux.rust-tar-test-{name}

# View failure diff
nix log .#checks.x86_64-linux.rust-tar-test-{name}

# Run the full 224-test matrix at -j8
awk '{print ".#checks.x86_64-linux.rust-tar-test-"$1}' names.txt \
  | xargs nix build --max-jobs 8 --keep-going --no-link
```

See `default.nix` for the full list of test names. Tests time out after
600 s (heavy sparse + checkpoint tests like `sptrcreat` / `sptrdiff01`
need the headroom under high-parallel `nix build` swarms).

## Supported features

All GNU tar 1.35 surface area exercised by the upstream test suite,
including:

### Core operations
- `c` / `t` / `x` / `d` / `r` / `u` / `--delete` / `--test-label` for
  regular files, directories, symlinks, and hard links.
- Hard-link detection via `(dev, inode)` map; second occurrence becomes
  a `Link` entry pointing at the first archived path.
- Volume labels (`-V LABEL` / `--label=`) written as a leading `V`
  block; extract / append / update / `--test-label` fnmatch-verify.

### Path and directory handling
- Positional `-C DIR` in create, append, and extract, plus inside `-T`
  files (each path carries its own chdir context).
- `-T -` (stdin), nested `-T FILE` with recursion detection, `--null`
  with auto-fallback when a stray NUL appears.
- Archive-can't-contain-itself check so `tar cf a.tar .` doesn't recurse
  into the growing archive.
- Positional `--no-recursion` / `--recursion` (and `--no-recurs` /
  `--no-recur` abbreviations).

### Excludes and matching
- `--exclude`, `--exclude-from`, `--exclude-caches[*]`, `--exclude-tag[*]`,
  `--exclude-backups`, `--exclude-vcs`.
- Match modifiers: `--wildcards` / `--no-wildcards`, `--anchored` /
  `--no-anchored`, `--ignore-case`, `--wildcards-match-slash`.
- Fast-path literal-pattern filter (per-path / per-basename `HashSet`s)
  keeps `exclude05`'s 1M-line pattern file under the harness budget.

### Owner / group / time / mode
- `--owner-map=FILE`, `--group-map=FILE`, `--owner=NAME[:UID]`,
  `--group=NAME[:GID]`, `--numeric-owner`.
- `--mtime=@SECONDS|ISO`, `--clamp-mtime`, `--mode=EXPR`,
  `--preserve-permissions`, `--no-same-permissions`.

### Transforms and naming
- Scoped `--transform` / `--xform`: `H` excludes hard-link targets, `S`
  excludes symlink targets; defaults match GNU 1.35.
- Format-aware name-length enforcement: V7 rejects > 99 chars; strict
  ustar rejects unsplittable > 100 chars; posix/pax ride PAX extended
  headers.
- Raw GNU header path writes bypass the tar crate's `..` / absolute-path
  validation; long names trigger GNU `LongLink` blocks.

### Compression
- Built-in `--gzip` / `--bzip2` / `--xz` plus `-I PROGRAM` /
  `--use-compress-program` external compressors. Non-zero child status
  surfaces GNU's `Error is not recoverable: exiting now`. Built-in
  compressors tag file-open failures `tar (child):` and bail before
  `--remove-files` runs on a half-built archive.
- Empty gzip / bzip2 / xz streams report `Child returned status 1`
  rather than `unexpected end of file`.

### Multi-volume
- `-M` / `--multi-volume`, `-L N` / `--tape-length=N` (with `K/M/G`),
  multiple `-f FILE` slots, `-R` / `--block-number`, bundled short forms
  for `M` / `R` / `n` / `w`.
- Create: `Vec<u8>`-backed split with proper GNU `M` continuation
  headers (mid-entry split markers and zero-size trailers as required).
  `--label=X -M` writes `X Volume 1`.
- Extract / list: stream concatenation that strips leading `M` blocks,
  with straddle detection for pax-format entries that fit
  header + partial data plus padding in one volume but continue in the
  next. `-tMR` walks volumes directly with proper block counts.

### Sparse files
- `--sparse` / `-S` actually emits sparse entries (oldgnu `S` typeflag
  with inline + chained `GnuExtSparseHeader` blocks).
- `--sparse-version=0.0|0.1|1.0` selects PAX-encoded sparse maps via
  `GNU.sparse.*` keys.
- `--hole-detection=raw|seek` chooses between byte-scan (always finds
  512-byte holes) and SEEK_HOLE/SEEK_DATA (faster but block-granular).
- Sparse multi-volume: parser walks the chain of ext sparse blocks so
  the splitter and stitcher keep the sparse map intact across volume
  boundaries.

### Listed-incremental (`-g` / `--listed-incremental`)
- Snapshot file I/O (GNU format 2 header + time + per-dir records).
- Per-dir dumpdir state in snapshot; child entries marked `Y`
  (changed/new), `N` (unchanged), `D` (directory).
- Directory rename detection by `(dev, inode)` match → `R old` / `T new`
  dumpdir codes; extract-side staged temp-rename pass survives cyclic
  / chained renames.
- Extract-side delete sweep removes disk children not mentioned in the
  dumpdir (gated on `-v`).
- `Directory is new` / `Directory has been renamed from` warnings
  (suppressible via `--warning=no-new-dir` / `no-rename-directory`).
- `File removed before we read it` warning when an entry vanishes
  mid-walk; the report walks up parents to surface the topmost gone
  ancestor.
- Two-pass walk (dirs then files) when positional `-C` sentinels are
  present; global cross-source sort with per-entry CWD tracking.
- Per-source dedup: if a directory's `(dev, inode)` is owned by a
  different source argument, it (and its descendants) are skipped, with
  owner tracking so the actual owning source is not locked out.
- Absolute-path source preservation in incremental mode (no leading
  `/` strip).
- `--incremental` / `-G` standalone (no snapshot file) accepted.

### Concatenate / catenate
- `-A` / `--catenate` / `--concatenate` raw-byte-copies source archive
  contents (up to EOF marker) onto the destination, then writes a new
  two-block terminator.

### Diff / extract
- Diff: `Not linked to X`, `Symlink differs`, `Mod time differs`,
  `Contents differ` with GNU wording; directory mtime omitted so child
  changes don't taint the parent.
- `tar d` comparison re-stats the on-disk file after reading the
  archive side, so a concurrent truncation reports `Size differs`.
- `PaddedReader` keeps the archive valid when a source file shrinks
  during read.
- Deferred directory-mode restore for read-only dirs; `--overwrite` +
  symlink handling honours `-h`; `--backup` renames to `NAME~`; mkdir
  failures emit GNU's `Cannot mkdir` / `Cannot open` pair using the
  archive-relative name.
- `--keep-directory-symlink` keeps a symlink-to-dir at a directory
  entry; default replaces it with a real directory.

### Checkpoint and signals
- `--checkpoint=N` + `--checkpoint-action=echo=FMT` /
  `--checkpoint-action=wait=SIGNAL`: a `CheckpointStream` wraps the
  archive read/write, counts `blocking_factor × 512`-byte records, and
  fires each action every N records. The signal handler is installed
  at parse time; `Wait` actions block in a `pause()` + atomic-flag
  loop. Sparse / GNUSparse diff also fires checkpoints per disk-side
  chunk so `genfile --run --checkpoint N` triggers correctly when the
  on-tape data is tiny but realsize is large.
- `--blocking-factor=N` / `-b N` sets the record size used by both the
  archive and the checkpoint counter.

### Misc options
- `--ignore-failed-read`, `--keep-old-files` / `-k`,
  `--skip-old-files`, `--backup`, `--remove-files`, `--verify` / `-W`,
  `--to-stdout` / `-O`, `--one-top-level[=DIR]`,
  `--show-transformed-names`, `--no-overwrite-dir`, `--occurrence`,
  `--index-file=FILE`.
- `-l` / `--check-links` emits `Missing links to 'PATH'.` when a
  multi-link file is archived without all peer hard links.
- Non-printable bytes octal-escaped in `-t` and `-vc` listings.
- `--warning=no-<name>` parses for `new-dir`, `rename-directory`,
  `file-removed`.

## Known limitations

- A handful of tests (`exclude05`, `sptrdiff01`, `sptrcreat`,
  `sparse03/05/06`, sparse-MV cluster) are wall-clock heavy: 200 MB
  sparse round-trips with checkpoint sync, run three times each
  (posix/gnu/oldgnu). They pass at `--max-jobs 8` with the 600 s harness
  timeout but may need longer under heavier contention.

## Layout

```text
rust/tar/
  Cargo.toml
  default.nix       # Nix package + harness + per-test flake checks
  testsuite.nix     # Per-test Nix sandbox runner
  CHANGELOG.md
  README.md
  src/
    main.rs         # Single-file implementation
```
