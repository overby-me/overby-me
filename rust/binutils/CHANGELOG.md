# Changelog

Round-by-round development log of `rust/binutils`. Reverse-chronological:
newest at top.

## Custom-test expansion (49/49 ✅)

- objdump `-f` trailing blank line + `objdump-file-headers` custom test
  (+1 custom test, 48→49): emit a trailing blank line after `start
  address 0x...` to match GNU's exact-byte output.
- 3 more custom tests for already-supported flags (+3 custom tests,
  45→48): `strings-radix-hex` (`-t x` offset format),
  `cxxfilt-no-strip-leading` (`-n` keep underscore), `readelf-wide`
  (`-S -W` wide section headers).
- readelf `-d` empty-file message + 4 custom tests (+4 custom tests,
  41→45): when there is no SHT_DYNAMIC section, print "There is no
  dynamic section in this file." instead of an empty "Dynamic
  section:" header. New tests: `readelf-dynamic`,
  `readelf-arch-specific`, `readelf-groups`, `objdump-section-filter`.
- nm `-j`/`--just-symbols` flag and 3 more custom tests (+3 custom
  tests, 38→41): print only symbol names (no address, type, size). New
  tests: `nm-just-symbols`, `readelf-notes`, `readelf-relocs`.
- readelf `-e`/`--headers` (file + section + program headers) + format
  polish (+1 custom test, 37→38): wires the multi-section `-e` flag;
  suppresses the redundant "There are N section headers" line when `-h`
  is also requested (info already in file header); makes the "R
  (retain)" Key-to-Flags entry conditional on the file having any
  SHF_GNU_RETAIN section; emits "There are no program headers in this
  file." for relocatable files instead of an empty Program Headers
  table. New test: `readelf-headers-all`.
- nm POSIX format size-when-zero handling + 5 more custom tests (+5
  custom tests, 32→37): `-P`/`--portability` now prints a blank size
  column for symbols with `st_size == 0` (except common where st_value
  is the alignment) — matches GNU's POSIX format. New tests:
  `nm-radix-octal`, `nm-radix-hex`, `nm-posix`, `ar-print`,
  `addr2line-pretty`.
- size `-d`/`-o`/`-x` radix flags + new `nm`/`size` custom tests (+5
  custom tests, 27→32): individual columns formatted in chosen radix
  (decimal plain / octal with leading 0 / hex with `0x`); total columns
  are always "dec hex" (or "oct hex" with `-o`); total octal value is
  *not* zero-padded (matches GNU). Plus `nm -t d`, `nm -A`, `size -d`,
  `size -o`, `size -x` custom tests.
- nm `-U`/`--defined-only`, `-n`/`--numeric-sort`,
  `-r`/`--reverse-sort` (+3 custom tests, 24→27): filter out undefined
  symbols, sort by address (undefined symbols first by name), and
  reverse the final order. Numeric sort puts U/w symbols before defined
  ones, then sorts defined by address.

## DejaGnu suite (320/321 → 328/329)

- Add readelf `.debug_pubnames`/`.debug_aranges`/`.debug_frame` dumpers
  (`-wp`/`-wr`/`-wf`) and reorder the bare `-w` dispatch to match GNU
  readelf's section ordering (.debug_abbrev → .debug_info → .debug_line
  raw → .debug_pubnames → .debug_aranges → .debug_str → .debug_frame).
  Apply `.rela.debug_frame` / `.rel.debug_frame` relocations so FDE
  `pc=` values resolve, with implicit-addend reading for SHT_REL. CIE
  state (code/data alignment) saved across CIE → FDE so
  DW_CFA_advance_loc/_loc1/_loc2/_loc4/_def_cfa/_def_cfa_offset/_offset/_restore/_set_loc
  all decode correctly. i386 register-name table separate from x86-64.
  Line-program `(view N)` annotations in special opcodes when PC
  doesn't advance. Fixes both `x86-64/compressed-1a` and
  `i386/compressed-1a` (+2 dejagnu, 326→328).
- Wire `binutils-all/i386/i386.exp` into the runtest invocation (+7
  dejagnu, 319→326): runs the i386-32-bit subdir tests (empty, ibt,
  pr21231a/b, shstk, plus the strip-on-debug-sections variants) — they
  exercise our i386 ELF32 read/write path. Also fix: emit a double
  blank line before objdump's "Disassembly of section ...:" header to
  match GNU's exact-byte output.
- objcopy `-O elf32-x86-64` (x32 ABI) and `-O elf64-x86-64` (x32→ELF64)
  conversions with `.note.gnu.property` merging (+8 dejagnu, 311→319):
  translates ELF64↔ELF32 section headers, symtab entries (16↔24
  bytes), RELA entries (12↔24 bytes), REL entries (8↔16 bytes), and
  SHF_COMPRESSED Chdr (12↔24 bytes); merges per-CU GNU property notes
  by ORing flag values for the same `pr_type` and re-aligning to the
  target ABI's property alignment (8 for ELF64, 4 for ELF32). Fixes
  `binutils-all/x86-64/pr23494a/c/d/e` and their `-x32` variants.
- Add `--sframe[=NAME]` SFrame v2/v3 header dump for both readelf and
  objdump (+2 dejagnu, 309→311): parses the SFrame preamble (magic
  0xdee2 + version + flags), abi_arch, cfa_fixed_ra_offset, num_fdes,
  num_fres; emits `Contents of the SFrame section <name>:` + `Header:`
  block.
- DWARF -wi format polish + objcopy x86-64-only `large` flag check (+1
  dejagnu, 308→309): readelf -wi prints offset 0 as "0" (not "0x0");
  trailing blank line at end of `.debug_info` dump; objcopy rejects
  `--set-section-flags X=...,large,...` when output target is non-x86-64
  ELF — fixes `large-sections-i386`.
- readelf `-l` "Section to Segment mapping" + strip recomputes PT_TLS
  p_memsz (+1 dejagnu, 307→308): per-segment list of contained sections
  using `SHF_ALLOC`/VMA vs file-offset comparison, with
  TLS-NOBITS-in-PT_LOAD/PT_GNU_RELRO exclusion; `strip_inplace_elf`
  rewrites PT_TLS `p_memsz` to `max(sh_addr+sh_size) - min(sh_addr)`.
  Fixes `strip (binutils-all/x86-64/pr27708.exe)`.
- Compact `.shstrtab` after `strip --strip-all` removes empty
  `.symtab`/`.strtab` (+2 dejagnu, 305→307): `elf_remove_empty_symtab`
  rebuilds `.shstrtab` to drop the names of removed sections, updates
  each surviving header's `sh_name` to the new offset, and shrinks
  `sh_size` accordingly. Fixes `strip on uncompressed/compressed debug
  sections`.
- x86-64.exp polish (+7 dejagnu, 298→305):
  - Property formatting: trailing space after "no copy on protected";
    per-bit `<unknown: HEX>` entries instead of merged hex (fixes
    pr21231a/b).
  - SHF_X86_64_LARGE (0x10000000): `large` keyword in objcopy
    `--set-section-flags`; readelf `-S` shows `l` without falling
    through to the generic `p` flag.
  - readelf `-l` prefix: `Elf file type is …`, `Entry point 0x…`, `There
    are N program headers, starting at offset M`. Auto-detect PIE via
    DT_FLAGS_1 & DF_1_PIE so ET_DYN PIE shows "(Position-Independent
    Executable file)". INTERP segment annotated with `[Requesting
    program interpreter: <path>]`.
  - Strip/objcopy/nm/readelf `--help` emits a `supported targets:`
    line listing common ELF formats including elf64-littleaarch64
    (fixes pr33230).
- Decode `.note.gnu.property` (NT_GNU_PROPERTY_TYPE_0) entries in
  `readelf -n` output (+12 dejagnu, 286→298): decodes
  GNU_PROPERTY_X86_FEATURE_1_AND (IBT/SHSTK/LAM_U48/LAM_U57),
  GNU_PROPERTY_X86_FEATURE_USED, GNU_PROPERTY_X86_ISA_USED/NEEDED,
  GNU_PROPERTY_STACK_SIZE, GNU_PROPERTY_NO_COPY_ON_PROTECTED. Wired in
  `binutils-all/x86-64/x86-64.exp`.
- Add `elfedit` tool with `--output-mach`/`--output-type`/
  `--output-osabi`/`--output-abiversion` (+6 elfedit.exp, 280→286):
  minimal in-place ELF header field patcher; readelf prints "Intel
  L1OM" / "Intel K1OM" / "Intel MCU" / "FenixOS".
- Add objcopy `--dump-section`, `--update-section`, and
  `--update-section + --remove-section` conflict detection (+6
  update-section.exp, 274→280): in-place ELF rewriter
  `objcopy_inplace_update_sections` reflows section file offsets
  respecting per-section sh_addralign so that `update-N.o` becomes
  byte-equal to `update-1.o` after section content replacement.
- Implement GNU location view pair extension for DWARF 5
  `.debug_loclists` (37→38 readelf.exp = 100%): `DW_LLE_GNU_view_pair`
  (kind=9) inline annotations alongside per-list view list iteration.
  Fixes `readelf locview-2`.
- Add readelf `-ws`/`-wm`/`--debug-dump=str`/`macro` support (30→31
  readelf.exp): `-ws` hex-dumps every `.debug_str*` section and decodes
  `.debug_str_offsets.dwo` entries; `-wm` walks `.debug_macro*` (DWARF 5
  header + flags + DW_MACRO_* opcodes).
- Decode dwarf-attribute enum values (29→30 readelf.exp):
  `format_data_attr` decodes DW_AT_ordering, DW_AT_visibility,
  DW_AT_inline, DW_AT_accessibility, DW_AT_calling_convention,
  DW_AT_identifier_case, DW_AT_virtuality, DW_AT_decimal_sign,
  DW_AT_endianity, DW_AT_defaulted, plus more values for DW_AT_language
  and DW_AT_encoding. New `format_block_with_attr` decodes
  `DW_AT_discr_list`. Fixes `readelf -wi dwarf-attributes`.
- Add objdump `--dwarf=Ranges` support (27→28 objdump.exp): wires
  `objdump -WR/--dwarf=Ranges` to `readelf_debug_ranges`; fixed
  `.debug_ranges` printer to handle "base address selection entry".
- Add objdump `-l`/`--line-numbers` annotation + build-id-debuglink
  lookup (25→27 objdump.exp): when main file lacks `.debug_info`, parse
  `.note.gnu.build-id`, derive `.build-id/<XX>/<rest>.debug` path, and
  load the alt file's DWARF context.
- Add objdump `-S`/`--source`/`--source-comment` source interleaving
  (23→25 objdump.exp): walks `.debug_line` via gimli, caches source
  files, emits source lines before the disassembly when (file, line)
  changes.
- Add objcopy `-I binary -O elf*` binary→ELF conversion (111→113
  objcopy.exp): synthesizes an ELF object with `.data` section
  containing raw bytes plus three symbols `_binary_<path>_start`,
  `_binary_<path>_end`, `_binary_<path>_size`.
- Fix objcopy unknown section flag warning (109→110→111 objcopy.exp):
  scans input ELF for unrecognized `sh_flags` bits and emits a warning
  to stderr; readelf `-t` `elf_section_flags_detail` now prints
  `UNKNOWN (xxxxxxxx)` for any non-OS, non-PROC unknown bits.
- Add DW_OP expression decoder (28→29 readelf.exp; 22→23 objdump.exp):
  emits the `(DW_OP_addr: <hex>)` / `(DW_OP_addrx <idx>)` annotation
  appended to `N byte block: ...` for `DW_FORM_exprloc`/`block*`.
  Handles all standard DWARF 5 ops plus GNU extensions.
- Add basic `readelf -wi` / `--debug-dump=info` / `--dwarf=info`
  DWARF `.debug_info` dumper (26→28 readelf.exp passes; unlocks 3
  objdump -Wi tests): custom DWARF reader handles CU headers, DIE
  traversal with depth, abbrev tables, and the core DW_FORM_* value
  formatting. Custom tolerant sLEB128 decoder detects over-long
  encodings and emits GNU-readelf-compatible warnings (required by
  pr26548).
- Fix ar.exp `replacing non-deterministic member` (12→13/14): unset
  SOURCE_DATE_EPOCH at dejagnu test entry (Nix build env sets it; the
  test explicitly requires it absent).
- Fix objcopy SHT_GROUP preservation (106→109 objcopy.exp): in-place
  ELF rewriter for `--remove-section` on files with COMDAT groups;
  preserves GROUP section type, drops orphan `.rela.X`/`.rel.X` when
  target removed, rebuilds `SHT_GROUP` contents with renumbered
  surviving indices, renumbers `sh_link`/`sh_info` everywhere.
- Fix strip on executables (104→106 objcopy.exp): in-place ELF
  section-table edit for ET_EXEC/ET_DYN preserves `sh_addr`,
  program-header layout, and PROGBITS/NOBITS distinction; `-K`
  rewrites `.symtab`/`.strtab` instead of dropping them.
- Fix readelf.exp failure (25→26/39): `-j`/`--display-section` on
  REL/RELA sections now emits a relocation table instead of a hex dump
  (GNU-compatible format).
- Fix objdump.exp failures (19→22/33): in-place ELF
  `sh_addr`/`e_entry` patching for `--change-section-address`,
  `--adjust-vma`, `--set-start`, `--adjust-start`.
- Fix objcopy.exp failures (12→44, then 44→71, then 71→76 /116):
  byte-copy fast path, `-O verilog`/`srec`/`ihex` output, glob-pattern
  section selectors, `--add-section`, `--add-symbol`,
  `--strip-section-headers`, SREC start/VMA handling, additional
  section/symbol handling edge cases.
- Fix more objdump.exp failures (17→19/33): `-Wk`/`--dwarf=links` for
  `.gnu_debuglink`/`.gnu_debugaltlink` parsing; `-s -j .zdebug_*`
  compressed-section notice; `--start-address`/`--stop-address` for
  `-s` and `-d`.
- Fix more readelf.exp failures (17→21/39): `-p` escape sequences (`\n`,
  `^X`), `--enable-checks` for zero-sized sections, `-j`/`--display-section`,
  `-r` for `SHT_RELR` bitmap-encoded relocations.
- Fix more objcopy: `--only-keep-debug` (PROGBITS→NOBITS), strip empty
  file (.symtab/.strtab removal), SHT_GROUP preservation via
  fast-path, `-wR`/`--debug-dump=ranges`, group signature symbol lookup.
- Fix readelf `--debug-dump=loc` (`-wo`) and `--decompress --hex-dump`
  for SHF_COMPRESSED + legacy `ZLIB`-prefixed sections (flate2 dep).
- Fix nm `--line-numbers` for extern globals: implement
  `nm_build_line_info` with gimli, handles `DW_AT_specification` chains
  for declaration→definition resolution.
- Fix objcopy strip-13/14/15: `validate_relocations()` emits unsupported
  reloc type / invalid symbol index errors; strip no-op fast-path
  preserves byte-exact section layout.
- Fix size.exp `-G` GNU format: separate text/data/bss classification
  rules and 4-column GNU layout.
- Add readelf `--debug-dump=links` / `-wK` / `-wN` for
  `.gnu_debuglink`, `.gnu_debugaltlink`, and `.debug_info` dwo-file
  links.
- Fix objdump `-Z -s` (decompress hex dump): remove stray debug
  eprintln; decompress legacy GNU `ZLIB`-magic format with concatenated
  zlib streams via flate2.
- Raise `minPass` thresholds for ar (1→12), nm (3→13), addr2line
  (0→3), strings (0→1), readelf (6→25), objdump (3→20), objcopy
  (12→104), size (2→3).
- Fix objdump.exp failures (3→17/29): `-i` info, `-f` file headers,
  archive support, `-s` hex dump, `--disassemble=SYM`,
  `--show-all-symbols`, `-j` section filter.
- Fix readelf.exp failures (6→17/39): `-h`, `-s`, `-r`; `-p` string
  dump, `-n` notes, `-t` section details, `-C/--demangle`, archive
  (regular & thin) inspection, SHF_GNU_RETAIN/SHF_MASKOS flag handling.
- Fix strings.exp failure (0→1/1): `--encoding=l/b` UTF-16LE/BE
  support.
- Fix addr2line.exp failures (0→3/3): real DWARF line program
  implementation, function name lookup, `-s`/`-e` flags.
- Fix nm.exp failures (3→12/13): POSIX format, radix, `--size-sort`,
  `--line-numbers` (DWARF), `--ifunc-chars`, `--no-weak`,
  `--print-armap`.
- Fix ar.exp failures (1→12/14): POSIX args, D/U flags, `tv`/`tvO`,
  `-m`, `-d` basename, `--output`, `--record-libdeps`.

## Initial test infrastructure

- Wire upstream DejaGnu test suite into Nix checks via
  `dejagnu-testsuite.nix`. Configure `site.exp` with tool paths,
  `AS_FOR_TARGET`, `CC_FOR_TARGET`. Add 9 per-file DejaGnu checks with
  calibrated thresholds plus an informational `dejagnu-all` check.
- Add `rust-binutils-dev` package (debug build for fast iteration).
- Create `testsuite.nix` with the custom comparison test runner. Create
  24 custom test scripts across 8 tools.
- Fix nm, c++filt, size, addr2line, readelf, objdump to pass custom
  tests:
  - **nm symbol type classification** — look up actual ELF section by
    index; classify by section name and flags.
  - **nm POSIX format** — `-P`/`--portability` output: `name type value
    size`; `--format=posix/sysv/bsd` selection.
  - **nm radix selection** — `-t d/o/x` / `--radix=` for
    decimal/octal/hex addresses.
  - **nm `--size-sort`** — sort symbols by size and display size.
  - **nm `--line-numbers`** — full `gimli`-based DWARF-4 parser with
    relocation application for `.o` files and `DW_AT_specification`
    resolution.
  - **nm `--ifunc-chars`** — custom type chars for GNU indirect
    functions (`STT_GNU_IFUNC`).
  - **nm `--no-weak`** — filter out weak symbols; `STB_GNU_UNIQUE`
    support (`u` type char).
  - **c++filt demangling** — replaced hand-rolled parser with
    `cpp_demangle` crate for full Itanium ABI support; post-process
    `(long)N` cast syntax → `Nl` suffix notation to match GNU
    libiberty.
  - **size Berkeley format** — classify sections by ELF flags
    (`SHF_ALLOC`, `SHF_WRITE`, `SHT_NOBITS`) instead of section name;
    bss defaults to 0.
  - **size SysV format** — filter to `SHF_ALLOC` sections only; match
    GNU column widths.
  - **addr2line output** — output `??:?` (not `??:0`) for unknown
    addresses; only print function name line with `-f`.
  - **readelf `-h` fix** — don't treat `-h` as `--help`; inline
    version/help check so `-h` means `--file-header`.
  - **readelf `-S` format** — two-line per section format matching GNU
    layout.
  - **readelf `-l` fix** — print "no program headers" message for
    object files instead of empty table.
  - **readelf `-s` fix** — raw ELF symtab parsing with section name,
    entry count, null symbol at index 0.
  - **objdump `-h` fix** — same `-h` → `--help` bug as readelf; filter
    to `SHF_ALLOC` sections; add flag description lines (`CONTENTS,
    ALLOC, LOAD, ...`).
  - **objdump `-d` disassembly** — added `iced-x86` crate for proper
    x86/x64 instruction decoding with AT&T syntax (`GasFormatter`);
    symbol labels at function boundaries; zero-byte run collapsing with
    `...` ellipsis.
  - **objdump `-t` fix** — tab separator between section and size;
    blank bind for common/undefined symbols; `*COM*` section name for
    common symbols.
  - **objdump `-r` fix** — map raw ELF relocation types to proper names
    (`R_X86_64_32`, `R_X86_64_PC32`, etc.); resolve symbol indices to
    names; show addends.
  - **objdump archive support** — `-f`, `-h`, `-t`, `-r`, `-d`, `-s`
    process `.a` archive members individually.
  - **addr2line DWARF implementation** — replaced stub with real
    `gimli`-based DWARF line program walker; function name lookup via
    `DW_TAG_subprogram`; `-s`/`--basenames` and `-e` flags.
  - **readelf `-p`/`-n`/`-t`/`-C` modes** — `-p`/`--string-dump` for
    printable nul-terminated strings; `-n`/`--notes` for `SHT_NOTE`
    parsing with NT_GNU_* type names; `-t`/`--section-details`
    three-line per-section output with full names and decoded flag
    names; `-C`/`--demangle` via `cpp_demangle`.
  - **readelf archive support** — regular and thin (`!<thin>\n`)
    archive inspection with `File: archive(member)` headers.
  - **objcopy byte-copy fast path** — when no transformations
    requested, use `fs::copy()` directly.
  - **objcopy `-O verilog/srec/ihex`** — Verilog hex output with
    `@ADDR` headers and `--verilog-data-width 1/2/4/8/16`
    byte-swapping; basic SREC and IHEX stubs.
  - **objcopy section ops** — `--set-section-alignment`,
    `--set-section-flags`, `--rename-section` parsing/application.
  - **objcopy symbol ops** — `--strip-symbol` with reloc check (emits
    `not stripping symbol` warning); `--keep-global-symbol` vs
    `--globalize-symbol` incompatibility error.
  - **strip archive support** — detects `!<arch>` magic and strips each
    ELF member individually.
  - **STT_NOTYPE symbol handling** — infer symbol kind (Text if section
    is code, else Data) instead of failing the writer.
  - **strings `-e` encoding** — added `--encoding=l/b` for UTF-16LE/BE
    multibyte string scanning.
  - **ar POSIX argument parsing** — support `ar -r -c archive file`
    style (multiple dash-prefixed args).
  - **ar deterministic mode** — `D`/`U` flags for deterministic
    (uid=0, gid=0, mtime=0, mode=0o100644) vs real metadata;
    `SOURCE_DATE_EPOCH` support.
  - **ar `tv` verbose format** — `rw-r--r-- 0/0 size date name`
    matching GNU; `O` flag for hex offsets.
  - **ar `-m` move** — move members to end of archive.
  - **ar `-d` basename matching** — delete/extract match by basename.
  - **ar `--output=dir -x`** — extract to specified output directory.
  - **ar `--record-libdeps`** — add `__.LIBDEP` member with library
    dependency info.
  - **nm `--print-armap`** — display archive symbol table index.
  - **testsuite normalization** — strip trailing whitespace in compare
    function to avoid false failures.
- Add x86 disassembly with `iced-x86` crate.
