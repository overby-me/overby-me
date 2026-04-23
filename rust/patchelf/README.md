# rust-patchelf

A patchelf-compatible ELF binary patching tool written in Rust.

## Status

**46/46 tests passing (100%)** — the upstream NixOS/patchelf 0.15.2
test suite (`tests/*.sh`), wired into Nix checks. Each test runs the
official shell script in a sandbox with `rust-patchelf` symlinked at
the expected `../src/patchelf` path against pre-built ELF fixtures
from the upstream autotools build.

## Usage

Run a single upstream test:

```sh
nix build .#checks.x86_64-linux.rust-patchelf-test-{name}
```

View a failing test's log:

```sh
nix log .#checks.x86_64-linux.rust-patchelf-test-{name}
```

Batch-run every test:

```sh
nix build ".#checks.x86_64-linux.rust-patchelf-test-*" --keep-going --no-link
```

The binary is available as `patchelf` from `pkgs.rust-patchelf`
(release build, LTO + strip) or `pkgs.rust-patchelf-dev` (debug build,
faster compile).

## Architecture

Eight source modules:

- `args` — `Action`, `Options`, `parse_args`, `print_usage`, `VERSION`,
  `@FILE` argument expansion.
- `elf` — low-level helpers: `dynstr_info`, `vaddr_to_offset`,
  `read_str_at`, `write_str_inplace`, `available_space_at`.
- `grow` — width/endian-generic dynstr / PT_INTERP growth engine
  (`ElfBits`, `append_to_dynstr`, `add_dynamic_entry`).
- `commands/print` — `print_interpreter`, `print_rpath`, `print_soname`,
  `print_needed`.
- `commands/interpreter` — `set_interpreter` with LOAD-slack and
  EOF-append fallbacks.
- `commands/rpath` — `set_rpath`, `add_rpath`, `shrink_rpath`,
  `remove_rpath` with `--force-rpath` plumbing.
- `commands/soname` — `set_soname` with grow fallback.
- `commands/needed` — `add_needed`, `remove_needed`, `replace_needed`.
- `commands/debug` — `add_debug_tag` (DT_NULL slot repurposing).
- `main` — argv glue, multi-action loop with per-iteration re-parse,
  `parse_with_workarounds` for the no-gnu-hash fixture.

## Features

### Read operations

- `--print-interpreter` — read `PT_INTERP`.
- `--print-rpath` — `DT_RUNPATH` first, fall back to `DT_RPATH`.
- `--print-soname` — `DT_SONAME` (errors when absent, matching upstream).
- `--print-needed` — every `DT_NEEDED` entry.

### Write operations

- `--set-interpreter PATH` — in-place when the new path fits the
  existing PT_INTERP; otherwise park in the LOAD #1 slack; otherwise
  append at file EOF and repoint `PT_INTERP` (the kernel reads it
  directly from the file via `p_offset`).
- `--set-rpath PATH` — overwrite an existing `DT_RPATH`/`DT_RUNPATH`
  in place, grow `.dynstr` if too long, or add a brand-new tag via
  the first `DT_NULL` slot in `.dynamic`. Honours `--force-rpath`
  (uses `DT_RPATH` and flips an existing `DT_RUNPATH` tag).
- `--add-rpath PATH` — append entries to an existing rpath; falls
  through to `--set-rpath` when none exists.
- `--shrink-rpath` — keep only entries that contain at least one
  needed library; satisfies each lib by the *first* matching entry
  (matches upstream).
- `--allowed-rpath-prefixes PREFIXES` — combines with
  `--shrink-rpath`: drop entries that do not start with one of the
  colon-delimited prefixes.
- `--remove-rpath` — re-tag `DT_RPATH`/`DT_RUNPATH` as `DT_DEBUG`
  (cannot use `DT_NULL` because that terminates dynamic iteration).
- `--set-soname NAME` — overwrite `DT_SONAME` in place or grow
  `.dynstr` and add via a `DT_NULL` slot.
- `--add-needed LIB` — grow `.dynstr` + add `DT_NEEDED`.
- `--remove-needed LIB` — zero out the `DT_NEEDED` entry.
- `--replace-needed OLD NEW` — overwrite in place when the new name
  fits, otherwise grow `.dynstr` and patch `d_val`.
- `--add-debug-tag` — overwrite the first `DT_NULL` slot with
  `DT_DEBUG`.

### Argument handling

- `@FILE` expands to file contents in any string argument; missing
  file produces upstream's `getting info about FILE` error.
- `--output FILE` writes to a different path while preserving the
  input file's mode.
- `--page-size SIZE` accepted for compatibility (used by the no-rpath
  arch tests).
- `--no-default-lib`, `--clear-execstack`, `--set-execstack`,
  `--print-execstack`, `--debug`, `--rename-dynamic-symbols`,
  `--clear-symbol-version` accepted as no-ops or stubs for upstream
  test compatibility.

### Growth engine

`src/grow.rs` (~400 lines) handles every case where an in-place edit
would not fit. Two strategies, tried in order:

1. **LOAD #1 slack.** gcc/ld leaves 2-3 KB of zero padding between
   the end of LOAD #1 `filesz` and the next page boundary. Park the
   new payload there, grow LOAD #1 `filesz`/`memsz` to cover it. No
   phdr changes, no segments shift, no VAs change.
2. **GNU_STACK / PT_NULL → PT_LOAD repurpose.** For tiny binaries
   with no slack (the `no-rpath-prebuild/*` arch fixtures), append
   the payload at file EOF (page-aligned) and rewrite the
   otherwise-useless `PT_GNU_STACK` (or `PT_NULL` on MIPS) phdr into
   a fresh read-only `PT_LOAD` covering the new region.

Both strategies are width/endian generic via `ElfBits`, which
abstracts over 32 vs 64-bit `Elf_Dyn` / `Elf_Phdr` / `Elf_Shdr`
field layouts and LE/BE word reads.

Strings in `.dynstr` are referenced by *offset into strtab*, so only
`DT_STRTAB`, `DT_STRSZ`, and the `.dynstr` section header need
updating when the table moves. PT_INTERP can live anywhere in the
file because the kernel reads it directly via `p_offset`.

### Multi-action invocations

`main.rs` re-parses the (possibly mutated) buffer at the start of
each action so a single command line like
`patchelf --set-interpreter /lib --set-rpath /opt --add-needed lib.so`
sees the post-previous-grow layout for each subsequent action. Each
mutating command takes `&mut Vec<u8>` and may grow the file.

### Compatibility workarounds

- **`no-gnu-hash`.** When goblin 0.9 rejects a binary because
  `DT_GNU_HASH` points at zero buckets (which
  `strip --remove-section=.gnu.hash` legitimately produces),
  `parse_with_workarounds` walks the program-header table by hand to
  locate `PT_DYNAMIC`, then rewrites the `d_tag` of any
  `DT_GNU_HASH` entry to `DT_DEBUG` in a *parsing copy* of the
  buffer. The on-disk buffer is untouched, so the original
  `DT_GNU_HASH` survives the round trip; goblin only sees the
  patched copy.
