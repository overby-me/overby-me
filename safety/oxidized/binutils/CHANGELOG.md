# Changelog

Round-by-round development log of `safety/oxidized/binutils`. Reverse-chronological:
newest at top.

## Custom-test expansion (135/135 → 138/138 ✅)

- objdump `-M`/`--disassembler-options` parsing now follows GNU's
  comma-separated semantics correctly: `intel`/`intel-syntax` flips to
  full Intel; `att`/`att-syntax`/`att-mnemonic` resets to AT&T;
  `intel-mnemonic` is a mnemonic-only knob that doesn't change the
  operand syntax (so `-M intel,intel-mnemonic` stays Intel, but `-M
  intel,att-mnemonic` flips back to AT&T). Used to do a simple
  `contains("intel")` which mistakenly switched on `intel-mnemonic`.
  objdump blank-line spacing for combined flags also matches GNU now:
  `-ht` (section headers + symbol table) emits no blank between them;
  `-dt` (symbol table + disassembly) emits one trailing blank from
  the symbol table instead of two. +3 custom tests (135→138):
  `objdump-disassemble-with-syms`, `objdump-headers-syms`,
  `objdump-intel-mnemonic`.

## DejaGnu suite (328/329 → 329/329 ✅)

- Fix the long-standing `pr26808.dwp` failure (readelf -wi). It and
  `pr26160` are both `readelf -wi` on the same `.debug_addr`-less
  DWP file but with different stderr handling: pr26808 (in
  `x86-64.exp`) redirects stderr to `/dev/null`; pr26160 (in
  `readelf.exp`, run via `readelf_test`) merges stderr into stdout
  via dejagnu's local-exec. Their expected outputs differ exactly by
  the inlined "Cannot fetch indexed address" warning. GNU emits the
  warning to stderr mid-attribute (between the attribute name and the
  indexed-address value), and dejagnu's 2>&1 merge causes it to
  appear inline. Replicated this by emitting an `\x01ADDR_INDEX_WARN\x01`
  sentinel inside the formatted attribute string and special-casing
  the caller to flush stdout, write the warning to stderr, then
  continue stdout. Both tests now pass: pr26808 (no stderr) sees a
  clean indexed-address line; pr26160 (merged) sees the inline
  warning.

## Custom-test expansion (135/135 ✅)

- addr2line distinguishes "in-section, no DWARF" from "out-of-section":
  addresses outside any section's range emit `??:0` (matching GNU)
  instead of `??:?`. readelf `-nW` now emits the GNU_PROPERTY_TYPE_0
  properties on the same line as the description (joined with `,`)
  instead of separate lines. +2 custom tests (133→135):
  `addr2line-out-of-section`, `readelf-notes-wide`.

## Custom-test expansion (133/133 ✅)

- objdump `--start-address`: when the start offset doesn't coincide
  with an existing symbol, the synthetic label is now `<sym+0xN>`
  (offset from the nearest preceding symbol) rather than the
  fall-back `<.section_name>`. Matches GNU's `<text_symbol+0x4>`
  style output exactly. readelf no longer `return`s after printing
  "There are no program headers in this file." — combined flags like
  `-dl` now correctly continue to the dynamic section message
  ("There is no dynamic section in this file.") instead of stopping
  early. +2 custom tests (131→133): `objdump-start-address`,
  `readelf-dynamic-program-headers`.

## Custom-test expansion (131/131 ✅)

- objdump bundled-short-options now thread argument-taking flags
  correctly: `-dM intel`, `-dMintel`, `-dMatt`, `-dj.text`, `-dj NAME`
  all parse as `-d -M intel` / `-d -j NAME`. The bundled `D` short
  flag now also sets `disassemble_all` (so `-dD` and `-D` behave the
  same). objdump `-z`/`--disassemble-zeroes` honored: when set, the
  `\t...` zero-run collapse is suppressed and zero bytes disassemble
  to actual instructions. For `-D`/`--disassemble-all`, sections
  without a real symbol now get a synthetic `<.section_name>:` label
  to match GNU's per-section starting line. Inter-section blank lines
  in disassembly output are now exactly one (was two), matching GNU's
  spacing for the second-and-later sections. +3 custom tests
  (128→131): `objdump-disassemble-zeroes`,
  `objdump-intel-syntax-bundled`, `objdump-section-filter-bundled`.

## Custom-test expansion (128/128 ✅)

- objdump `-M intel`/`-M att` (and the `-M=` / `--disassembler-options=`
  forms) now switch the disassembly syntax. Intel mode is byte-for-byte
  with GNU's `objdump -M intel`: uppercase `BYTE PTR / WORD PTR /
  DWORD PTR / QWORD PTR` size hints (uses iced-x86's IntelFormatter
  with `MemorySizeOptions::Always` and uppercase keywords). objdump
  `--insn-width` is accepted (consumed; we don't yet honor it). nm now
  silently accepts `--no-demangle` (we don't demangle by default; `-C`
  enables it), `--quiet` (archive-listing terseness; no-op for single
  ELFs), and `--special` (synonym for `--special-syms`). +3 custom
  tests (125→128): `nm-no-demangle`, `nm-quiet`, `objdump-intel-syntax`.

## Custom-test expansion (125/125 ✅)

- objdump `--no-show-raw-insn`/`--show-raw-insn` toggle the bytes column
  in disassembly. The section filter (`-j NAME`) also gates which
  sections appear in the section-headers (`-h`) and symbol-table (`-t`)
  output, not just disassembly. +3 custom tests (122→125):
  `objdump-headers-section-filter`, `objdump-no-show-raw-insn`,
  `objdump-syms-section-filter`.

## Custom-test expansion (122/122 ✅)

- objdump now distinguishes `-d` (text only) from `-D`/`--disassemble-all`
  (every loadable section). With `-D`, only SHF_ALLOC sections are
  disassembled — `.rela.*` and other metadata sections are skipped to
  match GNU. The section filter (`-j NAME`) now controls which
  sections are disassembled, including non-text sections like `.data`.
  size sysv format (`-A`) honors the radix flags `-d`/`-o`/`-x`:
  decimal stays bare, octal gets a leading `0`, hex gets `0x` prefix.
  nm `--size-sort` now drops symbols whose `st_size` is zero (in
  addition to undefined/weak/absolute) — matches GNU's filter (we
  don't yet replicate GNU's pseudo-size computation for symbols
  without explicit `st_size`, so the displayed list is a subset of
  GNU's output, ordered the same way). +4 custom tests (118→122):
  `nm-size-sort`, `objdump-disassemble-section`, `size-sysv-hex`,
  `size-sysv-octal`.

## Custom-test expansion (118/118 ✅)

- nm bundled-short-options gained the previously-missing one-char
  flags `n` (numeric sort), `r` (reverse), `U` (defined-only), `j`
  (just-symbols), `a` (debug-syms no-op). Combined forms like `-ng`,
  `-nU`, `-nrg`, `-ngS`, `-jA` now parse correctly. nm `-n`/`-p`
  semantics are now last-wins to match GNU: bundled `-np` keeps
  `-p` (no sort), the reversed bundling keeps `-n` (numeric); same
  for the spaced variants.
  readelf `-D`/`--use-dynamic` flag: when no dynamic section exists,
  prints "Dynamic symbol information is not available for displaying
  symbols." matching GNU; otherwise filters `--syms` to dynsym only.
  objdump `-x`/`--all-headers` (equivalent to `-a -f -p -h -r -t`)
  is now wired, and `-r` reloc output emits the second trailing
  blank line GNU does. +3 custom tests (115→118): `nm-numeric-extern`,
  `nm-last-wins-sort`, `readelf-use-dynamic`.

## Custom-test expansion (115/115 ✅)

- objdump and nm now match GNU's archive-listing format. objdump
  prints a single `In archive ARCH:` banner before each member's
  output (members use only their member-name in the per-file banner,
  not `archive(member.o)`); nm prints just `member.o:` per member.
  nm also filters out `STT_FILE` and `STT_SECTION` symbols by default
  (matching GNU nm), so archives compiled from stdin (`<stdin>` file
  symbol) no longer add stray rows. size `--common` now folds common
  symbol sizes into the bss column (and adds a `*COM*` row in `-A`
  format), matching GNU's totals. c++filt `-_`/`--strip-underscore`
  and `-n`/`--no-strip-underscore` flags: when stripping, demangle
  the un-prefixed form and only emit it on success — failures fall
  back to the original (so `_main` stays `_main`, not `main`).
  c++filt also rewrites cpp_demangle's literal-cast templates
  (`(long)1`, `(unsigned int)7`, etc.) to GNU's typed-suffix form
  (`1l`, `7u`, ...) so deeply nested template arguments line up
  byte-for-byte. +5 custom tests (110→115): `cxxfilt-strip-underscore`,
  `nm-archive`, `nm-print-armap`, `objdump-archive`, `size-common`.

## Custom-test expansion (110/110 ✅)

- addr2line `-f`/`--functions` now falls back to the symbol table when
  DWARF doesn't yield a function (objects without debug info still
  resolve to a sensible name); preference order on tied addresses:
  text-section symbols beat data, then highest start addr wins, then
  first-seen. Adds `-p`/`--pretty-print` (combines function and
  file:line as `func at FILE:LINE`), `-a`/`--addresses` (prints the
  address before each entry), and `-j`/`--section=NAME` (treats
  addresses as offsets into NAME). The combined `-afp` form prints
  `addr: func at file:line` on one line. Bundled short opts (`-fp`,
  `-Cf`, `-afp`) now parse correctly. strings `-f`/`--print-file-name`
  prefixes each emitted string with `<file>:`, and bundled forms
  like `-fn 5` / `-fn5` parse correctly. nm `-f sysv` (space-separated
  short form) and `-fposix` (no separator) join the existing
  `--format=NAME` parser. +5 custom tests (105→110):
  `addr2line-addresses`, `addr2line-addresses-functions-pretty`,
  `addr2line-functions`, `addr2line-pretty-functions`,
  `strings-print-file-name`.

## Custom-test expansion (105/105 ✅)

- readelf `-u`/`--unwind` (no-arch-specific decoder, prints "No
  processor specific unwind information to decode") and
  `-c`/`--archive-index` (when given a non-archive, prints "readelf:
  Error: File <path> is not an archive so its index cannot be
  displayed."). readelf `-p NAME` and `-x NAME` now emit GNU's
  " NOTE: This section has relocations against it..." line when a
  matching `.rela.NAME`/`.rel.NAME` exists. `-p` no longer prints a
  trailing blank line after "No strings found in this section." (only
  after actual strings). `-p` string detection: skip leading
  non-printable bytes (start-of-string requires a printable char) so
  sections with only nul/control bytes report "No strings found";
  control chars *inside* a string are still escaped (`\r` → `^M`).
  `-wL`/`--debug-dump=decodedline` is silent when there's no
  `.debug_line`. +5 custom tests (100→105):
  `readelf-archive-index-not-archive`, `readelf-hex-dump-text`,
  `readelf-string-dump-data`, `readelf-string-dump-text`,
  `readelf-unwind`.

## Custom-test expansion (100/100 ✅)

- nm `-S`/`--print-size` now shows the size column for symbols whose
  size is non-zero, matching GNU's BSD-style output exactly. nm also
  silently accepts `--target=NAME` and `--plugin=NAME` (we don't have
  BFD targets or linker plugins, so these are no-ops). objdump `-t`
  symbol-table flags column shrunk from 8 to 7 chars (matching GNU's
  `[scope][weak][ctor][warn][indir][dbg][kind]` layout); common
  symbols now correctly show `` for scope (not `g`), `O` for kind,
  and `*COM*` for section. ar `-O` (offset display) now works without
  `-v` and reports the data offset (post-header) like GNU. objdump
  bundled short flags now recognize `w` (e.g. `-hw`). +4 custom tests
  (96→100): `ar-offsets`, `nm-print-size`, `nm-target-noop`,
  `objdump-section-headers-wide`.

## Custom-test expansion (96/96 ✅)

- c++filt `-p`/`--no-params` strips the trailing `(...)` parameter
  list (template-aware: doesn't touch parens inside `<...>`); nm
  POSIX format honors `-t d/o/x` (was always hex). Replaced an old
  `cxxfilt-no-params` test that incorrectly invoked `-i` with the
  correct `-p` test. +7 custom tests (89→96): `cxxfilt-no-params`
  (corrected), `nm-no-weak`, `nm-posix-decimal`, `nm-posix-octal`,
  `readelf-groups-wide`, `readelf-notes-symbols`,
  `readelf-sections-arch-wide`.

- readelf `-V` no longer means `--version` (it's `--version-info`):
  with no SHT_GNU_VERDEF/SHT_GNU_VERNEED present, emits "No version
  information found in this file." matching GNU. +5 custom tests
  (84→89): `readelf-version-info`, `nm-line-numbers`,
  `nm-all-with-prefix`, `readelf-program-wide`, `readelf-dynamic-arch`.

- nm `--special-syms`/`--no-special-syms` accepted as no-ops (we don't
  generate synthetic symbols). +7 custom tests (77→84):
  `nm-format-bsd` (`-B`), `nm-special-syms`, `nm-extern-only-posix`
  (`-gP`), `nm-undefined-only-posix` (`-uP`),
  `readelf-notes-with-sections` (`--notes -S`), `readelf-header-notes`
  (`-nh`), `addr2line-basenames`.

- nm `--no-recurse-limit`/`--recurse-limit` accepted as no-ops
  (demangler-only knob), and c++filt `-s STYLE` / `--format STYLE`
  treated as a value-taking flag (the style argument was previously
  picked up as input). +4 custom tests (73→77): `nm-no-recurse-limit`,
  `nm-reverse-numeric`, `cxxfilt-style`, `cxxfilt-no-recurse-limit`.

- 6 more custom tests for multi-file invocations and addr2line `-i`
  (+6, 67→73): `nm-multiple-files`, `size-multiple-files`,
  `readelf-multiple-files`, `objdump-multiple-files`,
  `strings-multiple-files`, `addr2line-inlines`.

- 3 more custom tests for already-supported flag combinations (+3,
  64→67): `nm-radix-hex-print-file-name` (`-t x -A`),
  `strings-radix-decimal` (`-t d`), `strings-radix-octal` (`-t o`).

- nm `-A` POSIX prefix uses "file: " (space after colon); `-j` drops
  the file-name prefix entirely (matches GNU). +2 custom tests
  (62→64): `nm-print-file-name-posix`, `nm-just-symbols-with-prefix`.

- 5 more custom tests for already-supported flag combinations (+5,
  57→62): `readelf-header-sections` (`-hS`), `readelf-header-program`
  (`-hl`), `readelf-hex-dump-data` (`-x .data`), `cxxfilt-no-params`
  (`-i`), `addr2line-multiple` (multiple addresses on the command
  line).

- nm `-a`/`--debug-syms` flag accepted (no-op for files without debug
  syms); readelf `-p NAME` / `--string-dump=NAME` emits the trailing
  blank line GNU does after the strings list. +4 custom tests
  (53→57): `nm-debug-syms`, `readelf-string-dump`, `readelf-hex-dump`,
  `readelf-headers-combined`.

- readelf `-p NAME` missing-section warning format + 4 more custom
  tests (+4 custom tests, 49→53): the "Section 'X' was not dumped
  because it does not exist" warning now matches GNU exactly (no
  leading blank line, no trailing `!`). New tests:
  `readelf-string-dump-missing`, `readelf-histogram`, `cxxfilt-types`,
  `addr2line-demangle`.

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
