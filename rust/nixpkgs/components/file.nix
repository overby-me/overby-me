# file — file type identification
#
# The file command determines file types using magic bytes, ELF headers,
# and text heuristics. Used by configure scripts, libtool, and stdenv
# hooks to detect binary formats.
{
  pkgs,
  mkComponent,
  status,
  source,
  ...
}:
mkComponent {
  name = "file";
  original = pkgs.file;
  replacement = pkgs.rust-file;
  status = status.available;
  source = source.repo;
  phase = 2;
  description = "File type identification using magic bytes";
  notes = "Using rust-file from rust/file — ELF, script, archive, image, text detection";
}
