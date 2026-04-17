# rust-file

A GNU file-compatible file type detection tool written in Rust.

## Status

**320/320 tests passing (100%)** — regression suite from the upstream
[file/file-tests](https://github.com/file/file-tests) corpus (pinned at
`0bcc555a`). Each test runs both rust-file and the reference file(1)
against the same sample in a sandbox and diffs the output byte-for-byte.

## Usage

Run a single upstream test:

```/dev/null/example.sh#L1
nix build .#checks.x86_64-linux.rust-file-test-{type}__{file}
```

View a failing test’s log:

```/dev/null/example.sh#L1
nix log .#checks.x86_64-linux.rust-file-test-{type}__{file}
```

Batch-run every test in a single evaluator (much faster than looping):

```/dev/null/example.sh#L1
nix build .#checks.x86_64-linux.rust-file-test-* --keep-going --no-link
```

The binary is available as `file` from `pkgs.rust-file` (release build)
or `pkgs.rust-file-dev` (debug build, faster compile).

## Architecture

Single-file implementation in `src/main.rs` (~5400 lines, zero
dependencies). The detection pipeline reads up to 2 MiB per file and
walks a chain of magic-byte checks, falling through to text heuristics
when no binary signature matches.

Key functions:

- `main` — CLI argument parsing (`-b`, `-i`, `--mime-type`, `-L`, `-0`).
- `identify_file` — entry point per file: stdin, symlinks, directories,
  special files, empty files.
- `identify_data` — the primary dispatcher: ~170 binary format checks
  by magic bytes, then text heuristics.
- Format-specific parsers: `identify_elf`, `identify_jpeg`,
  `identify_png`, `identify_gif`, `identify_bmp`, `identify_pdf`,
  `identify_cabinet`, `identify_rpm`, `identify_rar`, `identify_7z`,
  `identify_lnk`, `identify_macho_fat`, `identify_ext_fs`,
  `identify_mbr`, `identify_ntfs_boot`, `identify_dump_be`,
  `identify_installshield`.
- OLE/CDF: `ole_structural_summary` (sector chain following),
  `format_ole_summary` (property set parsing).
- ELF: `identify_elf`, `find_elf_interp`, `find_nt_prpsinfo`,
  `find_gnu_build_id`, `find_gnu_abi_tag`, `find_netbsd_ident`.
- Text analysis: `is_text_data`, `looks_like_mail`, `looks_like_json`,
  `identify_utf16`, `identify_utf32`, `encoding_suffix_for_text`.
- Helpers: `format_unix_utc`, `format_filetime`, `tiff_summary`,
  `ogg_vorbis_vendor`, `file_printable`.

## Supported formats

### Binary formats

ELF (full parser: class, endian, OS ABI, machine, type, PIE, dynamic
linking, interpreter, GNU ABI tag, NetBSD ident, BuildID, debug_info,
stripped, core dump process info) · Mach-O (single and universal/fat
with per-arch detail) · PE32/PE32+ · Java class · ar/deb · RPM · RAR
v4/v5 · 7-zip · gzip (with XFL flags) · bzip2 · xz · zstd · tar · ZIP
(OOXML, OpenDocument) · Microsoft Cabinet (MSCF with CFFOLDER/CFFILE,
OneNote Package) · OLE/CDF Compound Document (MSI, MST, MSP, DOC, XLS,
PPT with full property set parsing via sector chain) · PDF · PNG · JPEG
(JFIF, Exif/TIFF IFD, SOF markers, comments) · GIF · BMP · TIFF ·
ICO/CUR · JPEG 2000 · Ogg/Vorbis · Matroska/WebM · RIFF/WAV/AVI ·
MIDI · MP3/MPEG · MNG · Netpbm · TGA · SQLite · QCOW/QED/VDI ·
Berkeley DB · Python .pyc · OneNote · gettext .mo · Samba TDB · MDMP ·
CHM · Z-machine · PGP · SELinux policy · ICC color profiles · PFB fonts
· TrueType/OpenType · MS Access · LZMA · glibc locale · dBase/DBF ·
MySQL FRM/MYI · AppleDouble · DS_Store · TZif · ISO 9660 · LVM2 ·
Linux swap · TNEF · PIF · Kodak PCD · MySQL binlog · DOS COM · Linux
bzImage · Linux S390 kernel · MS Windows shortcut (LNK) · ext2/ext3/ext4
filesystem · DOS/MBR boot sector · NTFS boot sector · Unix dump file ·
InstallShield Script · 3DS

### Text formats

Shebang scripts · XML/SVG · HTML · RTF · PostScript · PEM certificates
· PGP armored · unified/context diffs · gettext .po · troff · C/C++ ·
Rust · Python · JavaScript · Makefiles · M4 · mail (RFC 2822) · JSON ·
UTF-8/UTF-16/UTF-32 with BOM detection · ISO-8859 text · ASCII text
