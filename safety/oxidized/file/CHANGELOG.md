# Changelog

All notable changes to rust-file.

## [Unreleased]

### Test suite compatibility

Passes 320/320 of the upstream file/file-tests regression corpus
(pinned at commit 0bcc555a). Compares rust-file output against GNU
file 5.45 byte-for-byte in a Nix sandbox.

### Test harness

- Nix check per sample: `testsuite.nix` runs both `file -b` and
  `rust-file -b` against each of the 320 samples, normalizes Nix
  store paths, and diffs. Test names encode `type__file` from the
  `db/<type>/<file>` corpus layout.
- `default.nix` enumerates test pairs at eval time from a pinned
  `fetchTarball` of file-tests, generating ~320 checks without a
  hand-maintained list.
- `rust-file-dev` debug build for faster test iteration.

### JPEG

- Full JFIF segment parsing: version, resolution units, density,
  segment length.
- Exif segment parsing: detects `Exif\0\0` marker, extracts embedded
  TIFF IFD entries (description, manufacturer, model, orientation,
  xresolution, yresolution, resolutionunit, software, datetime) via
  `tiff_summary`.
- SOF marker parsing: baseline/extended/progressive type, precision,
  dimensions, number of components.
- COM marker: extracts comment text.

### Gzip

- XFL byte (offset 8): emits “max compression” (XFL=2) or “max speed”
  (XFL=4).
- Replicates upstream file 5.45 multi-stream trailer behavior for
  bit-identical output.
- OS name strings matched to upstream: “FAT filesystem (MS-DOS, OS/2,
  NT)”, “MacOS”, etc.

### Microsoft Cabinet (MSCF)

- Full header parsing: cabinet size, file count, folder count, flags,
  set ID, cabinet number.
- CFFOLDER entries: datablock count, compression type.
- CFFILE entries (first 2 shown): MS-DOS date/time decoding (always
  “Sun” weekday matching upstream bug), file attributes (+R, +H, +S,
  +A, +X, +Utf), filenames with octal escaping of non-ASCII bytes.
- OneNote Package detection via first filename extension.

### MySQL MyISAM (.MYI)

- Fixed record and deleted-record field offsets: 8-byte big-endian
  quads at offsets 28 and 36 (was 4-byte at 20 and 24).

### Mach-O universal binary

- Fat binary parser: iterates fat_arch entries, reads per-architecture
  Mach-O headers (cputype, filetype, flags), formats multi-arch
  summary with `\012` literal newlines matching upstream.
- CPU type names: vax, mc680x0, i386, x86_64, arm, arm64, ppc, ppc64.
- Flag names: NOUNDEFS, DYLDLINK, TWOLEVEL, PIE, etc.

### ext2/ext3/ext4 filesystem

- Superblock magic detection at offset 0x438 (0xEF53).
- UUID formatting (8-4-4-4-12 hex).
- Filesystem type from feature flags: extents → ext4, has_journal →
  ext3, else ext2.
- Feature flag output: (needs journal recovery), (extents),
  (huge files).

### DOS/MBR boot sector

- Partition table parsing: 4 entries at offsets 446–509.
- CHS geometry, LBA start sector, sector count.
- Extended partition table detection (types 0x05, 0x0F, 0x85).
- Active partition flag.

### NTFS boot sector

- BPB fields: sectors/cluster, media descriptor, sectors/track, heads,
  hidden sectors.
- NTFS-specific fields: total sectors, $MFT start cluster, $MFTMirror
  start cluster, bytes/RecordSegment, clusters/index block, serial
  number.
- NTLDR bootstrap detection via string search.

### TIFF

- Separated Exif-specific IFD tags (description, manufacturer, model,
  orientation, resolution, software, datetime) from standalone TIFF
  output. Standalone TIFF emits only the original tags (height, bps,
  compression, PhotometricInterpretation, width). Exif context (from
  JPEG APP1) emits all tags.

### OLE/CDF Compound Document

- Implemented proper sector chain following via `ole_read_chain`:
  reads the FAT from DIFAT entries, follows sector chains to
  reconstruct non-contiguous streams.
- `ole_structural_summary`: parses OLE header, builds FAT, locates
  directory entries, finds SummaryInformation stream, handles both
  regular and mini-streams.
- Fixes MSP (Windows Installer Patch) files where property data
  crosses sector boundaries.

### MS Windows shortcut (LNK)

- Header parsing: LinkFlags bitmask, FileAttributes bitmask.
- FILETIME timestamps (ctime, atime, mtime) converted via
  `format_filetime`.
- TrackerDataBlock: MachineID extraction (signature 0xA0000003).
- PropertyStoreDataBlock: EnableTargetMetadata detection.
- IDList parsing: root folder CLSID (mixed-endian GUID formatting),
  volume drive letter.
- LinkInfo: LocalBasePath extraction.
- ShowCommand: normal/showmaximized/showminnoactive.

### Linux S390 kernel

- 24-byte magic match at offset 8 (EBCDIC-space–padded boot header).
- Machine subtype via 8BADCCCC marker search at ~0x10000: Z10, Z9-109,
  Z990, Z900 (32-bit and 64-bit variants).

### Unix dump file

- Big-endian new-fs dump detection: magic 0xEA6C at offset 24.
- Header fields: record type, dump/previous dates, volume, level,
  label, filesystem, device, host, flags.

### InstallShield Script

- Magic 0xB8C90C00 at offset 0.
- Copyright string extraction (2-byte LE Pascal string at offset 13).
- Variable name scanning: locates SRCDIR and extracts subsequent
  variable index/name pairs.
