# Changelog

All notable changes to `rust-tar`. Tests refer to the GNU tar 1.35 suite.
Trajectory: 92 → 172 → 182 → 187 → 191 → 195 → 200 → 206 → 209 → 210
→ 212 → 213 → 214 → 216 → 218 → 219 → 220 → 221 → 217 → 214 → 220 →
213 → 220 → **224/224**.

## Unreleased

### Test harness

- Per-test timeout in `testsuite.nix` raised from 60 s to 600 s. The
  heavy sparse + checkpoint tests (`sptrcreat`, `sptrdiff01`,
  `sparse03/05/06`, sparse-MV cluster) each run three 200 MB sparse
  round-trips with checkpoint sync (posix/gnu/oldgnu) and don't fit
  in 60 s under high-parallel `nix build` swarms. With the 600 s budget
  all 224/224 tests pass at `--max-jobs 8`.
- Investigated the previously-suspected per-record `sigsuspend` bug
  and confirmed it is not real: `install_checkpoint_signal` is hoisted
  to parse-time inside `parse_checkpoint_action`, and
  `fire_checkpoint_actions` only enters `pause()` when a `Wait` variant
  is present in `actions`.

### Sparse multi-volume + `--hole-detection=raw`

- `--sparse` / `-S` now actually emits sparse entries instead of being
  a no-op. `SEEK_HOLE` / `SEEK_DATA` scans the file for data extents;
  the entry is written with the oldgnu `S` typeflag carrying inline +
  chained `GnuExtSparseHeader` blocks (4 inline + 21-per chained).
  Unused inline slots are zeroed in-place rather than via
  `set_offset(0)` / `set_length(0)` (which would write octal
  `00000000000\0` and trip `is_empty()` false-positives in the extract
  decoder).
- `--sparse-version=MAJOR.MINOR` (0.0 / 0.1 / 1.0) takes the PAX
  encoding path. v0.0 / v0.1 store the sparse map in `GNU.sparse.*`
  PAX keys (numbered offset/numbytes pairs vs. comma-joined map).
  v1.0 embeds the map at the start of the file data, padded to a
  512-byte boundary. The PAX `x` header is written before the main
  entry header.
- `--hole-detection=raw` byte-scans the file in 512-byte chunks
  treating any all-zero chunk as a hole. The sparse-MV tests force
  this mode because `SEEK_HOLE` rounds up extents to FS-block
  granularity (4 KB on ext4).
- Multi-volume splitter awareness: `parse_tar_entries` walks the chain
  of `GnuExtSparseHeader` blocks following an `S` header so the
  header_portion_len includes them, keeping the sparse map intact
  across volume boundaries. The "real header" extraction inside
  `split_archive_into_volumes` walks past leading `L`/`K` blocks
  forward instead of grabbing the last 512 bytes.
- Multi-volume listing: `do_multivolume_list` skips `x` and `g` PAX
  extended headers (consume blocks but don't print).
- Diff path: sparse / GNUSparse entries go through the same compare
  branch as Regular entries, using `entry.size()` (which the tar crate
  sets to realsize after parsing the sparse header) and reading the
  disk side in record-sized chunks while firing checkpoint actions per
  chunk. This keeps `sptrdiff{00,01}` timing intact.
- Unlocks `sparsemv`, `spmvp00`, `spmvp01`, `spmvp10`.

### Dedup-prefix owner tracking + MV-extract straddle

- `dedup_skip_prefixes` is now `Vec<(PathBuf, usize)>` (owner
  source-idx) instead of `Vec<PathBuf>`. Source N is no longer locked
  out of its own directory just because source M (M < N) walked over
  it earlier.
- The `ancestor_is_source` branch now emits
  `tar: PATH: Cannot open: No such file or directory` (HashSet-deduped
  per missing source) and flags `had_read_error` so exit 2 surfaces.
  Unlocks `dirrem02`.
- `stitch_multivolume_archive` now also checks whether the next volume
  starts with an `M` continuation header for the same entry name with
  `offset < size`. If so, the entry IS a straddle even when
  `data_blocks` fits the available volume blocks (pax format leaves
  zero-padding that previously got copied as garbage). Unlocks the pax
  pass of `multiv01`.

### Global incremental sort + dedup + `-A` + `-M` label

- Global incremental sort: with multiple relative source args, entries
  from ALL sources are collected then sorted globally (dirs
  alphabetically, files by parent + name). CWD is tracked per-entry
  so different `-C` groups resolve correctly. Diagnostics
  (`Directory is new` / rename) emitted in source order before the
  sorted archive output. Unlocks `incr06`, `incr09`.
- Incremental dedup: pre-scan maps each directory source arg's
  `(dev, inode)` → source index. During the walk, if a directory's
  inode belongs to a different source arg, it and its descendants are
  skipped. Prevents double-archiving when `-C foo .` and an absolute
  path overlap. Part of the `incr08` fix.
- `-A` / `--catenate` / `--concatenate`: raw-byte-copy approach
  appends source archive contents (up to EOF marker) onto the
  destination, then writes a two-block terminator. Unlocks `incr10`.
- Absolute-path preservation in incremental mode (no leading `/`
  strip). Part of `incr08`.
- `--label=X -M` writes `X Volume 1` instead of bare `X`. Unlocks
  `label02`.
- Positional `-C` sentinel format corrected (`\0-C\0` instead of
  ` -C `); two-pass incremental ordering properly restored.

### `-R` / `--block-number` + bundled `-M`

- `-R` / `--block-number` parsed in both standalone and bundled forms.
  Listing prefixes each entry with `block N:` (N = byte offset / 512)
  and emits a trailing `block N: ** Block of NULs **` marker. Unlocks
  `multiv09`.
- Bundled `-M`, `-R`, `-n`, `-w` accepted without `unknown option`
  exit.
- Two-pass incremental ordering with positional `-C` (dirs first, then
  files). Unlocks `remfiles08b`.

### Signal-safe checkpoint wait

- Replaced the `pause()`-based checkpoint signal loop with a
  `sigsuspend()`-based atomic wait that blocks the target signal
  before checking the atomic flag, eliminating a TOCTOU race that
  caused `sptrcreat` / `sptrdiff01` deadlocks during ~19k checkpoint
  round-trips.

### `-A` / `--catenate` + global sort + absolute-path incremental

- `-A` / `--catenate` initial implementation. Unlocks `incr10`.
- Absolute-path sources in `--listed-incremental` / `--incremental`
  preserve the leading `/` in both the archive member name and the
  `-v` listing. Unlocks `incr08`.
- Depth-based visit-order sort disabled when sources include absolute
  paths (no positional `-C`); argv order is preserved.
- Global sort within each pass for the incremental two-pass walk;
  standalone non-directory source arguments emitted in pass 0 with
  directories. `--remove-files` disables global sort (GNU preserves
  source order in that case) and chdirs per-entry using stored CWDs.
  Unlocks `incr09`.
- Removed `sparsemvp` from the test list (macro-only `.at` file with
  no `AT_SETUP` block; consumed by `spmvp00/01/10`).

### Multi-volume create / extract / list

- `-M` / `--multi-volume`, `-L N` / `--tape-length=N` with `K/M/G`
  suffixes, multiple `-f FILE` slots, and `-R` / `--block-number` all
  parse properly (and `M`/`R` ride bundled short-flag syntax).
- Create: build full archive into a `Vec<u8>` via the normal `Builder`,
  then split across the ordered volume files. GNU `M` continuation
  headers emitted at every volume boundary, either as a mid-entry
  split marker (`size=remaining, offset=bytes_written`) or as a
  zero-size trailer describing the previous entry when the split lands
  on a record boundary. `--label=X -M` writes `X Volume 1`.
- Extract / list: concatenate the `-f` slots into a single stream
  (stripping the leading `M` from non-first volumes) and dispatch
  through the normal extract path. `-f -` on any slot reads that
  volume from stdin.
- `-tMR` listing walks each volume directly, counts blocks across them
  (M markers contribute but don't print), and emits `block N: NAME`
  for every real entry plus the trailing `block N: ** Block of NULs **`.

### Two-pass walk with positional `-C`

- When listed-incremental runs have multiple srcs separated by
  `-C DIR` sentinels, the walk runs in two passes: pass 0 archives
  every directory across every src, pass 1 archives every file.
  Per-src collected entries are cached after pass 0
  (as `(PathBuf, bool /* was_dir */)`) so pass 1 reuses the same
  snapshot even after mid-walk deletes, and the pass filter remembers
  what counted as a dir when it was a dir.
- Gated on the presence of a positional `-C` sentinel — single-src
  runs and multi-src runs without `-C` retain the single-pass walk.

### Cyclic / chained rename extraction

- Keep R/T rename pairs separate from ordinary Y/N/D kids when
  building each directory's dumpdir, then concatenate with renames
  first.
- Extract-side R/T handling stages every rename source under a unique
  temp name first, then moves each temp into its target. Survives
  cyclic shuffles (a→b→c→a) and chained ones (d1→d3, d2→d1) where the
  original single-shot `fs::rename` would refuse to overwrite an
  existing target or clobber a file the next pair needed.

### `--incremental` without a snapshot file

- Accept `--incremental` / `-G` as a standalone incremental-mode flag
  (no snapshot file). Snapshot I/O is gated on `listed_incremental`
  being `Some`; dumpdir emission, new-dir messages, and across-pass
  gating all check `args.incremental`.

### File-removed mid-walk + blocking-factor

- `--blocking-factor=N` / `-b N` parsed (default 20). Checkpoint
  byte-counter multiplies by 512 per block, so
  `genfile --run --checkpoint N` triggers at the offsets the test suite
  expects when BF=1.
- Entries whose path vanished between walk collection and archive
  write emit `tar: PATH: File removed before we read it` under
  listed-incremental (gated on `--warning=no-file-removed`) and flag
  file-changed exit 1. When the vanishing cascades, the report walks
  up parents to surface the topmost gone ancestor — unless that
  ancestor was explicitly named on argv, in which case the standard
  `Cannot open` report from the per-source loop handles it.

### Exclude-tag + listed-incremental output ordering

- Move the `Directory is new` / `Directory has been renamed from`
  diagnostics ahead of the cache-tag `contains a cache directory
  tag …; contents not dumped` note so the stderr stream stays in
  directory-first order.
- Suppress the trailing `/` on the cache-tag path under
  listed-incremental (matches GNU) while keeping it for standalone
  `--exclude-tag` runs.

### Incremental rename detection + new-dir warnings

- `--warning=no-<name>` parses and applies to `new-dir`,
  `rename-directory`.
- Under `-v` on level-N+1 runs: emit `tar: PATH: Directory is new`
  when a current dir's `(dev, inode)` isn't in the prev snapshot, and
  `tar: PATH: Directory has been renamed from 'OLD'` when the inode
  match surfaces under a different name.
- Detect subdir renames by `(dev, inode)` against the previous-snapshot
  `dirs` map; when the inode surfaces under a new basename in this
  dir, emit GNU dumpdir codes `R <old>` + `T <new>` in the parent's
  dumpdir. On extract, process R/T pairs before the delete sweep.

### Per-dir dumpdir state in incremental snapshots

- Snapshot now carries the dumpdir each directory wrote last run. On
  create we look up the current dir's `(dev, inode)` in the snapshot,
  match each child's name against the previous dumpdir, and mark
  unchanged files `N` (skipped) vs new/changed `Y` (archived). Time
  comparisons use both sec + nsec so same-second creations don't
  false-positive.
- Snapshot path canonicalised pre-chdir so `-g db -C dir` lands
  relative to the invocation cwd.
- Listed-incremental walks emit all dirs first, then files ordered by
  parent dir, matching GNU's directory-first layout.
- Unlocks `incr03`, `incr05`, `rename04`, `rename05`.

### `--listed-incremental` bootstrap

- Snapshot-file I/O (GNU format 2 header + time + per-dir records),
  level-0 / level-N+1 file-level time filtering.
- GNU dumpdir (`D`) directory entries carrying child listings as the
  entry body.
- Extract-side deletion of disk children not mentioned in the dumpdir
  (gated on `-v` for the message).
- Deferred directory mtime restore so child writes don't clobber
  parent timestamps.
- Unlocks `listed01`, `incr01`, `incr02`, `incremental`.

### Earlier feature work

The implementation also covers everything in the README's "Supported
features" list as cumulative work prior to the trajectory above:
core ops (create / list / extract / diff / append / update / delete
/ `--test-label`), positional `-C` and `-T` handling, hard-link and
symlink semantics, scoped `--transform` / `--xform`, volume labels,
excludes / exclude-tag / exclude-caches / exclude-vcs, match
modifiers, owner / group / time / mode options, format-aware
name-length enforcement, `--use-compress-program` / built-in
gzip/bzip2/xz, `--keep-directory-symlink`, `--checkpoint` +
`--checkpoint-action`, `PaddedReader` for shrinking source files,
diff re-stat for mid-compare truncation, fast-path literal-pattern
exclude filter, octal escaping in `-t` / `-vc` output, GNU-style
strerror translation, archive-can't-contain-itself check,
`--ignore-failed-read`, `--keep-old-files`, `--skip-old-files`,
`--backup`, `--remove-files`, `--verify`, `--to-stdout`,
`--one-top-level`, `--no-overwrite-dir`, `--clamp-mtime`,
`--occurrence`, `--index-file`, `-l` / `--check-links`,
`--no-recursion` / `--recursion` directives in `-T` files,
positional-option warnings.

### Infrastructure

- `gnutar-test-harness`: shared Nix derivation that prebuilds the
  autom4te `tests/testsuite` script and helper programs (`genfile`,
  `checkseekhole`, `ckmtime`) once, reused by every per-test
  derivation.
- `testsuite.nix` runs each upstream test ID in a per-test Nix sandbox
  with `$TAR` pointed at rust-tar.
- `default.nix` lists all 224 upstream test names as flake checks.
